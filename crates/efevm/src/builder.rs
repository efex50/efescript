//! Builders for modules and function bodies.

use crate::error::{BuildError, LoadError};
use crate::module::{
    ApiRef, BrTable, DataSegment, EventRef, Export, ExportKind, FuncLines, Function, Global, Import, Interface, Inst,
    LineEntry, MemoryType, Module, ModuleData,
};
use crate::op::{Op, Shape};
use crate::types::{FuncType, ValType};
use std::collections::HashMap;

/// A jump target inside one function; create with `FunctionBuilder::new_label`.
/// A label must only be used with the `FunctionBuilder` that created it; out-of-range
/// label indices are reported as `BuildError::UnboundLabel`, but a label from a different
/// builder with an in-range index cannot be detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label(u32);

/// Builds one function body. Label operands are emitted as label ids and resolved to
/// instruction indices by `ModuleBuilder::finish`.
#[derive(Clone, Debug)]
pub struct FunctionBuilder {
    nregs: u16,
    code: Vec<Inst>,
    labels: Vec<Option<u32>>,
    br_tables: Vec<BrTable>,
    lines: Vec<LineEntry>,
    error: Option<BuildError>,
}

impl FunctionBuilder {
    pub fn new(nregs: u16) -> Self {
        FunctionBuilder {
            nregs,
            code: Vec::new(),
            labels: Vec::new(),
            br_tables: Vec::new(),
            lines: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, e: BuildError) {
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    /// Sets the register count.
    pub fn set_nregs(&mut self, nregs: u16) {
        self.nregs = nregs;
    }

    /// Current register count.
    pub fn nregs(&self) -> u16 {
        self.nregs
    }

    /// Raises the register count so that register `r` exists.
    pub fn ensure_reg(&mut self, r: u16) {
        match r.checked_add(1) {
            Some(n) => {
                if n > self.nregs {
                    self.nregs = n;
                }
            }
            None => self.fail(BuildError::TooManyRegisters),
        }
    }

    /// Index the next emitted instruction will get.
    pub fn len(&self) -> u32 {
        self.code.len() as u32
    }

    /// True if no instruction has been emitted.
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    fn check_label(&mut self, l: Label) {
        if l.0 as usize >= self.labels.len() {
            self.fail(BuildError::UnboundLabel);
        }
    }

    fn shape_check(&mut self, op: Op, ok: bool) -> bool {
        if !ok {
            self.fail(BuildError::WrongShape(op));
        }
        ok
    }

    pub fn new_label(&mut self) -> Label {
        self.labels.push(None);
        Label((self.labels.len() - 1) as u32)
    }

    /// Binds the label to the index of the next instruction.
    pub fn bind(&mut self, l: Label) {
        let here = self.code.len() as u32;
        match self.labels.get_mut(l.0 as usize) {
            Some(slot @ None) => *slot = Some(here),
            Some(Some(_)) => self.fail(BuildError::LabelBoundTwice),
            None => self.fail(BuildError::UnboundLabel),
        }
    }

    /// Raw escape hatch; label fields are NOT resolved for raw instructions of label shapes
    /// unless they hold label ids from this builder.
    pub fn emit(&mut self, inst: Inst) {
        self.code.push(inst);
    }

    /// Line entry for the next instruction.
    pub fn loc(&mut self, file: u32, line: u32, column: u32) {
        let inst = self.code.len() as u32;
        let entry = LineEntry { inst, file, line, column };
        match self.lines.last_mut() {
            Some(last) if last.inst == inst => *last = entry,
            _ => self.lines.push(entry),
        }
    }

    pub fn op0(&mut self, op: Op) {
        if self.shape_check(op, op.shape() == Shape::None) {
            self.emit(Inst::new(op, 0, 0, 0));
        }
    }

    pub fn op1(&mut self, op: Op, d: u16) {
        if self.shape_check(op, op.shape() == Shape::D) {
            self.emit(Inst::new(op, d as u32, 0, 0));
        }
    }

    pub fn op2(&mut self, op: Op, d: u16, a: u16) {
        if self.shape_check(op, op.shape() == Shape::DA) {
            self.emit(Inst::new(op, d as u32, a as u32, 0));
        }
    }

    pub fn op3(&mut self, op: Op, d: u16, a: u16, b: u16) {
        if self.shape_check(op, matches!(op.shape(), Shape::DAB | Shape::ABC)) {
            self.emit(Inst::new(op, d as u32, a as u32, b as u32));
        }
    }

    pub fn const_i32(&mut self, d: u16, v: i32) {
        self.emit(Inst::const_i32(d as u32, v));
    }
    pub fn const_i64(&mut self, d: u16, v: i64) {
        self.emit(Inst::const_i64(d as u32, v));
    }
    pub fn const_f32(&mut self, d: u16, v: f32) {
        self.emit(Inst::const_f32(d as u32, v));
    }
    pub fn const_f64(&mut self, d: u16, v: f64) {
        self.emit(Inst::const_f64(d as u32, v));
    }

    pub fn mov(&mut self, d: u16, a: u16) {
        self.emit(Inst::new(Op::Mov, d as u32, a as u32, 0));
    }

    pub fn select(&mut self, d: u16, cond: u16, a: u16, b: u16) {
        self.emit(Inst::select(d as u32, cond as u32, a as u32, b as u32));
    }

    pub fn load(&mut self, op: Op, d: u16, base: u16, offset: u32) {
        if self.shape_check(op, op.shape() == Shape::Load) {
            self.emit(Inst::new(op, d as u32, base as u32, offset));
        }
    }

    pub fn store(&mut self, op: Op, src: u16, base: u16, offset: u32) {
        if self.shape_check(op, op.shape() == Shape::Store) {
            self.emit(Inst::new(op, src as u32, base as u32, offset));
        }
    }

    pub fn memory_size(&mut self, d: u16) {
        self.emit(Inst::new(Op::MemorySize, d as u32, 0, 0));
    }
    pub fn memory_grow(&mut self, d: u16, pages: u16) {
        self.emit(Inst::new(Op::MemoryGrow, d as u32, pages as u32, 0));
    }
    pub fn memory_copy(&mut self, dst: u16, src: u16, len: u16) {
        self.emit(Inst::new(Op::MemoryCopy, dst as u32, src as u32, len as u32));
    }
    pub fn memory_fill(&mut self, dst: u16, value: u16, len: u16) {
        self.emit(Inst::new(Op::MemoryFill, dst as u32, value as u32, len as u32));
    }

    pub fn global_get(&mut self, d: u16, g: u32) {
        self.emit(Inst::new(Op::GlobalGet, d as u32, g, 0));
    }
    pub fn global_set(&mut self, g: u32, a: u16) {
        self.emit(Inst::new(Op::GlobalSet, g, a as u32, 0));
    }

    pub fn jmp(&mut self, l: Label) {
        self.check_label(l);
        self.emit(Inst::new(Op::Jmp, l.0, 0, 0));
    }
    pub fn br_if(&mut self, a: u16, l: Label) {
        self.check_label(l);
        self.emit(Inst::new(Op::BrIf, a as u32, l.0, 0));
    }
    pub fn br_ifz(&mut self, a: u16, l: Label) {
        self.check_label(l);
        self.emit(Inst::new(Op::BrIfz, a as u32, l.0, 0));
    }

    pub fn br_table(&mut self, a: u16, targets: &[Label], default: Label) {
        for &l in targets.iter().chain(std::iter::once(&default)) {
            self.check_label(l);
        }
        let idx = self.br_tables.len() as u32;
        self.br_tables.push(BrTable {
            targets: targets.iter().map(|l| l.0).collect(),
            default: default.0,
        });
        self.emit(Inst::new(Op::BrTable, a as u32, idx, 0));
    }

    pub fn call(&mut self, f: u32, base: u16) {
        self.emit(Inst::new(Op::Call, f, base as u32, 0));
    }
    pub fn call_indirect(&mut self, type_idx: u32, a: u16, base: u16) {
        self.emit(Inst::new(Op::CallIndirect, type_idx, a as u32, base as u32));
    }
    pub fn ret(&mut self, base: u16) {
        self.emit(Inst::new(Op::Ret, base as u32, 0, 0));
    }
    pub fn yield_(&mut self) {
        self.emit(Inst::new(Op::Yield, 0, 0, 0));
    }
    pub fn unreachable(&mut self) {
        self.emit(Inst::new(Op::Unreachable, 0, 0, 0));
    }
    pub fn nop(&mut self) {
        self.emit(Inst::new(Op::Nop, 0, 0, 0));
    }

    /// Resolves labels into a `Function` plus its line entries.
    fn resolve(self, type_idx: u32) -> Result<(Function, Vec<LineEntry>), BuildError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let labels = self.labels;
        let lookup = |id: u32| -> Result<u32, BuildError> {
            labels.get(id as usize).copied().flatten().ok_or(BuildError::UnboundLabel)
        };
        let mut code = self.code;
        for inst in &mut code {
            match inst.op.shape() {
                Shape::Jmp => inst.a = lookup(inst.a)?,
                Shape::Br => inst.b = lookup(inst.b)?,
                _ => {}
            }
        }
        let mut br_tables = self.br_tables;
        for t in &mut br_tables {
            for target in &mut t.targets {
                *target = lookup(*target)?;
            }
            t.default = lookup(t.default)?;
        }
        Ok((Function { type_idx, nregs: self.nregs, code, br_tables }, self.lines))
    }
}

/// Builds a whole module.
#[derive(Debug, Default)]
pub struct ModuleBuilder {
    types: Vec<FuncType>,
    type_ids: HashMap<FuncType, u32>,
    imports: Vec<Import>,
    funcs: Vec<(u32, Option<FunctionBuilder>)>,
    table: Vec<Option<u32>>,
    memory: MemoryType,
    globals: Vec<Global>,
    exports: Vec<Export>,
    data: Vec<DataSegment>,
    func_names: Vec<(u32, String)>,
    reg_names: Vec<(u32, u32, String)>,
    files: Vec<String>,
    file_ids: HashMap<String, u32>,
    errors: Vec<BuildError>,
    api: Option<ApiRef>,
    interface: Option<Interface>,
    events: Vec<EventRef>,
}

impl ModuleBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a function type, reusing an identical existing one.
    pub fn add_type(&mut self, ty: FuncType) -> u32 {
        if let Some(&i) = self.type_ids.get(&ty) {
            return i;
        }
        let i = self.types.len() as u32;
        self.type_ids.insert(ty.clone(), i);
        self.types.push(ty);
        i
    }

    /// Declares an import; returns its function index.
    pub fn import(&mut self, namespace: &str, name: &str, ty: FuncType) -> u32 {
        if !self.funcs.is_empty() {
            self.errors.push(BuildError::ImportAfterFunction);
        }
        let type_idx = self.add_type(ty);
        self.imports.push(Import {
            namespace: namespace.to_string(),
            name: name.to_string(),
            type_idx,
        });
        (self.imports.len() - 1) as u32
    }

    /// Declares a function without a body; returns its function index.
    pub fn declare_func(&mut self, ty: FuncType) -> u32 {
        let type_idx = self.add_type(ty);
        self.funcs.push((type_idx, None));
        (self.imports.len() + self.funcs.len() - 1) as u32
    }

    /// Supplies the body for a declared function.
    pub fn define(&mut self, func: u32, body: FunctionBuilder) {
        let slot = (func as usize)
            .checked_sub(self.imports.len())
            .and_then(|i| self.funcs.get_mut(i));
        match slot {
            Some(s) => s.1 = Some(body),
            None => self.errors.push(BuildError::UndefinedFunction(func)),
        }
    }

    /// Records the API index this module is built against (section 12): the index hash and
    /// `(namespace, version)` for each namespace it imports from. Replaces an earlier call.
    pub fn api_ref<S: Into<String>>(&mut self, hash: u64, namespaces: impl IntoIterator<Item = (S, u32)>) {
        let namespaces = namespaces.into_iter().map(|(n, v)| (n.into(), v)).collect();
        self.api = Some(ApiRef { hash, namespaces });
    }

    /// Sets section 13 `interface` (SPEC.md 9.2): what the module provides when loaded as a
    /// mod. Replaces an earlier call. Its rules are checked by `finish`.
    pub fn interface(&mut self, interface: Interface) {
        self.interface = Some(interface);
    }

    /// Adds a reference to script event `namespace.name` to section 14 (SPEC.md 9.3) and
    /// returns its event id (its index in the section), the value `engine.events.*` take.
    /// `payload` is `(struct name, size in bytes)` from the API index, `None` for an event
    /// without payload. A second call with the same `namespace.name` returns the first id;
    /// if the payload differs it is still added, and `finish` rejects the duplicate.
    pub fn event_ref(&mut self, namespace: &str, name: &str, payload: Option<(&str, u32)>) -> u32 {
        let payload = payload.map(|(s, n)| (s.to_string(), n));
        if let Some(i) = self.events.iter().position(|e| e.namespace == namespace && e.name == name && e.payload == payload) {
            return i as u32;
        }
        self.events.push(EventRef { namespace: namespace.to_string(), name: name.to_string(), payload });
        (self.events.len() - 1) as u32
    }

    pub fn add_func(&mut self, ty: FuncType, body: FunctionBuilder) -> u32 {
        let f = self.declare_func(ty);
        self.define(f, body);
        f
    }

    pub fn memory(&mut self, min_pages: u32, max_pages: Option<u32>) {
        self.memory = MemoryType { min_pages, max_pages };
    }

    pub fn global(&mut self, ty: ValType, mutable: bool, init: u64) -> u32 {
        self.globals.push(Global { ty, mutable, init });
        (self.globals.len() - 1) as u32
    }

    /// Adds an i32 global; the init bits are `v as u32` zero-extended.
    pub fn global_i32(&mut self, mutable: bool, v: i32) -> u32 {
        self.global(ValType::I32, mutable, u64::from(v as u32))
    }
    pub fn global_i64(&mut self, mutable: bool, v: i64) -> u32 {
        self.global(ValType::I64, mutable, v as u64)
    }
    pub fn global_f32(&mut self, mutable: bool, v: f32) -> u32 {
        self.global(ValType::F32, mutable, u64::from(v.to_bits()))
    }
    pub fn global_f64(&mut self, mutable: bool, v: f64) -> u32 {
        self.global(ValType::F64, mutable, v.to_bits())
    }

    pub fn table(&mut self, entries: Vec<Option<u32>>) {
        self.table = entries;
    }

    pub fn export_func(&mut self, name: &str, func: u32) {
        self.exports.push(Export { name: name.to_string(), kind: ExportKind::Func(func) });
    }

    pub fn export_global(&mut self, name: &str, global: u32) {
        self.exports.push(Export { name: name.to_string(), kind: ExportKind::Global(global) });
    }

    pub fn data(&mut self, offset: u64, bytes: &[u8]) {
        self.data.push(DataSegment { offset, bytes: bytes.to_vec() });
    }

    pub fn func_name(&mut self, func: u32, name: &str) {
        self.func_names.push((func, name.to_string()));
    }

    pub fn reg_name(&mut self, func: u32, reg: u16, name: &str) {
        self.reg_names.push((func, reg as u32, name.to_string()));
    }

    /// Registers a source file name; returns its id for `FunctionBuilder::loc`.
    pub fn file(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.file_ids.get(name) {
            return i;
        }
        let i = self.files.len() as u32;
        self.file_ids.insert(name.to_string(), i);
        self.files.push(name.to_string());
        i
    }

    /// Resolves labels, then verifies.
    pub fn finish(self) -> Result<Module, LoadError> {
        Module::new(self.finish_unverified()?)
    }

    /// Resolves labels only.
    pub fn finish_unverified(self) -> Result<ModuleData, LoadError> {
        if let Some(e) = self.errors.into_iter().next() {
            return Err(LoadError::Build(e));
        }
        let nimports = self.imports.len() as u32;
        let mut functions = Vec::with_capacity(self.funcs.len());
        let mut line_funcs = Vec::new();
        for (i, (type_idx, body)) in self.funcs.into_iter().enumerate() {
            let func_index = nimports + i as u32;
            let body = body.ok_or(LoadError::Build(BuildError::UndefinedFunction(func_index)))?;
            let (f, lines) = body.resolve(type_idx).map_err(LoadError::Build)?;
            if !lines.is_empty() {
                line_funcs.push(FuncLines { func: func_index, entries: lines });
            }
            functions.push(f);
        }
        let mut data = ModuleData {
            types: self.types,
            imports: self.imports,
            functions,
            table: self.table,
            memory: self.memory,
            globals: self.globals,
            exports: self.exports,
            data: self.data,
            api: self.api,
            interface: self.interface,
            events: self.events,
            ..Default::default()
        };
        data.names.funcs = self.func_names;
        data.names.regs = self.reg_names;
        data.lines.files = self.files;
        data.lines.funcs = line_funcs;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> FuncType {
        FuncType::default()
    }

    #[test]
    fn labels_forward_backward_and_table() {
        let mut m = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(2);
        let top = f.new_label();
        let end = f.new_label();
        let other = f.new_label();
        f.bind(top); // 0
        f.br_ifz(0, end); // 0 -> 5
        f.jmp(top); // 1 -> 0
        f.br_table(1, &[top, other], end); // 2
        f.bind(other); // 3
        f.nop(); // 3
        f.nop(); // 4
        f.bind(end); // 5
        f.ret(0);
        let idx = m.add_func(unit(), f);
        m.export_func("f", idx);
        let d = m.finish_unverified().unwrap();
        let code = &d.functions[0].code;
        assert_eq!(code[0].b, 5);
        assert_eq!(code[1].a, 0);
        assert_eq!(d.functions[0].br_tables[0].targets, vec![0, 3]);
        assert_eq!(d.functions[0].br_tables[0].default, 5);
    }

    #[test]
    fn nregs_helpers() {
        let mut f = FunctionBuilder::new(2);
        assert_eq!(f.nregs(), 2);
        f.ensure_reg(1);
        assert_eq!(f.nregs(), 2);
        f.ensure_reg(5);
        assert_eq!(f.nregs(), 6);
        f.set_nregs(3);
        assert_eq!(f.nregs(), 3);
        f.ensure_reg(65534);
        assert_eq!(f.nregs(), 65535);
        f.ret(0);
        let mut m = ModuleBuilder::new();
        m.add_func(unit(), f);
        assert!(m.finish_unverified().is_ok());

        let mut f = FunctionBuilder::new(0);
        f.ensure_reg(65535);
        f.ret(0);
        let mut m = ModuleBuilder::new();
        m.add_func(unit(), f);
        assert_eq!(
            m.finish_unverified().unwrap_err(),
            LoadError::Build(BuildError::TooManyRegisters)
        );
    }

    #[test]
    fn len_and_is_empty() {
        let mut f = FunctionBuilder::new(1);
        assert!(f.is_empty());
        assert_eq!(f.len(), 0);
        f.nop();
        f.nop();
        assert!(!f.is_empty());
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn typed_globals() {
        let mut m = ModuleBuilder::new();
        let a = m.global_i32(true, -1);
        let b = m.global_i64(false, -2);
        let c = m.global_f32(false, 1.5);
        let d = m.global_f64(true, -2.5);
        assert_eq!((a, b, c, d), (0, 1, 2, 3));
        let data = m.finish_unverified().unwrap();
        let g = &data.globals;
        assert_eq!((g[0].ty, g[0].mutable, g[0].init), (ValType::I32, true, 0xFFFF_FFFF));
        assert_eq!((g[1].ty, g[1].mutable, g[1].init), (ValType::I64, false, (-2i64) as u64));
        assert_eq!((g[2].ty, g[2].init), (ValType::F32, u64::from(1.5f32.to_bits())));
        assert_eq!((g[3].ty, g[3].init), (ValType::F64, (-2.5f64).to_bits()));
    }

    #[test]
    fn foreign_label_out_of_range() {
        let mut other = FunctionBuilder::new(0);
        other.new_label();
        let foreign = other.new_label();
        let mut f = FunctionBuilder::new(0);
        f.jmp(foreign);
        f.ret(0);
        let mut m = ModuleBuilder::new();
        m.add_func(unit(), f);
        assert_eq!(m.finish_unverified().unwrap_err(), LoadError::Build(BuildError::UnboundLabel));
    }

    #[test]
    fn dedupe_files() {
        let mut m = ModuleBuilder::new();
        let a = m.file("a");
        let b = m.file("b");
        assert_eq!(m.file("a"), a);
        assert_ne!(a, b);
    }

    #[test]
    fn dedupe_types() {
        let mut m = ModuleBuilder::new();
        let a = m.add_type(FuncType::new([ValType::I32], []));
        let b = m.add_type(FuncType::new([ValType::I32], []));
        let c = m.add_type(FuncType::new([ValType::I64], []));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn import_after_function() {
        let mut m = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(0);
        f.ret(0);
        m.add_func(unit(), f);
        m.import("env", "x", unit());
        assert_eq!(
            m.finish_unverified().unwrap_err(),
            LoadError::Build(BuildError::ImportAfterFunction)
        );
    }

    #[test]
    fn wrong_shape() {
        let mut m = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(3);
        f.op3(Op::I32Load, 0, 1, 2);
        f.ret(0);
        m.add_func(unit(), f);
        assert_eq!(
            m.finish_unverified().unwrap_err(),
            LoadError::Build(BuildError::WrongShape(Op::I32Load))
        );
    }

    #[test]
    fn unbound_label_and_undefined_function() {
        let mut m = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(0);
        let l = f.new_label();
        f.jmp(l);
        m.add_func(unit(), f);
        assert_eq!(m.finish_unverified().unwrap_err(), LoadError::Build(BuildError::UnboundLabel));

        let mut m = ModuleBuilder::new();
        m.import("a", "b", unit());
        let f = m.declare_func(unit());
        assert_eq!(f, 1);
        assert_eq!(
            m.finish_unverified().unwrap_err(),
            LoadError::Build(BuildError::UndefinedFunction(1))
        );
    }

    #[test]
    fn bound_twice_and_lines() {
        let mut m = ModuleBuilder::new();
        let file = m.file("a.efe");
        let mut f = FunctionBuilder::new(1);
        f.loc(file, 3, 4);
        f.nop();
        f.ret(0);
        let i = m.add_func(unit(), f);
        let d = m.finish_unverified().unwrap();
        assert_eq!(d.lines.funcs[0].func, i);
        assert_eq!(d.lines.files, vec!["a.efe".to_string()]);

        let mut m = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(0);
        let l = f.new_label();
        f.bind(l);
        f.bind(l);
        f.ret(0);
        m.add_func(unit(), f);
        assert_eq!(
            m.finish_unverified().unwrap_err(),
            LoadError::Build(BuildError::LabelBoundTwice)
        );
    }
}
