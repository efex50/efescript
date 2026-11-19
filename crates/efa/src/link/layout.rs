//! Phase 1 of the linker: indices, data layout, scratch region, memory and the
//! program-wide checks; and the assembly of section 13 from the provides
//! (`docs/e8-design.md` section 4, `docs/efa-syntax.md` 16.5 to 16.8).

use std::collections::HashMap;

use efevm::api::{ApiType, Param, StructDesc};
use efevm::{FuncType, Provide};

use super::{
    interface_value, provide_type, Decls, Diagnostic, LookupError, ProvideType, Space, Span, Symbol, SymbolMap,
    Unit, UnitSyms,
};

/// First data address (efa-syntax section 8).
pub const DATA_START: u64 = 1024;
/// Bytes per memory page.
pub const PAGE: u64 = 65536;
/// Size of the scratch region when a provide needs one and the root sets none (16.8).
pub const DEFAULT_SCRATCH: u32 = 4096;
/// Largest scratch region (SPEC.md 9.2 rule 8).
pub const MAX_SCRATCH: u32 = 1_048_576;

pub(crate) fn idx(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `n` rounded up to `align` (a power of two; anything else counts as 1), saturating.
pub fn align_up(n: u64, align: u64) -> u64 {
    if align <= 1 || !align.is_power_of_two() {
        return n;
    }
    n.saturating_add(align - 1) & !(align - 1)
}

/// The note pointing at an earlier declaration: "first defined at line N" in the same unit,
/// "first defined in FILE:N" in another one; none for a declaration without a location.
pub fn first_note(units: &[&dyn Unit], here: usize, first_unit: usize, first: Span) -> Option<String> {
    if first.line == 0 {
        return None;
    }
    if here == first_unit {
        Some(format!("first defined at line {}", first.line))
    } else {
        let file = units.get(first_unit).map(|u| u.file()).unwrap_or("");
        Some(format!("first defined in {file}:{}", first.line))
    }
}

fn report(units: &[&dyn Unit], unit: usize, span: Span, msg: String, note: Option<String>, diags: &mut Vec<Diagnostic>) {
    let d = match units.get(unit) {
        Some(u) => u.diagnostic(span, msg),
        None => Diagnostic::without_location("", msg),
    };
    diags.push(match note {
        Some(n) => d.with_note(n),
        None => d,
    });
}

fn needs_scratch(t: &ProvideType) -> bool {
    match t {
        ProvideType::Api(a) => !a.is_scalar(),
        ProvideType::Struct(_) | ProvideType::IndexStruct(_) => true,
    }
}

/// Phase 1: assigns every index and address and runs the program-wide checks.
pub fn layout(units: &[&dyn Unit], decls: Vec<Decls>, diags: &mut Vec<Diagnostic>) -> SymbolMap {
    let mut map = SymbolMap::default();

    // Imports: merged by `ns.name`, numbered in order of first appearance.
    let mut first_import: HashMap<(String, String), (usize, usize, Span)> = HashMap::new();
    let mut unit_imports: Vec<Vec<u32>> = Vec::new();
    for (u, d) in decls.iter().enumerate() {
        let mut v = Vec::new();
        for imp in &d.imports {
            let key = (imp.namespace.clone(), imp.name.clone());
            match first_import.get(&key) {
                Some(&(i, fu, fspan)) => {
                    let ty0 = map.imports.get(i).map(|x| x.2.clone()).unwrap_or_else(|| FuncType::new([], []));
                    if fu != u && ty0 != imp.ty {
                        let file0 = units.get(fu).map(|x| x.file()).unwrap_or("");
                        report(
                            units,
                            u,
                            imp.span,
                            format!(
                                "import `{}.{}` is declared as {} here but as {} in `{file0}`",
                                imp.namespace, imp.name, imp.ty, ty0
                            ),
                            first_note(units, u, fu, fspan),
                            diags,
                        );
                    }
                    v.push(idx(i));
                }
                None => {
                    let i = map.imports.len();
                    first_import.insert(key, (i, u, imp.span));
                    map.imports.push((imp.namespace.clone(), imp.name.clone(), imp.ty.clone()));
                    v.push(idx(i));
                }
            }
        }
        unit_imports.push(v);
    }

    // Events (efa-syntax 17): merged by `ns.name`, numbered in order of first appearance.
    let unit_events = merge_events(units, &decls, &first_import, &mut map, diags);

    // Functions after the imports, globals, data: unit order, then source order.
    let mut next_func = map.imports.len();
    let mut next_global = 0usize;
    let mut cursor = DATA_START;
    let mut any_data = false;
    for (u, ((d, imports), events)) in decls.into_iter().zip(unit_imports).zip(unit_events).enumerate() {
        let funcs: Vec<u32> = (0..d.funcs.len()).map(|k| idx(next_func + k)).collect();
        next_func += d.funcs.len();
        let globals: Vec<u32> = (0..d.globals.len()).map(|k| idx(next_global + k)).collect();
        next_global += d.globals.len();
        let mut data = Vec::new();
        for x in &d.data {
            let addr = align_up(cursor, x.align);
            cursor = addr.saturating_add(x.size);
            any_data = true;
            data.push(addr);
        }
        let unit = units.get(u);
        map.units.push(UnitSyms {
            file: unit.map(|x| x.file().to_string()).unwrap_or_default(),
            namespace: unit.map(|x| x.namespace().to_string()).unwrap_or_default(),
            decls: d,
            imports,
            funcs,
            globals,
            data,
            events,
        });
    }
    let data_end = if any_data { cursor } else { 0 };

    // `memory` and `scratch` only in the root.
    for (u, us) in map.units.iter().enumerate().skip(1) {
        let root = units.first().map(|x| x.file()).unwrap_or("");
        if let Some(m) = &us.decls.memory {
            report(units, u, m.span, format!("`memory` may only appear in the root file `{root}`"), None, diags);
        }
        if let Some(s) = &us.decls.scratch {
            report(units, u, s.span, format!("`scratch` may only appear in the root file `{root}`"), None, diags);
        }
    }

    // Scratch region (16.8): after the last data item, 8-aligned.
    let needing: Option<(usize, String)> = map.units.iter().enumerate().find_map(|(u, us)| {
        us.decls
            .provides
            .iter()
            .find(|p| p.params.iter().any(|q| needs_scratch(&q.ty)))
            .map(|p| (u, p.name.clone()))
    });
    let root_scratch = map.units.first().and_then(|r| r.decls.scratch.clone());
    let scratch_size = match &root_scratch {
        Some(s) => {
            if s.size == 0 {
                if let Some((_, p)) = &needing {
                    report(
                        units,
                        0,
                        s.span,
                        format!("`scratch 0` leaves no room for the `str`, `bytes` and struct arguments of provide `{p}`"),
                        Some("remove the line for the default of 4096 bytes, or give a size".to_string()),
                        diags,
                    );
                }
            }
            s.size
        }
        None if needing.is_some() => DEFAULT_SCRATCH,
        None => 0,
    };
    let mut end = data_end;
    if scratch_size > 0 {
        let off = align_up(if any_data { cursor } else { DATA_START }, 8);
        map.scratch = Some((off, scratch_size));
        end = off.saturating_add(u64::from(scratch_size));
    }
    map.data_end = end;

    // Memory (efa-syntax section 8; the scratch region counts toward it).
    let scratch_note = map
        .scratch
        .map(|(_, n)| format!("the scratch region of {n} bytes follows the data"));
    map.memory = (1, None);
    match map.units.first().and_then(|r| r.decls.memory.clone()) {
        None => match u32::try_from(end.div_ceil(PAGE).max(1)) {
            Ok(p) => map.memory = (p, None),
            Err(_) => report(units, 0, Span::none(), "the data does not fit in any memory".to_string(), None, diags),
        },
        Some(m) => {
            if let Some(min) = m.min {
                if end > u64::from(min) * PAGE {
                    report(
                        units,
                        0,
                        m.span,
                        format!(
                            "the data needs {} bytes but `memory {}` provides only {}",
                            end,
                            min,
                            u64::from(min) * PAGE
                        ),
                        scratch_note.clone(),
                        diags,
                    );
                }
                map.memory = (min, m.max);
            }
        }
    }

    // At most one table in the program.
    let mut table: Option<(usize, Span)> = None;
    for (u, us) in map.units.iter().enumerate() {
        if let Some(span) = us.decls.table {
            match table {
                None => table = Some((u, span)),
                Some((fu, fspan)) => report(
                    units,
                    u,
                    span,
                    "duplicate `table` declaration".to_string(),
                    first_note(units, u, fu, fspan),
                    diags,
                ),
            }
        }
    }

    // Export names: unique across functions and globals of the whole program.
    let mut exports: HashMap<String, (usize, Span)> = HashMap::new();
    let check = |u: usize, name: &str, span: Span, what: &str, seen: &mut HashMap<String, (usize, Span)>, diags: &mut Vec<Diagnostic>| {
        if let Some(&(fu, fspan)) = seen.get(name) {
            report(units, u, span, format!("duplicate {what} `{name}`"), first_note(units, u, fu, fspan), diags);
        } else {
            seen.insert(name.to_string(), (u, span));
        }
    };
    for (u, us) in map.units.iter().enumerate() {
        for f in &us.decls.funcs {
            if let Some(e) = &f.export {
                check(u, e, f.span, "export name", &mut exports, diags);
            }
        }
    }
    for (u, us) in map.units.iter().enumerate() {
        for g in &us.decls.globals {
            if let Some(e) = &g.export {
                check(u, e, g.span, "export name", &mut exports, diags);
            }
        }
    }
    // Provide names and exported struct names: unique in the program.
    let mut provides: HashMap<String, (usize, Span)> = HashMap::new();
    let mut structs: HashMap<String, (usize, Span)> = HashMap::new();
    for (u, us) in map.units.iter().enumerate() {
        for p in &us.decls.provides {
            check(u, &p.name, p.span, "provide name", &mut provides, diags);
        }
        for s in us.decls.structs.iter().filter(|s| s.export) {
            check(u, &s.desc.name, s.span, "exported struct", &mut structs, diags);
        }
    }
    // Provided events: unique, and never equal to a provide name (SPEC.md 9.2 rule 9).
    let mut pevents: HashMap<String, (usize, Span)> = HashMap::new();
    for (u, us) in map.units.iter().enumerate() {
        for e in &us.decls.provided_events {
            check(u, &e.name, e.span, "provided event", &mut pevents, diags);
            if let Some(&(pu, pspan)) = provides.get(&e.name) {
                report(
                    units,
                    u,
                    e.span,
                    format!("provided event `{0}` has the same name as provide `{0}`: a function and an event never share a full name", e.name),
                    first_note(units, u, pu, pspan),
                    diags,
                );
            }
        }
    }
    check_unindexed_events(units, &map, diags);
    map
}

/// `&S (8 bytes)` or `no payload`.
fn payload_text(p: &Option<(String, u32)>) -> String {
    match p {
        Some((n, size)) => format!("&{n} ({size} bytes)"),
        None => "no payload".to_string(),
    }
}

/// The name and size of an event payload struct declared by a unit, found in the raw
/// declarations (the symbol map does not exist yet). Reports a private or unknown struct.
fn payload_struct(
    units: &[&dyn Unit],
    decls: &[Decls],
    from: usize,
    ty: &Option<ProvideType>,
    diags: &mut Vec<Diagnostic>,
) -> Result<Option<(String, u32)>, ()> {
    match ty {
        None => Ok(None),
        Some(ProvideType::Api(_)) => Err(()),
        Some(ProvideType::IndexStruct(d)) => Ok(Some((d.name.clone(), d.size))),
        Some(ProvideType::Struct(r)) => {
            let found = decls.get(r.unit).and_then(|d| d.structs.iter().find(|s| s.desc.name == r.name));
            match found {
                None => {
                    report(units, from, r.span, format!("unknown struct `{}`", r.name), None, diags);
                    Err(())
                }
                Some(s) if s.public || s.export || r.unit == from => Ok(Some((s.desc.name.clone(), s.desc.size))),
                Some(s) => {
                    if let Some(f) = units.get(from) {
                        let file = units.get(r.unit).map(|x| x.file()).unwrap_or("");
                        diags.push(super::private_error(*f, r.span, &r.name, file, s.span));
                    }
                    Err(())
                }
            }
        }
    }
}

/// Section 14: merges the `import event` items of all units. Returns the id of every event
/// of every unit.
fn merge_events(
    units: &[&dyn Unit],
    decls: &[Decls],
    first_import: &HashMap<(String, String), (usize, usize, Span)>,
    map: &mut SymbolMap,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Vec<u32>> {
    let mut first: HashMap<(String, String), (usize, usize, Span)> = HashMap::new();
    let mut out = Vec::new();
    for (u, d) in decls.iter().enumerate() {
        let mut ids = Vec::new();
        for ev in &d.event_imports {
            let Ok(payload) = payload_struct(units, decls, u, &ev.payload, diags) else {
                ids.push(u32::MAX);
                continue;
            };
            let key = (ev.namespace.clone(), ev.name.clone());
            if let Some(&(_, fu, _)) = first_import.get(&key) {
                let file0 = units.get(fu).map(|x| x.file()).unwrap_or("");
                report(
                    units,
                    u,
                    ev.span,
                    format!("`{}.{}` is an event here but a function import in `{file0}`", ev.namespace, ev.name),
                    Some("an event and a function never share a full name".to_string()),
                    diags,
                );
            }
            match first.get(&key) {
                Some(&(i, fu, fspan)) => {
                    let p0 = map.events.get(i).map(|e| e.2.clone()).unwrap_or(None);
                    if p0 != payload {
                        let file0 = units.get(fu).map(|x| x.file()).unwrap_or("");
                        report(
                            units,
                            u,
                            ev.span,
                            format!(
                                "event `{}.{}` is declared with {} here but with {} in `{file0}`",
                                ev.namespace,
                                ev.name,
                                payload_text(&payload),
                                payload_text(&p0)
                            ),
                            first_note(units, u, fu, fspan),
                            diags,
                        );
                    }
                    ids.push(idx(i));
                }
                None => {
                    let i = map.events.len();
                    first.insert(key, (i, u, ev.span));
                    map.events.push((ev.namespace.clone(), ev.name.clone(), payload));
                    ids.push(idx(i));
                }
            }
        }
        out.push(ids);
    }
    out
}

/// With an API index, an event that is not in it is only accepted as the program's own
/// provided event (17.3): same name, same payload struct name.
fn check_unindexed_events(units: &[&dyn Unit], map: &SymbolMap, diags: &mut Vec<Diagnostic>) {
    let payload_name = |t: &Option<ProvideType>| -> Option<String> {
        match t {
            Some(ProvideType::Struct(r)) => Some(r.name.clone()),
            Some(ProvideType::IndexStruct(d)) => Some(d.name.clone()),
            _ => None,
        }
    };
    for (u, us) in map.units.iter().enumerate() {
        for ev in us.decls.event_imports.iter().filter(|e| e.unindexed) {
            let own = map.units.iter().flat_map(|x| x.decls.provided_events.iter()).find(|p| p.name == ev.name);
            match own {
                None => {
                    let d = match units.get(u) {
                        Some(x) => x.diagnostic(ev.span, format!("unknown event `{}.{}`", ev.namespace, ev.name)),
                        None => Diagnostic::without_location("", format!("unknown event `{}.{}`", ev.namespace, ev.name)),
                    };
                    let note = ev.hint.clone().map(|s| format!("did you mean `{s}`?")).unwrap_or_else(|| {
                        format!(
                            "it is not in the API index; a mod's own event is accepted when the program has `provide event {}`",
                            ev.name
                        )
                    });
                    diags.push(d.with_note(note));
                }
                Some(p) => {
                    let (a, b) = (payload_name(&ev.payload), payload_name(&p.payload));
                    if a != b {
                        let show = |n: &Option<String>| n.as_ref().map_or("no payload".to_string(), |n| format!("`&{n}`"));
                        report(
                            units,
                            u,
                            ev.span,
                            format!(
                                "event `{}.{}` is imported with payload {} but the program provides `{}` with {}",
                                ev.namespace,
                                ev.name,
                                show(&a),
                                p.name,
                                show(&b)
                            ),
                            None,
                            diags,
                        );
                    }
                }
            }
        }
    }
}

/// Section 13 from the provides: provides in unit then source order with their function
/// index; the structs (every own `export struct` in unit then source order, then a copy of
/// each index struct a provide uses, with its owner, in first-use order); the scratch region.
/// `None` when the program has no provide, no exported struct and no scratch region, so a
/// program without the E8 items has no section 13.
pub fn interface(units: &[&dyn Unit], syms: &SymbolMap, diags: &mut Vec<Diagnostic>) -> Option<efevm::Interface> {
    let mut own: Vec<StructDesc> = Vec::new();
    for us in &syms.units {
        for s in us.decls.structs.iter().filter(|s| s.export) {
            own.push(s.desc.clone());
        }
    }
    let mut copies: Vec<StructDesc> = Vec::new();
    let mut provides: Vec<Provide> = Vec::new();
    for (u, us) in syms.units.iter().enumerate() {
        for p in &us.decls.provides {
            let mut ok = true;
            let param_ty = |q: &super::ProvideParam, result: bool, copies: &mut Vec<StructDesc>, diags: &mut Vec<Diagnostic>| -> Option<ApiType> {
                let t = match &q.ty {
                    ProvideType::Api(ApiType::Ptr) => {
                        report(
                            units,
                            u,
                            q.span,
                            format!("`ptr` cannot be used in provide `{}`: a raw address means nothing in another module's memory", p.name),
                            Some("pass `bytes`, `str` or `&Struct` instead".to_string()),
                            diags,
                        );
                        return None;
                    }
                    ProvideType::Api(t) => t.clone(),
                    ProvideType::Struct(r) => match syms.lookup(u, r.unit, Space::Struct, &r.name) {
                        Ok(Symbol::Struct(desc)) => {
                            let exported = syms
                                .unit(r.unit)
                                .and_then(|t| t.decls.structs.iter().find(|s| s.desc.name == r.name))
                                .is_some_and(|s| s.export);
                            if !exported {
                                report(
                                    units,
                                    u,
                                    r.span,
                                    format!("struct `{}` is used by provide `{}` and must be `export`", r.name, p.name),
                                    Some("write `export struct` so its layout goes into the module's interface".to_string()),
                                    diags,
                                );
                                return None;
                            }
                            ApiType::Struct(desc.name)
                        }
                        Ok(_) | Err(LookupError::Unknown) => {
                            report(units, u, r.span, format!("unknown struct `{}`", r.name), None, diags);
                            return None;
                        }
                        Err(e) => {
                            if let Some(from) = units.get(u) {
                                diags.push(e.diagnostic(*from, r.span, "struct", &r.name));
                            }
                            return None;
                        }
                    },
                    ProvideType::IndexStruct(desc) => {
                        if !copies.iter().any(|c| c.name == desc.name) {
                            copies.push(desc.clone());
                        }
                        ApiType::Struct(desc.name.clone())
                    }
                };
                if result && !t.is_scalar() {
                    report(
                        units,
                        u,
                        q.span,
                        format!("result `{}` of provide `{}` must be a scalar (`i32 i64 f32 f64 bool entity`), found `{t}`", q.name, p.name),
                        None,
                        diags,
                    );
                    return None;
                }
                Some(t)
            };
            let mut params = Vec::new();
            for q in &p.params {
                match param_ty(q, false, &mut copies, diags) {
                    Some(t) => params.push(Param::new(q.name.clone(), t)),
                    None => ok = false,
                }
            }
            let mut results = Vec::new();
            for q in &p.results {
                match param_ty(q, true, &mut copies, diags) {
                    Some(t) => results.push(Param::new(q.name.clone(), t)),
                    None => ok = false,
                }
            }
            let mut seen: HashMap<&str, Span> = HashMap::new();
            for q in p.params.iter().chain(&p.results) {
                if let Some(first) = seen.get(q.name.as_str()) {
                    report(
                        units,
                        u,
                        q.span,
                        format!("duplicate parameter `{}` in provide `{}`", q.name, p.name),
                        first_note(units, u, u, *first),
                        diags,
                    );
                    ok = false;
                } else {
                    seen.insert(&q.name, q.span);
                }
            }
            let target = &p.target;
            let shown = match syms.unit(target.unit) {
                Some(t) if target.unit != u && !t.namespace.is_empty() => format!("{}.{}", t.namespace, target.name),
                _ => target.name.clone(),
            };
            let func = match syms.lookup(u, target.unit, Space::Func, &target.name) {
                Ok(Symbol::Func { index, ty }) => Some((index, ty)),
                Ok(_) => None,
                Err(e) => {
                    if let Some(from) = units.get(u) {
                        diags.push(e.diagnostic(*from, target.span, "function", &shown));
                    }
                    ok = false;
                    None
                }
            };
            let Some((index, fty)) = func else { continue };
            if ok {
                let lowered = provide_type(&params, &results);
                if lowered != fty {
                    report(
                        units,
                        u,
                        p.span,
                        format!(
                            "provide `{}` lowers to `{lowered}`, but its function `{shown}` has `{fty}`",
                            p.name
                        ),
                        Some("`bool` is i32, `entity` and `&S` are i64, `str` and `bytes` are two i64 (pointer, length)".to_string()),
                        diags,
                    );
                }
            }
            provides.push(Provide { name: p.name.clone(), func: index, doc: p.doc.clone(), params, results });
        }
    }
    // Provided events (17.2): the payload struct must be an exported struct or an index struct.
    let mut events: Vec<efevm::ProvidedEvent> = Vec::new();
    for (u, us) in syms.units.iter().enumerate() {
        for e in &us.decls.provided_events {
            let payload = match &e.payload {
                None => None,
                Some(ProvideType::Api(_)) => continue,
                Some(ProvideType::IndexStruct(desc)) => {
                    if !copies.iter().any(|c| c.name == desc.name) {
                        copies.push(desc.clone());
                    }
                    Some(desc.name.clone())
                }
                Some(ProvideType::Struct(r)) => match syms.lookup(u, r.unit, Space::Struct, &r.name) {
                    Ok(Symbol::Struct(desc)) => {
                        let exported = syms
                            .unit(r.unit)
                            .and_then(|t| t.decls.structs.iter().find(|s| s.desc.name == r.name))
                            .is_some_and(|s| s.export);
                        if !exported {
                            report(
                                units,
                                u,
                                r.span,
                                format!("struct `{}` is the payload of provided event `{}` and must be `export`", r.name, e.name),
                                Some("write `export struct` so its layout goes into the module's interface".to_string()),
                                diags,
                            );
                            continue;
                        }
                        Some(desc.name)
                    }
                    Ok(_) | Err(LookupError::Unknown) => {
                        report(units, u, r.span, format!("unknown struct `{}`", r.name), None, diags);
                        continue;
                    }
                    Err(err) => {
                        if let Some(from) = units.get(u) {
                            diags.push(err.diagnostic(*from, r.span, "struct", &r.name));
                        }
                        continue;
                    }
                },
            };
            events.push(efevm::ProvidedEvent { name: e.name.clone(), payload, doc: e.doc.clone() });
        }
    }
    if provides.is_empty() && own.is_empty() && events.is_empty() && syms.scratch.is_none() {
        return None;
    }
    own.extend(copies);
    Some(interface_value(syms.scratch, own, provides, events))
}
