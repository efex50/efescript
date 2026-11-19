//! Load-time verification (SPEC.md section 8).

use crate::error::LoadError;
use crate::api::interface::crosses_modules;
use crate::api::{ApiType, Param, StructDesc, is_ident, is_namespace};
use crate::module::{ExportKind, Inst, Interface, ModuleData, Provide};
use crate::op::Shape;
use crate::types::{PAGE_SIZE, ValType};
use std::collections::HashSet;
use std::fmt;

/// One variant per verification rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    TypeIndex,
    FunctionIndex,
    GlobalIndex,
    TableEntry,
    DuplicateExport,
    Register,
    Label,
    BrTableIndex,
    CallWindow,
    RetWindow,
    NregsLessThanParams,
    ImmutableGlobalSet,
    /// 32-bit global with upper init bits set.
    GlobalInit,
    /// min_pages > max_pages.
    MemoryLimits,
    /// Offset + length beyond min_pages * 64 KiB, or overflow.
    DataOutOfRange,
    /// Function does not end in a terminator (including empty code).
    MissingTerminator,
    NonZeroUnusedOperand,
    NameIndex,
    LineIndex,
    /// A namespace in the `api` section (12) that is not a valid namespace name or appears twice.
    ApiNamespace,
    /// A rule of section 13 `interface` (SPEC.md 9.2). `what` names the provide, struct,
    /// scratch region or the section (`interface`) for count limits.
    Interface { what: String, reason: String },
    /// A rule of section 14 `events` (SPEC.md 9.3). `what` names the event reference
    /// (`event 3 `game.score``) or the section (`events`) for the count limit.
    Events { what: String, reason: String },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            VerifyError::Interface { what, reason } => return write!(f, "interface: {what}: {reason}"),
            VerifyError::Events { what, reason } => return write!(f, "events: {what}: {reason}"),
            VerifyError::TypeIndex => "type index out of range",
            VerifyError::FunctionIndex => "function index out of range",
            VerifyError::GlobalIndex => "global index out of range",
            VerifyError::TableEntry => "table entry out of range",
            VerifyError::DuplicateExport => "duplicate export name",
            VerifyError::Register => "register out of range",
            VerifyError::Label => "label out of range",
            VerifyError::BrTableIndex => "br_table index out of range",
            VerifyError::CallWindow => "call window does not fit in registers",
            VerifyError::RetWindow => "return window does not fit in registers",
            VerifyError::NregsLessThanParams => "nregs is less than the parameter count",
            VerifyError::ImmutableGlobalSet => "global.set on an immutable global",
            VerifyError::GlobalInit => "global initializer has bits outside its 32-bit type",
            VerifyError::MemoryLimits => "memory minimum exceeds maximum",
            VerifyError::DataOutOfRange => "data segment outside initial memory",
            VerifyError::MissingTerminator => "function does not end in a terminator",
            VerifyError::NonZeroUnusedOperand => "unused operand field is not zero",
            VerifyError::NameIndex => "name entry index out of range",
            VerifyError::LineIndex => "line entry index out of range",
            VerifyError::ApiNamespace => "invalid or duplicate namespace in the api section",
        };
        f.write_str(s)
    }
}

impl std::error::Error for VerifyError {}

fn fail<T>(func: Option<u32>, inst: Option<u32>, error: VerifyError) -> Result<T, LoadError> {
    Err(LoadError::Verify { func, inst, error })
}

/// Verifies a whole module; returns the first error found.
pub(crate) fn verify(m: &ModuleData) -> Result<(), LoadError> {
    let ntypes = m.types.len();
    let nimports = m.imports.len();
    let nfuncs = m.num_funcs() as u64;
    let nglobals = m.globals.len();

    if let Some(api) = &m.api {
        for (i, (ns, _)) in api.namespaces.iter().enumerate() {
            if !crate::api::is_namespace(ns) || api.namespaces[..i].iter().any(|(o, _)| o == ns) {
                return fail(None, None, VerifyError::ApiNamespace);
            }
        }
    }
    for (i, imp) in m.imports.iter().enumerate() {
        if imp.type_idx as usize >= ntypes {
            return fail(Some(i as u32), None, VerifyError::TypeIndex);
        }
    }
    for (i, f) in m.functions.iter().enumerate() {
        if f.type_idx as usize >= ntypes {
            return fail(Some((nimports + i) as u32), None, VerifyError::TypeIndex);
        }
    }
    for e in m.table.iter().flatten() {
        if u64::from(*e) >= nfuncs {
            return fail(None, None, VerifyError::TableEntry);
        }
    }
    if m.memory.max_pages.is_some_and(|max| max < m.memory.min_pages) {
        return fail(None, None, VerifyError::MemoryLimits);
    }
    for g in &m.globals {
        if matches!(g.ty, ValType::I32 | ValType::F32) && (g.init >> 32) != 0 {
            return fail(None, None, VerifyError::GlobalInit);
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for e in &m.exports {
        match e.kind {
            ExportKind::Func(f) if u64::from(f) >= nfuncs => {
                return fail(None, None, VerifyError::FunctionIndex);
            }
            ExportKind::Global(g) if g as usize >= nglobals => {
                return fail(None, None, VerifyError::GlobalIndex);
            }
            _ => {}
        }
        if !seen.insert(e.name.as_str()) {
            return fail(None, None, VerifyError::DuplicateExport);
        }
    }
    let mem_bytes = u128::from(m.memory.min_pages) * u128::from(PAGE_SIZE);
    for d in &m.data {
        if u128::from(d.offset) + d.bytes.len() as u128 > mem_bytes {
            return fail(None, None, VerifyError::DataOutOfRange);
        }
    }

    for (i, f) in m.functions.iter().enumerate() {
        verify_function(m, (nimports + i) as u32, f)?;
    }

    for (f, _) in &m.names.funcs {
        if u64::from(*f) >= nfuncs {
            return fail(None, None, VerifyError::NameIndex);
        }
    }
    for (f, r, _) in &m.names.regs {
        let ok = (*f as usize)
            .checked_sub(nimports)
            .and_then(|i| m.functions.get(i))
            .is_some_and(|func| *r < u32::from(func.nregs));
        if !ok {
            return fail(None, None, VerifyError::NameIndex);
        }
    }
    for fl in &m.lines.funcs {
        let func = (fl.func as usize).checked_sub(nimports).and_then(|i| m.functions.get(i));
        let Some(func) = func else {
            return fail(Some(fl.func), None, VerifyError::LineIndex);
        };
        for e in &fl.entries {
            if e.inst as usize >= func.code.len() || e.file as usize >= m.lines.files.len() {
                return fail(Some(fl.func), Some(e.inst), VerifyError::LineIndex);
            }
        }
    }
    if let Some(iface) = &m.interface {
        verify_interface(m, iface).map_err(|error| LoadError::Verify { func: None, inst: None, error })?;
    }
    verify_events(m).map_err(|error| LoadError::Verify { func: None, inst: None, error })?;
    Ok(())
}

/// Limit of SPEC.md 9.3 rule 1.
const MAX_EVENT_REFS: usize = 1024;

/// SPEC.md 9.3: at most 1024 references; namespaces are namespace names, event and payload
/// struct names identifiers; `(namespace, name)` unique.
fn verify_events(m: &ModuleData) -> Result<(), VerifyError> {
    let err = |what: String, reason: String| Err(VerifyError::Events { what, reason });
    if m.events.len() > MAX_EVENT_REFS {
        return err("events".into(), format!("{} event references, at most {MAX_EVENT_REFS}", m.events.len()));
    }
    let mut seen: HashSet<(&str, &str)> = HashSet::with_capacity(m.events.len());
    for (i, e) in m.events.iter().enumerate() {
        let what = || format!("event {i} `{}`", e.qualified());
        if !is_namespace(&e.namespace) {
            return err(what(), format!("namespace `{}` is not a namespace name", e.namespace));
        }
        if !is_ident(&e.name) {
            return err(what(), "name is not an identifier".into());
        }
        if let Some((s, _)) = &e.payload
            && !is_ident(s)
        {
            return err(what(), format!("payload struct name `{s}` is not an identifier"));
        }
        if !seen.insert((e.namespace.as_str(), e.name.as_str())) {
            return err(what(), "duplicate event reference".into());
        }
    }
    Ok(())
}

/// Limits of SPEC.md 9.2 rule 1 and rule 8.
const MAX_PROVIDES: usize = 1024;
const MAX_PROVIDED_EVENTS: usize = 1024;
const MAX_STRUCTS: usize = 256;
const MAX_PARAMS: usize = 64;
const MAX_RESULTS: usize = 16;
const MAX_FIELDS: usize = 256;
const MAX_SCRATCH: u32 = 1_048_576;

/// SPEC.md 9.2 rules 1-9. Counts are checked first, so the quadratic uniqueness checks
/// are bounded.
fn verify_interface(m: &ModuleData, iface: &Interface) -> Result<(), VerifyError> {
    let err = |what: String, reason: String| Err(VerifyError::Interface { what, reason });
    let pname = |p: &Provide| format!("provide `{}`", p.name);
    let sname = |s: &StructDesc| format!("struct `{}`", s.name);

    // Rule 1: counts.
    if iface.provides.len() > MAX_PROVIDES {
        return err("interface".into(), format!("{} provides, at most {MAX_PROVIDES}", iface.provides.len()));
    }
    if iface.structs.len() > MAX_STRUCTS {
        return err("interface".into(), format!("{} structs, at most {MAX_STRUCTS}", iface.structs.len()));
    }
    if iface.events.len() > MAX_PROVIDED_EVENTS {
        return err("interface".into(), format!("{} events, at most {MAX_PROVIDED_EVENTS}", iface.events.len()));
    }
    for s in &iface.structs {
        if s.fields.len() > MAX_FIELDS {
            return err(sname(s), format!("{} fields, at most {MAX_FIELDS}", s.fields.len()));
        }
    }
    for p in &iface.provides {
        if p.params.len() > MAX_PARAMS {
            return err(pname(p), format!("{} parameters, at most {MAX_PARAMS}", p.params.len()));
        }
        if p.results.len() > MAX_RESULTS {
            return err(pname(p), format!("{} results, at most {MAX_RESULTS}", p.results.len()));
        }
    }

    // Rule 7: structs.
    for (i, s) in iface.structs.iter().enumerate() {
        if let Err(reason) = s.validate() {
            return err(sname(s), reason);
        }
        if iface.structs[..i].iter().any(|o| o.name == s.name) {
            return err(sname(s), "duplicate struct name".into());
        }
        if !s.owner.is_empty() && !is_namespace(&s.owner) {
            return err(sname(s), format!("owner `{}` is not a namespace name", s.owner));
        }
        if s.owner.is_empty() && s.component.is_some() {
            return err(sname(s), "an own struct (empty owner) cannot have a component id".into());
        }
        if let Some(id) = s.component
            && iface.structs[..i].iter().any(|o| o.component == Some(id))
        {
            return err(sname(s), format!("duplicate component id {id}"));
        }
    }

    let nimports = m.imports.len() as u64;
    for (i, p) in iface.provides.iter().enumerate() {
        // Rule 2: names and doc.
        if !is_ident(&p.name) {
            return err(pname(p), "name is not an identifier".into());
        }
        if iface.provides[..i].iter().any(|o| o.name == p.name) {
            return err(pname(p), "duplicate provide name".into());
        }
        let all: Vec<&Param> = p.params.iter().chain(&p.results).collect();
        for (j, a) in all.iter().enumerate() {
            if !is_ident(&a.name) {
                return err(pname(p), format!("parameter or result name `{}` is not an identifier", a.name));
            }
            if all[..j].iter().any(|o| o.name == a.name) {
                return err(pname(p), format!("duplicate parameter or result name `{}`", a.name));
            }
        }
        if p.doc.trim().is_empty() {
            return err(pname(p), "doc is blank".into());
        }
        // Rule 3: a defined function.
        let f = u64::from(p.func);
        if f < nimports {
            return err(pname(p), format!("function {f} is an import, not a function of this module"));
        }
        if f >= u64::from(m.num_funcs()) {
            return err(pname(p), format!("function index {f} out of range"));
        }
        // Rule 5 (and `ptr`, which has no encoding): types that can cross modules.
        for a in &all {
            if !crosses_modules(&a.ty) {
                return err(pname(p), format!("`{}` is a ptr, which cannot cross modules", a.name));
            }
        }
        if let Some(r) = p.results.iter().find(|r| !r.ty.is_scalar()) {
            return err(pname(p), format!("result `{}` is {}, results must be scalar", r.name, r.ty));
        }
        // Rule 6: struct parameters name a listed struct.
        for a in &p.params {
            if let ApiType::Struct(s) = &a.ty
                && iface.struct_desc(s).is_none()
            {
                return err(pname(p), format!("parameter `{}`: struct `{s}` is not listed in the interface", a.name));
            }
        }
        // Rule 4: the lowered signature is the function's type.
        let want = p.func_type();
        match m.func_type(p.func) {
            Some(ty) if *ty == want => {}
            Some(ty) => {
                return err(pname(p), format!("signature lowers to {want} but function {f} has type {ty}"));
            }
            None => return err(pname(p), format!("function {f} has no valid type")),
        }
    }

    // Rule 9: provided events.
    for (i, e) in iface.events.iter().enumerate() {
        let what = || format!("event `{}`", e.name);
        if !is_ident(&e.name) {
            return err(what(), "name is not an identifier".into());
        }
        if iface.events[..i].iter().any(|o| o.name == e.name) {
            return err(what(), "duplicate event name".into());
        }
        if iface.provide(&e.name).is_some() {
            return err(what(), "a provide has the same name".into());
        }
        if e.doc.trim().is_empty() {
            return err(what(), "doc is blank".into());
        }
        if let Some(s) = &e.payload
            && iface.struct_desc(s).is_none()
        {
            return err(what(), format!("payload struct `{s}` is not listed in the interface"));
        }
    }

    // Rule 8: scratch.
    match iface.scratch {
        None | Some((_, 0)) if iface.needs_scratch() => {
            let p = iface.provides.iter().find(|p| p.needs_scratch()).map(pname).unwrap_or_default();
            return err("scratch".into(), format!("{p} has a str, bytes or struct parameter, so a scratch region of at least 1 byte is required"));
        }
        None => {}
        Some((offset, size)) => {
            if offset % 8 != 0 {
                return err("scratch".into(), format!("offset {offset} is not a multiple of 8"));
            }
            if size > MAX_SCRATCH {
                return err("scratch".into(), format!("size {size} is larger than {MAX_SCRATCH}"));
            }
            let (start, end) = (u128::from(offset), u128::from(offset) + u128::from(size));
            let mem = u128::from(m.memory.min_pages) * u128::from(PAGE_SIZE);
            if end > mem {
                return err("scratch".into(), format!("[{start}, {end}) is outside the initial memory of {mem} bytes"));
            }
            for (i, d) in m.data.iter().enumerate() {
                let (ds, de) = (u128::from(d.offset), u128::from(d.offset) + d.bytes.len() as u128);
                // Empty ranges (a size-0 scratch, an empty segment) overlap nothing.
                if start < end && ds < de && start < de && ds < end {
                    return err("scratch".into(), format!("[{start}, {end}) overlaps data segment {i} at [{ds}, {de})"));
                }
            }
        }
    }
    Ok(())
}

fn verify_function(m: &ModuleData, findex: u32, f: &crate::module::Function) -> Result<(), LoadError> {
    let Some(ty) = m.types.get(f.type_idx as usize) else {
        return fail(Some(findex), None, VerifyError::TypeIndex);
    };
    let nregs = u32::from(f.nregs);
    if (ty.params.len() as u64) > u64::from(nregs) {
        return fail(Some(findex), None, VerifyError::NregsLessThanParams);
    }
    let ncode = f.code.len();
    for t in &f.br_tables {
        if t.targets.iter().chain(std::iter::once(&t.default)).any(|l| *l as usize >= ncode) {
            return fail(Some(findex), None, VerifyError::Label);
        }
    }
    for (i, inst) in f.code.iter().enumerate() {
        if let Err(e) = verify_inst(m, f, ty.results.len() as u64, inst) {
            return fail(Some(findex), Some(i as u32), e);
        }
    }
    match f.code.last() {
        Some(last) if last.op.is_terminator() => Ok(()),
        _ => fail(Some(findex), None, VerifyError::MissingTerminator),
    }
}

fn verify_inst(
    m: &ModuleData,
    f: &crate::module::Function,
    nresults: u64,
    i: &Inst,
) -> Result<(), VerifyError> {
    let nregs = u32::from(f.nregs);
    let zero = |fields: &[u32]| {
        if fields.iter().all(|x| *x == 0) { Ok(()) } else { Err(VerifyError::NonZeroUnusedOperand) }
    };
    let reg = |r: u32| if r < nregs { Ok(()) } else { Err(VerifyError::Register) };
    let label = |l: u32| if (l as usize) < f.code.len() { Ok(()) } else { Err(VerifyError::Label) };
    let window = |base: u32, n: usize| {
        if u64::from(base) + n as u64 <= u64::from(nregs) { Ok(()) } else { Err(VerifyError::CallWindow) }
    };
    match i.op.shape() {
        Shape::None => zero(&[i.a, i.b, i.c]),
        Shape::D => {
            zero(&[i.b, i.c])?;
            reg(i.a)
        }
        Shape::DA => {
            zero(&[i.c])?;
            reg(i.a)?;
            reg(i.b)
        }
        Shape::DAB | Shape::ABC => {
            reg(i.a)?;
            reg(i.b)?;
            reg(i.c)
        }
        Shape::ConstI32 | Shape::ConstF32 => {
            zero(&[i.c])?;
            reg(i.a)
        }
        Shape::ConstI64 | Shape::ConstF64 => reg(i.a),
        Shape::Select => {
            reg(i.a)?;
            reg(i.b)?;
            let (ra, rb) = i.select_regs();
            reg(ra)?;
            reg(rb)
        }
        Shape::Load | Shape::Store => {
            reg(i.a)?;
            reg(i.b)
        }
        Shape::GlobalGet => {
            zero(&[i.c])?;
            reg(i.a)?;
            if (i.b as usize) < m.globals.len() { Ok(()) } else { Err(VerifyError::GlobalIndex) }
        }
        Shape::GlobalSet => {
            zero(&[i.c])?;
            let g = m.globals.get(i.a as usize).ok_or(VerifyError::GlobalIndex)?;
            reg(i.b)?;
            if g.mutable { Ok(()) } else { Err(VerifyError::ImmutableGlobalSet) }
        }
        Shape::Jmp => {
            zero(&[i.b, i.c])?;
            label(i.a)
        }
        Shape::Br => {
            zero(&[i.c])?;
            reg(i.a)?;
            label(i.b)
        }
        Shape::BrTable => {
            zero(&[i.c])?;
            reg(i.a)?;
            if (i.b as usize) < f.br_tables.len() { Ok(()) } else { Err(VerifyError::BrTableIndex) }
        }
        Shape::Call => {
            zero(&[i.c])?;
            let ty = m.func_type(i.a).ok_or(
                if u64::from(i.a) < u64::from(m.num_funcs()) {
                    VerifyError::TypeIndex
                } else {
                    VerifyError::FunctionIndex
                },
            )?;
            window(i.b, ty.params.len().max(ty.results.len()))
        }
        Shape::CallIndirect => {
            let ty = m.types.get(i.a as usize).ok_or(VerifyError::TypeIndex)?;
            reg(i.b)?;
            window(i.c, ty.params.len().max(ty.results.len()))
        }
        Shape::Ret => {
            zero(&[i.b, i.c])?;
            if u64::from(i.a) + nresults <= u64::from(nregs) {
                Ok(())
            } else {
                Err(VerifyError::RetWindow)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Op;
    use crate::builder::{FunctionBuilder, ModuleBuilder};
    use crate::module::{
        BrTable, DataSegment, Export, FuncLines, Function, Global, Import, LineEntry, Module,
    };
    use crate::types::FuncType;

    fn ty(p: usize, r: usize) -> FuncType {
        FuncType::new(vec![ValType::I32; p], vec![ValType::I32; r])
    }

    /// One function (index 0): type (i32)->(i32), nregs 4, `ret 0`.
    fn base() -> ModuleData {
        ModuleData {
            types: vec![ty(1, 1)],
            functions: vec![Function {
                type_idx: 0,
                nregs: 4,
                code: vec![Inst::new(Op::Ret, 0, 0, 0)],
                br_tables: vec![],
            }],
            ..Default::default()
        }
    }

    fn with_code(code: Vec<Inst>) -> ModuleData {
        let mut m = base();
        m.functions[0].code = code;
        m
    }

    fn ret() -> Inst {
        Inst::new(Op::Ret, 0, 0, 0)
    }

    fn err(m: ModuleData) -> (Option<u32>, Option<u32>, VerifyError) {
        match Module::new(m) {
            Err(LoadError::Verify { func, inst, error }) => (func, inst, error),
            other => panic!("expected verify error, got {other:?}"),
        }
    }

    fn kind(m: ModuleData) -> VerifyError {
        err(m).2
    }

    fn inst_err(code: Vec<Inst>) -> (Option<u32>, Option<u32>, VerifyError) {
        err(with_code(code))
    }

    #[test]
    fn accepts_base() {
        assert!(Module::new(base()).is_ok());
    }

    #[test]
    fn accepts_calls_brtable_globals_data() {
        let mut b = ModuleBuilder::new();
        let t = b.add_type(ty(1, 1));
        let imp = b.import("env", "f", ty(1, 1));
        b.memory(1, Some(2));
        let g = b.global(ValType::I32, true, 5);
        let gc = b.global(ValType::I64, false, u64::MAX);
        b.data(65530, &[1, 2, 3, 4, 5, 6]);
        let mut fb = FunctionBuilder::new(6);
        let l0 = fb.new_label();
        let l1 = fb.new_label();
        let ld = fb.new_label();
        fb.global_get(1, g);
        fb.global_set(g, 1);
        fb.call(imp, 2);
        fb.call_indirect(t, 0, 3);
        fb.br_table(0, &[l0, l1], ld);
        fb.bind(l0);
        fb.select(1, 0, 2, 3);
        fb.br_if(0, l1);
        fb.bind(l1);
        fb.jmp(ld);
        fb.bind(ld);
        fb.ret(0);
        let f = b.add_func(ty(1, 1), fb);
        b.table(vec![Some(f), None, Some(imp)]);
        b.export_func("main", f);
        b.export_func("imp", imp);
        b.export_global("g", g);
        b.export_global("gc", gc);
        b.func_name(f, "main");
        b.reg_name(f, 5, "x");
        let file = b.file("a.ef");
        let _ = file;
        assert!(b.finish().is_ok());
    }

    #[test]
    fn accepts_max_window() {
        // call with base + 1 == nregs, ret base 3 with 1 result
        let mut m = base();
        m.functions[0].code = vec![Inst::new(Op::Call, 0, 3, 0), Inst::new(Op::Ret, 3, 0, 0)];
        assert!(Module::new(m).is_ok());
    }

    #[test]
    fn type_index() {
        let mut m = base();
        m.functions[0].type_idx = 5;
        assert_eq!(err(m), (Some(0), None, VerifyError::TypeIndex));
        let mut m = base();
        m.imports.push(Import { namespace: "a".into(), name: "b".into(), type_idx: 9 });
        assert_eq!(err(m), (Some(0), None, VerifyError::TypeIndex));
        assert_eq!(inst_err(vec![Inst::new(Op::CallIndirect, 7, 0, 0), ret()]), (Some(0), Some(0), VerifyError::TypeIndex));
    }

    #[test]
    fn function_index() {
        assert_eq!(inst_err(vec![Inst::new(Op::Call, 1, 0, 0), ret()]), (Some(0), Some(0), VerifyError::FunctionIndex));
        let mut m = base();
        m.exports.push(Export { name: "x".into(), kind: ExportKind::Func(1) });
        assert_eq!(kind(m), VerifyError::FunctionIndex);
    }

    #[test]
    fn global_index() {
        assert_eq!(inst_err(vec![Inst::new(Op::GlobalGet, 0, 0, 0), ret()]).2, VerifyError::GlobalIndex);
        assert_eq!(inst_err(vec![Inst::new(Op::GlobalSet, 3, 0, 0), ret()]).2, VerifyError::GlobalIndex);
        let mut m = base();
        m.exports.push(Export { name: "x".into(), kind: ExportKind::Global(0) });
        assert_eq!(kind(m), VerifyError::GlobalIndex);
    }

    #[test]
    fn table_entry() {
        let mut m = base();
        m.table = vec![Some(0), None, Some(1)];
        assert_eq!(kind(m), VerifyError::TableEntry);
    }

    #[test]
    fn duplicate_export() {
        let mut m = base();
        m.globals.push(Global { ty: ValType::I32, mutable: false, init: 0 });
        m.exports.push(Export { name: "x".into(), kind: ExportKind::Func(0) });
        m.exports.push(Export { name: "x".into(), kind: ExportKind::Global(0) });
        assert_eq!(kind(m), VerifyError::DuplicateExport);
    }

    #[test]
    fn register() {
        assert_eq!(inst_err(vec![Inst::new(Op::I32Add, 0, 1, 4), ret()]), (Some(0), Some(0), VerifyError::Register));
        assert_eq!(inst_err(vec![Inst::select(0, 1, 2, 4), ret()]).2, VerifyError::Register);
        assert_eq!(inst_err(vec![Inst::const_i32(70000, 1), ret()]).2, VerifyError::Register);
        assert_eq!(inst_err(vec![Inst::new(Op::BrIf, 9, 0, 0), ret()]).2, VerifyError::Register);
    }

    #[test]
    fn label() {
        assert_eq!(inst_err(vec![Inst::new(Op::Jmp, 2, 0, 0), ret()]), (Some(0), Some(0), VerifyError::Label));
        assert_eq!(inst_err(vec![Inst::new(Op::BrIf, 0, 2, 0), ret()]).2, VerifyError::Label);
        let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 0, 0), ret()]);
        m.functions[0].br_tables.push(BrTable { targets: vec![0, 1], default: 7 });
        assert_eq!(kind(m), VerifyError::Label);
        let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 0, 0), ret()]);
        m.functions[0].br_tables.push(BrTable { targets: vec![0, 9], default: 0 });
        assert_eq!(kind(m), VerifyError::Label);
    }

    #[test]
    fn br_table_index() {
        let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 1, 0), ret()]);
        m.functions[0].br_tables.push(BrTable { targets: vec![], default: 0 });
        assert_eq!(err(m), (Some(0), Some(0), VerifyError::BrTableIndex));
    }

    #[test]
    fn call_window() {
        // callee type (i32)->(i32), base 4 + 1 > 4
        assert_eq!(inst_err(vec![Inst::new(Op::Call, 0, 4, 0), ret()]), (Some(0), Some(0), VerifyError::CallWindow));
        assert_eq!(inst_err(vec![Inst::new(Op::Call, 0, u32::MAX, 0), ret()]).2, VerifyError::CallWindow);
        let mut m = with_code(vec![Inst::new(Op::CallIndirect, 0, 0, 4), ret()]);
        assert_eq!(kind(m.clone()), VerifyError::CallWindow);
        m.types.push(ty(0, 0));
        m.functions[0].code[0] = Inst::new(Op::CallIndirect, 1, 0, 4);
        assert!(Module::new(m).is_ok());
    }

    #[test]
    fn ret_window() {
        assert_eq!(inst_err(vec![Inst::new(Op::Ret, 4, 0, 0)]), (Some(0), Some(0), VerifyError::RetWindow));
    }

    #[test]
    fn nregs_less_than_params() {
        let mut m = base();
        m.types[0] = ty(5, 0);
        assert_eq!(err(m), (Some(0), None, VerifyError::NregsLessThanParams));
    }

    #[test]
    fn immutable_global_set() {
        let mut m = with_code(vec![Inst::new(Op::GlobalSet, 0, 0, 0), ret()]);
        m.globals.push(Global { ty: ValType::I32, mutable: false, init: 0 });
        assert_eq!(err(m.clone()), (Some(0), Some(0), VerifyError::ImmutableGlobalSet));
        m.globals[0].mutable = true;
        assert!(Module::new(m).is_ok());
    }

    #[test]
    fn global_init() {
        for ty in [ValType::I32, ValType::F32] {
            let mut m = base();
            m.globals.push(Global { ty, mutable: false, init: 1 << 32 });
            assert_eq!(kind(m), VerifyError::GlobalInit);
        }
        let mut m = base();
        m.globals.push(Global { ty: ValType::I64, mutable: false, init: 1 << 32 });
        assert!(Module::new(m).is_ok());
    }

    #[test]
    fn memory_limits() {
        let mut m = base();
        m.memory.min_pages = 3;
        m.memory.max_pages = Some(2);
        assert_eq!(kind(m), VerifyError::MemoryLimits);
    }

    #[test]
    fn data_out_of_range() {
        let mut m = base();
        m.memory.min_pages = 1;
        m.data.push(DataSegment { offset: 65535, bytes: vec![1, 2] });
        assert_eq!(kind(m.clone()), VerifyError::DataOutOfRange);
        m.data[0] = DataSegment { offset: u64::MAX, bytes: vec![1] };
        assert_eq!(kind(m.clone()), VerifyError::DataOutOfRange);
        m.data[0] = DataSegment { offset: 65534, bytes: vec![1, 2] };
        assert!(Module::new(m).is_ok());
    }

    #[test]
    fn missing_terminator() {
        assert_eq!(inst_err(vec![]), (Some(0), None, VerifyError::MissingTerminator));
        assert_eq!(inst_err(vec![ret(), Inst::new(Op::Nop, 0, 0, 0)]).2, VerifyError::MissingTerminator);
        assert!(Module::new(with_code(vec![Inst::new(Op::Unreachable, 0, 0, 0)])).is_ok());
    }

    #[test]
    fn non_zero_unused_operand() {
        let bad = |i: Inst| assert_eq!(inst_err(vec![i, ret()]), (Some(0), Some(0), VerifyError::NonZeroUnusedOperand));
        bad(Inst::new(Op::Nop, 1, 0, 0));
        bad(Inst::new(Op::MemorySize, 0, 1, 0));
        bad(Inst::new(Op::Mov, 0, 0, 1));
        bad(Inst::new(Op::ConstI32, 0, 0, 1));
        bad(Inst::new(Op::Jmp, 0, 1, 0));
        bad(Inst::new(Op::BrIf, 0, 0, 1));
        bad(Inst::new(Op::Call, 0, 0, 1));
        assert_eq!(inst_err(vec![Inst::new(Op::Ret, 0, 1, 0)]).2, VerifyError::NonZeroUnusedOperand);
        let mut m = with_code(vec![Inst::new(Op::GlobalGet, 0, 0, 1), ret()]);
        m.globals.push(Global { ty: ValType::I32, mutable: true, init: 0 });
        assert_eq!(kind(m), VerifyError::NonZeroUnusedOperand);
    }

    #[test]
    fn name_index() {
        let mut m = base();
        m.names.funcs.push((3, "x".into()));
        assert_eq!(kind(m), VerifyError::NameIndex);
        let mut m = base();
        m.names.regs.push((0, 4, "x".into()));
        assert_eq!(kind(m), VerifyError::NameIndex);
        let mut m = base();
        m.imports.push(Import { namespace: "a".into(), name: "b".into(), type_idx: 0 });
        m.names.regs.push((0, 0, "x".into()));
        assert_eq!(kind(m), VerifyError::NameIndex);
    }

    #[test]
    fn line_index() {
        let entry = |inst, file| LineEntry { inst, file, line: 1, column: 1 };
        let mk = |func, e| {
            let mut m = base();
            m.lines.files.push("a".into());
            m.lines.funcs.push(FuncLines { func, entries: vec![e] });
            m
        };
        assert_eq!(kind(mk(5, entry(0, 0))), VerifyError::LineIndex);
        assert_eq!(kind(mk(0, entry(1, 0))), VerifyError::LineIndex);
        assert_eq!(kind(mk(0, entry(0, 1))), VerifyError::LineIndex);
        assert!(Module::new(mk(0, entry(0, 0))).is_ok());
    }

    #[test]
    fn never_panics_on_garbage() {
        let mut m = base();
        m.functions[0].nregs = u16::MAX;
        m.functions[0].code = vec![
            Inst::new(Op::Call, u32::MAX, u32::MAX, u32::MAX),
            Inst::new(Op::Ret, u32::MAX, 0, 0),
        ];
        assert!(Module::new(m).is_err());
    }
}
