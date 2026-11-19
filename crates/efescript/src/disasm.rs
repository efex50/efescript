//! Disassembler: efevm module -> `.efa` text (contract: docs/efa-syntax.md section 12).
//!
//! The output uses raw forms only (no call sugar, no immediates in source positions), so the
//! assembler turns it back into the same module (the `lines` section aside). Anything `.efa`
//! cannot express is reported as a `; warning:` comment.

use efevm::{ExportKind, FuncType, Inst, Module, ModuleData, Op, Shape, ValType};
use std::collections::{BTreeSet, HashMap, HashSet};

const RESERVED: &[&str] = &[
    "import", "memory", "const", "global", "table", "fn", "export", "mut", "reg", "nregs", "null",
    "db", "dw", "dd", "dq", "resb", "i32", "i64", "f32", "f64", "inf", "nan",
];

/// Prints `module` as `.efa` source that assembles back to the same module.
pub fn disassemble(module: &Module) -> String {
    disassemble_data(module.data())
}

/// Same as [`disassemble`] for raw (possibly unverified) module data. Never panics; indices that
/// are out of range print as `f{N}` / `g{N}` / `r{N}`.
pub fn disassemble_data(d: &ModuleData) -> String {
    let nimports = d.imports.len();
    let mut imports_group: Vec<String> = Vec::new();

    // Imports.
    let mut fnames: Vec<String> = Vec::new();
    for imp in &d.imports {
        let (text, changed) = import_name(&imp.namespace, &imp.name);
        if changed {
            imports_group.push(format!(
                "; warning: import \"{}.{}\" is not a valid qualified name, printed as {}",
                imp.namespace, imp.name, text
            ));
        }
        let sig = d.types.get(imp.type_idx as usize);
        imports_group.push(format!("import {}{}", text, sig_text(sig)));
        fnames.push(text);
    }

    // Function names.
    let mut used: HashSet<String> = HashSet::new();
    for (i, _) in d.functions.iter().enumerate() {
        let abs = (nimports + i) as u32;
        let named = d.names.funcs.iter().find(|(f, _)| *f == abs).map(|(_, n)| n.as_str());
        let exported = d.exports.iter().find_map(|e| match e.kind {
            ExportKind::Func(f) if f == abs => Some(e.name.as_str()),
            _ => None,
        });
        let base = match named.or(exported) {
            Some(n) => sanitize(n),
            None => sanitize(&format!("f{abs}")),
        };
        fnames.push(unique(base, abs, &mut used));
    }
    // Exports of imports cannot be expressed.
    for e in &d.exports {
        if let ExportKind::Func(f) = e.kind {
            if (f as usize) < nimports {
                imports_group
                    .push(format!("; warning: export \"{}\" of an import cannot be expressed", e.name));
            }
        }
    }

    // Memory (and the type table check).
    let mut mem_group: Vec<String> = Vec::new();
    if expected_types(d).as_ref() != Some(&d.types) {
        mem_group.push(
            "; warning: the type table has unused or reordered types, it cannot be expressed".into(),
        );
    }
    mem_group.push(match d.memory.max_pages {
        Some(max) => format!("memory {} {}", d.memory.min_pages, max),
        None => format!("memory {}", d.memory.min_pages),
    });

    // Globals.
    let mut gnames: Vec<Option<String>> = vec![None; d.globals.len()];
    let mut gexported: Vec<bool> = vec![false; d.globals.len()];
    let mut gused: HashSet<String> = HashSet::new();
    let mut globals_group: Vec<String> = Vec::new();
    for (gi, slot) in gnames.iter_mut().enumerate() {
        let first_ok = d.exports.iter().find(|e| {
            matches!(e.kind, ExportKind::Global(g) if g as usize == gi)
                && valid_ident(&e.name)
                && !gused.contains(&e.name)
        });
        if let Some(e) = first_ok {
            gused.insert(e.name.clone());
            *slot = Some(e.name.clone());
            if let Some(x) = gexported.get_mut(gi) {
                *x = true;
            }
        }
    }
    let gnames: Vec<String> = gnames
        .into_iter()
        .enumerate()
        .map(|(gi, n)| match n {
            Some(n) => n,
            None => unique(sanitize(&format!("g{gi}")), gi as u32, &mut gused),
        })
        .collect();
    for (gi, g) in d.globals.iter().enumerate() {
        let name = gnames.get(gi).cloned().unwrap_or_else(|| format!("g{gi}"));
        for e in &d.exports {
            if matches!(e.kind, ExportKind::Global(x) if x as usize == gi) && e.name != name {
                globals_group.push(format!(
                    "; warning: export \"{}\" of global {} cannot be expressed",
                    e.name, name
                ));
            }
        }
        let init = match g.ty {
            ValType::I32 => format!("{}", g.init as u32 as i32),
            ValType::I64 => format!("{}", g.init as i64),
            ValType::F32 => f32_text(g.init as u32),
            ValType::F64 => f64_text(g.init),
        };
        globals_group.push(format!(
            "{}global {}{}: {} = {}",
            if gexported.get(gi).copied().unwrap_or(false) { "export " } else { "" },
            if g.mutable { "mut " } else { "" },
            name,
            type_text(g.ty),
            init
        ));
    }

    // Table.
    let mut table_group: Vec<String> = Vec::new();
    if !d.table.is_empty() {
        let entries: Vec<String> = d
            .table
            .iter()
            .map(|e| match e {
                Some(i) => func_name(&fnames, *i),
                None => "null".to_string(),
            })
            .collect();
        table_group.push(format!("table [{}]", entries.join(", ")));
    }

    // Data.
    let data_group = data_lines(d);

    // Functions.
    let mut func_groups: Vec<Vec<String>> = Vec::new();
    for (i, f) in d.functions.iter().enumerate() {
        let abs = (nimports + i) as u32;
        func_groups.push(function_lines(d, abs, f, &fnames, &gnames));
    }

    // Section 12 is printed as a comment: `.efa` has no syntax for it, the assembler writes it
    // when given `--api`. A disassembly without the comment is therefore not byte-identical
    // for modules that have the section (reassemble with the same index to get it back).
    let api_group: Vec<String> = match &d.api {
        Some(api) => {
            let ns: Vec<String> = api.namespaces.iter().map(|(n, v)| format!("{n}@{v}")).collect();
            vec![format!("; api fnv1a64:{:016x} {}", api.hash, ns.join(" ")).trim_end().to_string()]
        }
        None => Vec::new(),
    };
    let mut groups: Vec<Vec<String>> =
        vec![api_group, imports_group, mem_group, globals_group, table_group, data_group];
    groups.extend(func_groups);
    let mut out = String::new();
    let mut first = true;
    for g in groups.iter().filter(|g| !g.is_empty()) {
        if !first {
            out.push('\n');
        }
        first = false;
        for l in g {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Names

fn is_reg_like(s: &str) -> bool {
    s.strip_prefix('r').is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
}

fn ident_chars(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    it.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A name the assembler accepts as a declaration (no dots, not reserved, not `r<digits>`).
fn valid_ident(s: &str) -> bool {
    ident_chars(s) && !RESERVED.contains(&s) && !is_reg_like(s)
}

fn sanitize(s: &str) -> String {
    let mut o: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if o.is_empty() {
        return "_".into();
    }
    if o.starts_with(|c: char| c.is_ascii_digit()) || RESERVED.contains(&o.as_str()) || is_reg_like(&o)
    {
        o.insert(0, '_');
    }
    o
}

/// Makes `base` unique within `used` (appending `_{n}`, then more underscores if needed).
fn unique(base: String, n: u32, used: &mut HashSet<String>) -> String {
    let mut cand = base;
    if used.contains(&cand) {
        cand = format!("{cand}_{n}");
        while used.contains(&cand) {
            cand.push('_');
        }
    }
    used.insert(cand.clone());
    cand
}

/// The printed `ns.name` of an import and whether it had to be changed.
fn import_name(ns: &str, name: &str) -> (String, bool) {
    let full = format!("{ns}.{name}");
    let ok = !ns.is_empty() && !name.is_empty() && full.split('.').all(ident_chars) && !name.contains('.');
    if ok {
        return (full, false);
    }
    let seg = |s: &str| -> String {
        let mut o: String =
            s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        if o.is_empty() || o.starts_with(|c: char| c.is_ascii_digit()) {
            o.insert(0, '_');
        }
        o
    };
    let nsp: Vec<String> = ns.split('.').map(seg).collect();
    let nsp = nsp.join(".");
    (format!("{}.{}", nsp, seg(name)), true)
}

fn func_name(fnames: &[String], i: u32) -> String {
    fnames.get(i as usize).cloned().unwrap_or_else(|| format!("f{i}"))
}

fn type_text(t: ValType) -> &'static str {
    match t {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
    }
}

fn results_text(r: &[ValType]) -> String {
    match r {
        [t] => type_text(*t).to_string(),
        _ => format!("({})", r.iter().map(|t| type_text(*t)).collect::<Vec<_>>().join(", ")),
    }
}

/// `(a, b) -> r` form used by `call_indirect` (always with an arrow).
fn full_sig_text(t: Option<&FuncType>) -> String {
    let empty = FuncType::default();
    let t = t.unwrap_or(&empty);
    format!(
        "({}) -> {}",
        t.params.iter().map(|t| type_text(*t)).collect::<Vec<_>>().join(", "),
        results_text(&t.results)
    )
}

/// `(a, b) -> r` for imports; the arrow is omitted when there are no results.
fn sig_text(t: Option<&FuncType>) -> String {
    let empty = FuncType::default();
    let t = t.unwrap_or(&empty);
    let params = t.params.iter().map(|t| type_text(*t)).collect::<Vec<_>>().join(", ");
    if t.results.is_empty() {
        format!("({params})")
    } else {
        format!("({params}) -> {}", results_text(&t.results))
    }
}

/// The type table the assembler would build for this module (imports, functions, then
/// `call_indirect` signatures in body order). `None` if a type index is out of range.
fn expected_types(d: &ModuleData) -> Option<Vec<FuncType>> {
    let mut out: Vec<FuncType> = Vec::new();
    let mut seen: HashSet<FuncType> = HashSet::new();
    let mut add = |t: &FuncType| {
        if seen.insert(t.clone()) {
            out.push(t.clone());
        }
    };
    for i in &d.imports {
        add(d.types.get(i.type_idx as usize)?);
    }
    for f in &d.functions {
        add(d.types.get(f.type_idx as usize)?);
    }
    for f in &d.functions {
        for inst in &f.code {
            if inst.op == Op::CallIndirect {
                add(d.types.get(inst.a as usize)?);
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Floats

fn f32_text(bits: u32) -> String {
    let v = f32::from_bits(bits);
    if v.is_nan() {
        if bits == 0x7fc0_0000 { "nan".into() } else { format!("nan:0x{bits:08x}") }
    } else {
        format!("{v:?}")
    }
}

fn f64_text(bits: u64) -> String {
    let v = f64::from_bits(bits);
    if v.is_nan() {
        if bits == 0x7ff8_0000_0000_0000 { "nan".into() } else { format!("nan:0x{bits:016x}") }
    } else {
        format!("{v:?}")
    }
}

// ---------------------------------------------------------------------------------------------
// Data

fn align8(x: u64) -> u64 {
    x.saturating_add(7) & !7
}

fn data_items(bytes: &[u8]) -> Vec<String> {
    let printable = |b: u8| (0x20..=0x7e).contains(&b);
    let mut items = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if printable(bytes[i]) {
            let mut j = i;
            while j < bytes.len() && printable(bytes[j]) {
                j += 1;
            }
            if j - i >= 3 {
                let mut s = String::from("\"");
                for &b in &bytes[i..j] {
                    match b {
                        b'"' => s.push_str("\\\""),
                        b'\\' => s.push_str("\\\\"),
                        _ => s.push(b as char),
                    }
                }
                s.push('"');
                items.push(s);
                i = j;
                continue;
            }
        }
        items.push(format!("0x{:02x}", bytes[i]));
        i += 1;
    }
    items
}

fn data_lines(d: &ModuleData) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor: u64 = 1024;
    let mut pad = 0u32;
    for (n, seg) in d.data.iter().enumerate() {
        if seg.bytes.is_empty() {
            out.push(format!("; warning: empty data segment {n} at offset {} cannot be expressed", seg.offset));
            continue;
        }
        let s = align8(cursor);
        if seg.offset < 1024 {
            out.push(format!(
                "; warning: data segment at offset {} cannot be expressed (below 1024)",
                seg.offset
            ));
            continue;
        }
        if seg.offset == s {
            // fits as is
        } else if seg.offset > s && seg.offset % 8 == 0 {
            out.push(format!("pad{pad} resb {}", seg.offset - s));
            pad += 1;
        } else {
            out.push(format!("; warning: data segment at offset {} cannot be expressed", seg.offset));
            continue;
        }
        let items = data_items(&seg.bytes);
        let lines: Vec<String> = items.chunks(12).map(|c| c.join(", ")).collect();
        out.push(format!("data{n} db {}", lines.join(",\n    ")));
        cursor = seg.offset.saturating_add(seg.bytes.len() as u64);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Functions

struct RegNames {
    names: Vec<String>,
    /// Number of names on the `reg` line.
    reg_line: Vec<String>,
    nparams: usize,
    contiguous: bool,
}

impl RegNames {
    fn get(&self, r: u32) -> String {
        match self.names.get(r as usize) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => format!("r{r}"),
        }
    }
}

fn reg_names(d: &ModuleData, abs: u32, nparams: usize) -> RegNames {
    let mut map: HashMap<u32, &str> = HashMap::new();
    for (f, r, n) in &d.names.regs {
        if *f == abs {
            map.entry(*r).or_insert(n.as_str());
        }
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut names: Vec<String> = Vec::new();
    // Parameters.
    for k in 0..nparams {
        match map.get(&(k as u32)) {
            Some(n) => names.push(unique(sanitize(n), k as u32, &mut used)),
            None => names.push(String::new()),
        }
    }
    // The contiguous run after the parameters.
    let mut reg_line = Vec::new();
    let mut k = nparams;
    while let Some(n) = map.get(&(k as u32)) {
        let s = unique(sanitize(n), k as u32, &mut used);
        names.push(s.clone());
        reg_line.push(s);
        k += 1;
    }
    let contiguous = map.keys().all(|r| (*r as usize) < k);
    RegNames { names, reg_line, nparams, contiguous }
}

fn required_nregs(d: &ModuleData, f: &efevm::Function, base: u64) -> u64 {
    let mut n = base;
    let mut bump = |v: u64| {
        if v > n {
            n = v;
        }
    };
    let results = d.types.get(f.type_idx as usize).map(|t| t.results.len()).unwrap_or(0) as u64;
    for i in &f.code {
        let (a, b, c) = (u64::from(i.a), u64::from(i.b), u64::from(i.c));
        match i.op.shape() {
            Shape::None | Shape::Jmp => {}
            Shape::D | Shape::ConstI32 | Shape::ConstI64 | Shape::ConstF32 | Shape::ConstF64 => bump(a + 1),
            Shape::DA | Shape::Load | Shape::Store => {
                bump(a + 1);
                bump(b + 1);
            }
            Shape::DAB | Shape::ABC => {
                bump(a + 1);
                bump(b + 1);
                bump(c + 1);
            }
            Shape::Select => {
                let (ra, rb) = i.select_regs();
                bump(a + 1);
                bump(b + 1);
                bump(u64::from(ra) + 1);
                bump(u64::from(rb) + 1);
            }
            Shape::GlobalGet | Shape::Br => bump(a + 1),
            Shape::GlobalSet => bump(b + 1),
            Shape::BrTable => bump(a + 1),
            Shape::Call => {
                let t = d.func_type(i.a);
                let w = t.map(|t| t.params.len().max(t.results.len())).unwrap_or(0) as u64;
                bump(b + w);
            }
            Shape::CallIndirect => {
                let t = d.types.get(i.a as usize);
                let w = t.map(|t| t.params.len().max(t.results.len())).unwrap_or(0) as u64;
                bump(b + 1);
                bump(c + w);
            }
            Shape::Ret => bump(a + results),
        }
    }
    n
}

fn function_lines(
    d: &ModuleData,
    abs: u32,
    f: &efevm::Function,
    fnames: &[String],
    gnames: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let name = func_name(fnames, abs);
    let ty = d.types.get(f.type_idx as usize);
    let empty = FuncType::default();
    let ty = ty.unwrap_or(&empty);
    let rn = reg_names(d, abs, ty.params.len());

    // Exports.
    let mut exported = false;
    for e in &d.exports {
        if matches!(e.kind, ExportKind::Func(x) if x == abs) {
            if e.name == name && !exported {
                exported = true;
            } else {
                out.push(format!(
                    "; warning: export \"{}\" of function {} cannot be expressed",
                    e.name, name
                ));
            }
        }
    }

    let params: Vec<String> = ty
        .params
        .iter()
        .enumerate()
        .map(|(k, t)| format!("{}: {}", rn.get(k as u32), type_text(*t)))
        .collect();
    let mut header = format!("{}fn {}({})", if exported { "export " } else { "" }, name, params.join(", "));
    if !ty.results.is_empty() {
        header.push_str(" -> ");
        header.push_str(&results_text(&ty.results));
    }
    header.push_str(" {");
    out.push(header);

    if !rn.reg_line.is_empty() {
        out.push(format!("    reg {}", rn.reg_line.join(", ")));
    }
    if !rn.contiguous {
        out.push("    ; warning: register names not contiguous".into());
    }
    let required = required_nregs(d, f, (rn.nparams + rn.reg_line.len()) as u64);
    if required != u64::from(f.nregs) {
        out.push(format!("    nregs {}", f.nregs));
    }

    // Labels.
    let mut targets: BTreeSet<u32> = BTreeSet::new();
    for i in &f.code {
        match i.op.shape() {
            Shape::Jmp => {
                targets.insert(i.a);
            }
            Shape::Br => {
                targets.insert(i.b);
            }
            Shape::BrTable => {
                if let Some(t) = f.br_tables.get(i.b as usize) {
                    targets.extend(t.targets.iter().copied());
                    targets.insert(t.default);
                }
            }
            _ => {}
        }
    }
    let labels: Vec<u32> = targets.into_iter().collect();
    let label = |t: u32| match labels.binary_search(&t) {
        Ok(k) => format!("L{k}"),
        Err(_) => format!("L{t}"),
    };

    for (idx, inst) in f.code.iter().enumerate() {
        if let Ok(k) = labels.binary_search(&(idx as u32)) {
            out.push(format!("L{k}:"));
        }
        out.push(format!("    {}", inst_text(d, f, inst, &rn, fnames, gnames, &label)));
    }
    out.push("}".into());
    out
}

fn inst_text(
    d: &ModuleData,
    f: &efevm::Function,
    i: &Inst,
    rn: &RegNames,
    fnames: &[String],
    gnames: &[String],
    label: &dyn Fn(u32) -> String,
) -> String {
    let m = i.op.name();
    let r = |x: u32| rn.get(x);
    let g = |x: u32| gnames.get(x as usize).cloned().unwrap_or_else(|| format!("g{x}"));
    match i.op.shape() {
        Shape::None => m.to_string(),
        Shape::D => format!("{m} {}", r(i.a)),
        Shape::DA => format!("{m} {}, {}", r(i.a), r(i.b)),
        Shape::DAB | Shape::ABC => format!("{m} {}, {}, {}", r(i.a), r(i.b), r(i.c)),
        Shape::ConstI32 => format!("{m} {}, {}", r(i.a), i.b as i32),
        Shape::ConstI64 => format!("{m} {}, {}", r(i.a), i.const_i64_value() as i64),
        Shape::ConstF32 => format!("{m} {}, {}", r(i.a), f32_text(i.b)),
        Shape::ConstF64 => format!("{m} {}, {}", r(i.a), f64_text(i.const_i64_value())),
        Shape::Select => {
            let (ra, rb) = i.select_regs();
            format!("{m} {}, {}, {}, {}", r(i.a), r(i.b), r(ra), r(rb))
        }
        Shape::Load | Shape::Store => format!("{m} {}, {}, {}", r(i.a), r(i.b), i.c),
        Shape::GlobalGet => format!("{m} {}, {}", r(i.a), g(i.b)),
        Shape::GlobalSet => format!("{m} {}, {}", g(i.a), r(i.b)),
        Shape::Jmp => format!("{m} {}", label(i.a)),
        Shape::Br => format!("{m} {}, {}", r(i.a), label(i.b)),
        Shape::BrTable => match f.br_tables.get(i.b as usize) {
            Some(t) => format!(
                "{m} {}, [{}], {}",
                r(i.a),
                t.targets.iter().map(|x| label(*x)).collect::<Vec<_>>().join(", "),
                label(t.default)
            ),
            None => format!("{m} {}, [], L0", r(i.a)),
        },
        Shape::Call => format!("{m} {}, {}", func_name(fnames, i.a), r(i.b)),
        Shape::CallIndirect => {
            format!("{m} {}, {}, {}", full_sig_text(d.types.get(i.a as usize)), r(i.b), r(i.c))
        }
        Shape::Ret => format!("{m} {}", r(i.a)),
    }
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use efevm::{FunctionBuilder, ModuleBuilder};

    /// Encoded bytes without the lines section (and without names when `names` is false).
    fn strip(m: &Module, names: bool) -> Vec<u8> {
        let mut data = m.data().clone();
        data.lines = Default::default();
        if !names {
            data.names = Default::default();
        }
        Module::new(data).expect("verified").encode()
    }

    fn roundtrip(m: &Module, names: bool) -> String {
        let text = disassemble(m);
        let back = match crate::asm::assemble(&text, "dis.efa") {
            Ok(b) => b,
            Err(e) => panic!("reassembly failed: {e}\n--- text ---\n{text}"),
        };
        assert_eq!(strip(m, names), strip(&back, names), "bytes differ\n--- text ---\n{text}");
        text
    }

    fn ty(p: &[ValType], r: &[ValType]) -> FuncType {
        FuncType::new(p.iter().copied(), r.iter().copied())
    }

    fn all_ops_module(named: bool) -> Module {
        use ValType::*;
        let mut mb = ModuleBuilder::new();
        let imp = mb.import("std", "println", ty(&[I64, I64], &[]));
        let callee_ty = ty(&[I64, I64], &[I64]);
        let callee = mb.declare_func(callee_ty.clone());
        let main = mb.declare_func(ty(&[I64], &[I64]));
        let g0 = mb.global_i64(false, -5);
        let g1 = mb.global_f64(true, 1.5);
        let g2 = mb.global_i32(true, -1);
        let g3 = mb.global_f32(false, 0.1);
        let _ = (g0, g2, g3);
        mb.table(vec![Some(callee), None, Some(imp)]);
        mb.memory(1, Some(4));
        mb.data(1024, b"hello world!");
        mb.data(1040, &[1, 2, 3]);
        mb.data(2048, &[0xff, b'A', b'B', b'C', 0, b'"', b'\\']);
        let t_callee = mb.add_type(callee_ty);
        let t_other = mb.add_type(ty(&[F32], &[]));
        let t_multi = mb.add_type(ty(&[], &[I32, I64]));

        let mut cb = FunctionBuilder::new(3);
        cb.op3(Op::I64Add, 2, 0, 1);
        cb.ret(2);
        mb.define(callee, cb);

        let mut fb = FunctionBuilder::new(16);
        let top = fb.new_label();
        let mid = fb.new_label();
        let end = fb.new_label();
        fb.bind(top);
        fb.nop();
        fb.bind(mid);
        for &op in Op::ALL {
            match op.shape() {
                Shape::None => fb.emit(Inst::new(op, 0, 0, 0)),
                Shape::D => fb.op1(op, 1),
                Shape::DA => fb.op2(op, 1, 2),
                Shape::DAB | Shape::ABC => fb.emit(Inst::new(op, 1, 2, 3)),
                Shape::ConstI32 => fb.const_i32(1, -7),
                Shape::ConstI64 => fb.const_i64(1, i64::MIN + 12345),
                Shape::ConstF32 => fb.const_f32(1, 0.1),
                Shape::ConstF64 => fb.const_f64(1, -0.0),
                Shape::Select => fb.select(1, 2, 3, 4),
                Shape::Load => fb.load(op, 1, 2, 16),
                Shape::Store => fb.store(op, 1, 2, 0xffff_fff0),
                Shape::GlobalGet => fb.global_get(1, g1),
                Shape::GlobalSet => fb.global_set(g1, 2),
                Shape::Jmp => fb.jmp(top),
                Shape::Br => {
                    if op == Op::BrIf {
                        fb.br_if(2, mid)
                    } else {
                        fb.br_ifz(2, end)
                    }
                }
                Shape::BrTable => {
                    fb.br_table(2, &[top, mid, mid], end);
                    fb.br_table(3, &[], top);
                }
                Shape::Call => {
                    fb.call(callee, 4);
                    fb.call(imp, 5);
                }
                Shape::CallIndirect => {
                    fb.call_indirect(t_callee, 3, 4);
                    fb.call_indirect(t_other, 3, 10);
                    fb.call_indirect(t_multi, 0, 8);
                }
                Shape::Ret => fb.ret(5),
            }
        }
        fb.bind(end);
        fb.ret(0);
        mb.define(main, fb);

        mb.export_func("main", main);
        mb.export_global("ticks", g1);
        if named {
            mb.func_name(imp, "std.println");
            mb.func_name(callee, "callee");
            mb.func_name(main, "main");
            mb.reg_name(callee, 0, "a");
            mb.reg_name(callee, 1, "b");
            mb.reg_name(callee, 2, "sum");
            mb.reg_name(main, 0, "n");
            mb.reg_name(main, 1, "x");
            mb.reg_name(main, 2, "y");
        }
        mb.finish().expect("module verifies")
    }

    #[test]
    fn every_opcode_prints() {
        let m = all_ops_module(true);
        let text = disassemble(&m);
        for &op in Op::ALL {
            assert!(
                text.lines().any(|l| l.trim_start().split(' ').next() == Some(op.name())),
                "missing {}",
                op.name()
            );
        }
        assert!(text.contains("pad0 resb "), "{text}");
        assert!(text.contains("br_table y, [L"), "{text}");
        assert!(text.contains("call_indirect () -> (i32, i64), n, r8"), "{text}");
    }

    #[test]
    fn many_unnamed_functions_roundtrip() {
        let mut mb = ModuleBuilder::new();
        mb.memory(1, None);
        for _ in 0..70 {
            let f = mb.declare_func(ty(&[], &[]));
            let mut fb = FunctionBuilder::new(0);
            fb.ret(0);
            mb.define(f, fb);
        }
        let m = mb.finish().expect("verifies");
        let text = disassemble(&m);
        assert!(text.contains("fn _f32()") && text.contains("fn _f64()"), "{text}");
        roundtrip(&m, false);
    }

    #[test]
    fn output_parses() {
        for m in [all_ops_module(true), all_ops_module(false), hello_module()] {
            let text = disassemble(&m);
            if let Err(e) = crate::asm::parse(&text, "dis.efa") {
                panic!("does not parse: {e}\n{text}");
            }
        }
    }

    #[test]
    fn every_opcode_roundtrips() {
        roundtrip(&all_ops_module(true), true);
        roundtrip(&all_ops_module(false), false);
    }

    #[test]
    fn no_names_section() {
        let m = all_ops_module(false);
        let text = disassemble(&m);
        assert!(text.contains("fn f1(r0: i64, r1: i64) -> i64 {"), "{text}");
        assert!(text.contains("export fn main(r0: i64) -> i64 {") || text.contains("export fn f2("), "{text}");
        assert!(text.contains("i64.add r2, r0, r1"), "{text}");
        assert!(text.contains("import std.println(i64, i64)"), "{text}");
    }

    fn hello_module() -> Module {
        use ValType::*;
        let mut mb = ModuleBuilder::new();
        let p = mb.import("std", "println", ty(&[I64, I64], &[]));
        let main = mb.declare_func(ty(&[], &[]));
        mb.memory(1, None);
        mb.data(1024, b"hello world!");
        let mut fb = FunctionBuilder::new(2);
        fb.const_i64(0, 1024);
        fb.const_i64(1, 12);
        fb.call(p, 0);
        fb.ret(0);
        mb.define(main, fb);
        mb.export_func("main", main);
        mb.func_name(p, "std.println");
        mb.func_name(main, "main");
        mb.finish().expect("verifies")
    }

    #[test]
    fn hello_is_readable() {
        let text = disassemble(&hello_module());
        let expected = "import std.println(i64, i64)\n\nmemory 1\n\ndata0 db \"hello world!\"\n\n\
                        export fn main() {\n    const.i64 r0, 1024\n    const.i64 r1, 12\n    \
                        call std.println, r0\n    ret r0\n}\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn hello_roundtrips() {
        roundtrip(&hello_module(), true);
    }

    #[test]
    fn nregs_printed_only_when_different() {
        use ValType::*;
        let mut mb = ModuleBuilder::new();
        let f = mb.declare_func(ty(&[I64], &[I64]));
        let mut fb = FunctionBuilder::new(9);
        fb.op2(Op::Mov, 1, 0);
        fb.ret(1);
        mb.define(f, fb);
        let m = mb.finish().expect("verifies");
        let text = disassemble(&m);
        assert!(text.contains("    nregs 9\n"), "{text}");
        let m2 = all_ops_module(true);
        // computed value matches: no nregs lines in the callee
        let t2 = disassemble(&m2);
        assert!(!t2.contains("fn callee(a: i64, b: i64) -> i64 {\n    reg sum\n    nregs"), "{t2}");
    }

    #[test]
    fn float_edge_cases() {
        use ValType::*;
        let mut mb = ModuleBuilder::new();
        let f = mb.declare_func(ty(&[], &[]));
        mb.memory(0, None);
        let g_a = mb.global(F64, false, 0x7ff8_0000_0000_0001);
        let g_b = mb.global(F32, true, u64::from(0xffc0_0000u32));
        let _ = (g_a, g_b);
        mb.global_f64(false, -0.0);
        mb.global_f32(false, 0.1);
        mb.global_f64(false, f64::INFINITY);
        mb.global_f32(false, f32::NEG_INFINITY);
        let mut fb = FunctionBuilder::new(2);
        for bits in [
            0x7ff8_0000_0000_0000u64,
            0x7ff8_0000_0000_0001,
            0xfff8_0000_0000_0000,
            0x7ff0_0000_0000_0001,
            (-0.0f64).to_bits(),
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            1e300f64.to_bits(),
            5e-324f64.to_bits(),
            0.1f64.to_bits(),
            f64::MAX.to_bits(),
        ] {
            fb.emit(Inst::new(Op::ConstF64, 0, bits as u32, (bits >> 32) as u32));
        }
        for bits in [
            0x7fc0_0000u32,
            0x7fc0_0001,
            0xffc0_0000,
            0x7f80_0001,
            (-0.0f32).to_bits(),
            f32::INFINITY.to_bits(),
            f32::NEG_INFINITY.to_bits(),
            0.1f32.to_bits(),
            1e-45f32.to_bits(),
            f32::MAX.to_bits(),
            1e20f32.to_bits(),
        ] {
            fb.emit(Inst::new(Op::ConstF32, 1, bits, 0));
        }
        fb.ret(0);
        mb.define(f, fb);
        let m = mb.finish().expect("verifies");
        let text = disassemble(&m);
        assert!(text.contains("const.f64 r0, nan\n"), "{text}");
        assert!(text.contains("const.f64 r0, nan:0x7ff8000000000001\n"), "{text}");
        assert!(text.contains("const.f64 r0, nan:0xfff8000000000000\n"), "{text}");
        assert!(text.contains("const.f64 r0, -0.0\n"), "{text}");
        assert!(text.contains("const.f32 r1, 0.1\n"), "{text}");
        assert!(text.contains("const.f32 r1, nan:0x7fc00001\n"), "{text}");
        assert!(text.contains("const.f32 r1, -inf\n"), "{text}");
        assert!(text.contains("memory 0\n"), "{text}");
        roundtrip(&m, false);
    }

    #[test]
    fn section_15_example_roundtrips() {
        let src = include_str!("../docs/efa-syntax.md");
        let start = match src.find("## 15.") {
            Some(s) => s,
            None => panic!("section 15 missing"),
        };
        let rest = &src[start..];
        let a = rest.find("```\n").map(|i| i + 4).unwrap_or(0);
        let b = rest[a..].find("```").map(|i| i + a).unwrap_or(rest.len());
        let program = &rest[a..b];
        let m = match crate::asm::assemble(program, "tour.efa") {
            Ok(m) => m,
            Err(e) => panic!("tour does not assemble: {e}"),
        };
        let text = roundtrip(&m, true);
        // second generation is a fixed point
        let again = crate::asm::assemble(&text, "dis.efa").expect("assembles");
        assert_eq!(text, disassemble(&again));
    }

    #[test]
    fn hello_and_sum_from_text_roundtrip() {
        let hello = "import std.println(i64, i64)\nmsg db \"hello world!\"\nexport fn main() {\n    \
                     call std.println(msg, msg.len)\n    ret\n}\n";
        let sum = "export fn sum(n: i64) -> i64 {\n    reg acc, t\n    const.i64 acc, 0\nloop:\n    \
                   i64.eqz t, n\n    br_if t, done\n    i64.add acc, acc, n\n    i64.sub n, n, 1\n    \
                   jmp loop\ndone:\n    ret acc\n}\n";
        for (name, src) in [("hello.efa", hello), ("sum.efa", sum)] {
            let m = match crate::asm::assemble(src, name) {
                Ok(m) => m,
                Err(e) => panic!("{name}: {e}"),
            };
            roundtrip(&m, true);
        }
    }

    #[test]
    fn sanitising_and_warnings() {
        use ValType::*;
        let mut mb = ModuleBuilder::new();
        let a = mb.declare_func(ty(&[I64], &[]));
        let b = mb.declare_func(ty(&[], &[]));
        let c = mb.declare_func(ty(&[], &[]));
        mb.memory(1, None);
        for f in [a, b, c] {
            let mut fb = FunctionBuilder::new(4);
            fb.ret(0);
            mb.define(f, fb);
        }
        mb.func_name(a, "1bad.name");
        mb.func_name(b, "r5");
        mb.func_name(c, "r5");
        mb.reg_name(a, 0, "import");
        mb.reg_name(a, 2, "far");
        mb.export_func("other", b);
        mb.data(1030, &[1]);
        let m = mb.finish().expect("verifies");
        let text = disassemble(&m);
        assert!(text.contains("fn _1bad_name(_import: i64)"), "{text}");
        assert!(text.contains("fn _r5() {"), "{text}");
        assert!(text.contains("fn _r5_2() {"), "{text}");
        assert!(text.contains("; warning: export \"other\" of function _r5 cannot be expressed"), "{text}");
        assert!(text.contains("; warning: register names not contiguous"), "{text}");
        assert!(text.contains("; warning: data segment at offset 1030"), "{text}");
    }
}
