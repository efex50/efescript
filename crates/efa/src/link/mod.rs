//! The linker: in-memory **units** (one per source file, any front end) linked into one
//! `efevm` module in two phases (`docs/e8-design.md` D4 and section 4; the `.efa` rules it
//! implements are `docs/efa-syntax.md` section 16).
//!
//! 1. **Layout** ([`layout`]): every unit [`Unit::declare`]s its symbols with everything that
//!    decides indices and addresses. The linker merges imports, numbers functions and globals,
//!    lays out data and the scratch region, sizes the memory and runs the program-wide checks
//!    (export and provide names, one table, `memory`/`scratch` only in the root). The result
//!    is a [`SymbolMap`].
//! 2. **Emit**: every unit [`Unit::resolve`]s its layout-dependent values (global initial
//!    values, data bytes, table entries); the linker writes imports, function declarations,
//!    globals, table, memory and data in the builder order of efa-syntax section 7; then every
//!    unit [`Unit::define`]s its function bodies (one `file` entry per unit in the lines
//!    section); the linker writes exports, names (functions qualified by the unit's
//!    namespace, then register names), the API reference and section 13, and verifies once.
//!
//! No relocation exists and nothing is patched afterwards. A program of one unit produces
//! exactly the builder calls of the single-file assembler, so its bytes are unchanged.
//!
//! Every phase collects diagnostics into a sink and carries on, so one run reports as much
//! as it can; `finish()` runs only when no diagnostic was produced.

use std::collections::HashMap;

use efevm::api::{ApiIndex, ApiType, Param, StructDesc};
use efevm::{FuncType, FunctionBuilder, Interface, LoadError, ModuleBuilder, Provide, ValType};

pub use crate::asm::ast::Span;
pub use crate::asm::diag::Diagnostic;

pub mod layout;
#[cfg(test)]
mod tests;

/// One source file (or any other front-end unit) of a program.
///
/// The linker calls [`declare`](Unit::declare) on every unit, then
/// [`resolve`](Unit::resolve) on every unit, then [`define`](Unit::define) on every unit,
/// always in unit order. Unit 0 is the root. Diagnostics go into the sink; a unit reports
/// each problem once, in the phase that finds it.
pub trait Unit {
    /// The file name used in diagnostics and in the lines section.
    fn file(&self) -> &str;
    /// The namespace that qualifies this unit's function names in the names section
    /// (`util.math`); empty for the root.
    fn namespace(&self) -> &str;
    /// The units (by index) whose declarations this unit reads while declaring (the files it
    /// `use`s): they are declared first, so [`Declared`] holds their values. A cycle is
    /// tolerated; a unit of the cycle may then miss the values of a later one.
    fn deps(&self) -> Vec<usize> {
        Vec::new()
    }
    /// Everything that decides indices and layout. No bodies, no layout-dependent values.
    /// `prior` holds the declarations of the units named by [`deps`](Unit::deps).
    fn declare(&mut self, prior: &Declared<'_>, diags: &mut Vec<Diagnostic>) -> Decls;
    /// The values that depend on the layout, with every symbol known.
    fn resolve(&mut self, cx: &ResolveCx<'_>, diags: &mut Vec<Diagnostic>) -> Resolved;
    /// Emits the function bodies (with `cx.define`) and register names (with `cx.reg_name`).
    fn define(&mut self, cx: &mut DefineCx<'_>, diags: &mut Vec<Diagnostic>);
    /// A diagnostic at `span` of this unit. The default has no source line; a unit with
    /// source text overrides it so the snippet is shown.
    fn diagnostic(&self, span: Span, message: String) -> Diagnostic {
        Diagnostic::with_line(self.file(), "", span, message)
    }
}

/// A `const` value: an integer, or the source text of a float literal (efa-syntax 1), so
/// that each use parses it directly as `f32` or `f64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstValue {
    Int(i128),
    Float(String),
}

/// `import ns.name` with its resolved signature.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDecl {
    pub namespace: String,
    pub name: String,
    pub ty: FuncType,
    pub span: Span,
}

/// A function of the unit. Its index is assigned by the linker.
#[derive(Clone, Debug, PartialEq)]
pub struct FuncDecl {
    pub name: String,
    pub ty: FuncType,
    /// Visible to other units (`pub` or `export`).
    pub public: bool,
    /// The export name, if the function is exported.
    pub export: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlobalDecl {
    pub name: String,
    pub ty: ValType,
    pub mutable: bool,
    pub public: bool,
    pub export: Option<String>,
    pub span: Span,
}

/// A data item: its size and alignment decide the layout; its bytes come in
/// [`Resolved::data`].
#[derive(Clone, Debug, PartialEq)]
pub struct DataDecl {
    pub name: String,
    pub size: u64,
    /// A power of two (8 for `.efa`); anything else is treated as 1.
    pub align: u64,
    pub public: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstDecl {
    pub name: String,
    pub value: ConstValue,
    pub public: bool,
    pub span: Span,
}

/// A struct layout of the unit (`owner` empty).
#[derive(Clone, Debug, PartialEq)]
pub struct StructDecl {
    pub desc: StructDesc,
    pub public: bool,
    /// `export struct`: one of the module's own types in section 13.
    pub export: bool,
    pub span: Span,
}

/// A reference to a named item of unit `unit`, written at `span` of the declaring unit.
#[derive(Clone, Debug, PartialEq)]
pub struct SymRef {
    pub unit: usize,
    pub name: String,
    pub span: Span,
}

/// The type of a provide parameter or result.
#[derive(Clone, Debug, PartialEq)]
pub enum ProvideType {
    /// Any `ApiType` but `Struct` (and `Ptr`, which the linker refuses).
    Api(ApiType),
    /// `&S` for a struct of a unit of the program; it must be `export`.
    Struct(SymRef),
    /// `&S` for a struct of the API index; a copy with its owner goes into section 13.
    IndexStruct(StructDesc),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProvideParam {
    pub name: String,
    pub ty: ProvideType,
    pub span: Span,
}

/// `provide NAME(params) -> (results) = target, "doc"`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvideDecl {
    pub name: String,
    pub params: Vec<ProvideParam>,
    pub results: Vec<ProvideParam>,
    pub doc: String,
    pub target: SymRef,
    pub span: Span,
}

/// `import event ns.name [(&S)]`: an entry of section 14 (efa-syntax 17.1).
#[derive(Clone, Debug, PartialEq)]
pub struct EventImportDecl {
    pub namespace: String,
    pub name: String,
    /// The payload struct: `Struct` (a struct of a unit of the program) or `IndexStruct`
    /// (from the API index); `None` for an event without payload.
    pub payload: Option<ProvideType>,
    /// The event is not in the API index although one was given: only valid when the program
    /// provides an event of the same name (a mod sending its own event, 17.3).
    pub unindexed: bool,
    /// Note for the "unknown event" error (a "did you mean").
    pub hint: Option<String>,
    pub span: Span,
}

/// `provide event NAME [(&S)], "doc"` (efa-syntax 17.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ProvideEventDecl {
    pub name: String,
    /// `Struct` (must be `export`) or `IndexStruct` (a copy goes into section 13).
    pub payload: Option<ProvideType>,
    pub doc: String,
    pub span: Span,
}

/// `memory MIN [MAX]`; `min` is `None` when the unit could not evaluate it (already reported).
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryDecl {
    pub min: Option<u32>,
    pub max: Option<u32>,
    pub span: Span,
}

/// `scratch N`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScratchDecl {
    pub size: u32,
    pub span: Span,
}

/// The declarations of the units declared so far, by unit index (`None`: not declared yet),
/// with their file names.
#[derive(Clone, Copy, Debug)]
pub struct Declared<'a> {
    pub decls: &'a [Option<Decls>],
    pub files: &'a [String],
}

impl Declared<'_> {
    pub const EMPTY: Declared<'static> = Declared { decls: &[], files: &[] };
}

/// A value name of a unit that is known when the unit is declared.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticValue {
    pub name: String,
    /// `None` for a data address: it exists, but only the layout knows it.
    pub value: Option<ConstValue>,
    pub public: bool,
    pub span: Span,
}

/// The value names of `d` that other units can use before the layout exists: consts, data
/// symbols (without an address), `x.len` and the constants of the structs.
pub fn static_values(d: &Decls) -> Vec<StaticValue> {
    let mut v = Vec::new();
    for c in &d.consts {
        v.push(StaticValue { name: c.name.clone(), value: Some(c.value.clone()), public: c.public, span: c.span });
    }
    for x in &d.data {
        v.push(StaticValue { name: x.name.clone(), value: None, public: x.public, span: x.span });
        v.push(StaticValue {
            name: format!("{}.len", x.name),
            value: Some(ConstValue::Int(i128::from(x.size))),
            public: x.public,
            span: x.span,
        });
    }
    for s in &d.structs {
        for (name, val) in struct_consts(&s.desc) {
            v.push(StaticValue {
                name,
                value: Some(ConstValue::Int(val)),
                public: s.public || s.export,
                span: s.span,
            });
        }
    }
    v
}

/// Everything a unit declares, in source order per kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decls {
    pub imports: Vec<ImportDecl>,
    pub funcs: Vec<FuncDecl>,
    pub globals: Vec<GlobalDecl>,
    pub data: Vec<DataDecl>,
    pub consts: Vec<ConstDecl>,
    pub structs: Vec<StructDecl>,
    pub provides: Vec<ProvideDecl>,
    /// `import event` items, in source order (each event once per unit).
    pub event_imports: Vec<EventImportDecl>,
    /// `provide event` items, in source order.
    pub provided_events: Vec<ProvideEventDecl>,
    pub memory: Option<MemoryDecl>,
    /// The span of the unit's `table` item (its entries come in [`Resolved::table`]).
    pub table: Option<Span>,
    pub scratch: Option<ScratchDecl>,
}

/// The layout-dependent values of a unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    /// The initial value bits of every global of [`Decls::globals`], in order.
    pub globals: Vec<u64>,
    /// The bytes of every data item of [`Decls::data`], in order; empty for an item without
    /// a segment (`resb`). At most the declared size.
    pub data: Vec<Vec<u8>>,
    /// The entries of the unit's table (function indices), if it declared one.
    pub table: Option<Vec<Option<u32>>>,
}

/// The name space a symbol lives in. Functions, globals and values (consts, data symbols,
/// `x.len`, struct constants) are separate, as in efa-syntax section 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    Func,
    Global,
    Value,
    Struct,
}

/// What a name of a unit means after layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Symbol {
    Func { index: u32, ty: FuncType },
    Global { index: u32, ty: ValType, mutable: bool },
    /// A const, a data address, `x.len` or a struct constant (`S.size`, `S.align`, `S.f`).
    Value(ConstValue),
    Struct(StructDesc),
}

/// One entry of [`SymbolMap::symbols`].
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolEntry {
    pub space: Space,
    pub name: String,
    pub symbol: Symbol,
    pub public: bool,
    /// Where it is declared (in the unit's file).
    pub span: Span,
}

/// Why [`SymbolMap::lookup`] found nothing usable.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupError {
    Unknown,
    /// The item exists but is private to `file`, declared at `span`.
    Private { file: String, span: Span },
}

impl LookupError {
    /// The diagnostic for `name` used at `span` of unit `from` (a private item gets the
    /// ``x is private to `file` `` error with a note pointing at its declaration).
    pub fn diagnostic(&self, from: &dyn Unit, span: Span, what: &str, name: &str) -> Diagnostic {
        match self {
            LookupError::Unknown => from.diagnostic(span, format!("unknown {what} `{name}`")),
            LookupError::Private { file, span: decl } => {
                let short = name.rsplit('.').next().unwrap_or(name);
                private_error(from, span, short, file, *decl)
            }
        }
    }
}

/// ``x is private to `file` `` with the note "declared at file:line; mark it `pub`".
pub fn private_error(from: &dyn Unit, span: Span, name: &str, file: &str, decl: Span) -> Diagnostic {
    let (msg, note) = private_message(name, file, decl);
    from.diagnostic(span, msg).with_note(note)
}

/// The message and note of [`private_error`]; `name` is the item's own name (`x`, `x.len`).
pub fn private_message(name: &str, file: &str, decl: Span) -> (String, String) {
    (
        format!("`{name}` is private to `{file}`"),
        format!("declared at {file}:{}; mark it `pub` to use it from other files", decl.line),
    )
}

/// A unit's part of the [`SymbolMap`].
#[derive(Clone, Debug, PartialEq)]
pub struct UnitSyms {
    pub file: String,
    pub namespace: String,
    pub decls: Decls,
    /// Function index of every import of [`Decls::imports`].
    pub imports: Vec<u32>,
    /// Function index of every function of [`Decls::funcs`].
    pub funcs: Vec<u32>,
    /// Global index of every global of [`Decls::globals`].
    pub globals: Vec<u32>,
    /// Address of every data item of [`Decls::data`].
    pub data: Vec<u64>,
    /// Section 14 id of every event of [`Decls::event_imports`].
    pub events: Vec<u32>,
}

/// The result of phase 1: every index and address of the program.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SymbolMap {
    pub units: Vec<UnitSyms>,
    /// `(namespace, name, type)` of every module import, by import index.
    pub imports: Vec<(String, String, FuncType)>,
    /// Section 14: `(namespace, name, payload (struct name, size))` by event id, merged
    /// across units in order of first appearance.
    pub events: Vec<(String, String, Option<(String, u32)>)>,
    /// `(min, max)` pages.
    pub memory: (u32, Option<u32>),
    /// `(offset, size)` of the scratch region, if there is one.
    pub scratch: Option<(u64, u32)>,
    /// End of the data (and scratch region); 0 when the program has neither.
    pub data_end: u64,
}

impl SymbolMap {
    pub fn unit(&self, u: usize) -> Option<&UnitSyms> {
        self.units.get(u)
    }

    /// Every symbol of unit `u` (public and private), derived names included: a data item
    /// `x` gives `x` and `x.len`, a struct `S` gives `S`, `S.size`, `S.align` and `S.<field>`
    /// (a field named `size` or `align` loses against the structural constant).
    pub fn symbols(&self, u: usize) -> Vec<SymbolEntry> {
        let Some(us) = self.units.get(u) else { return Vec::new() };
        let d = &us.decls;
        let mut v = Vec::new();
        for (k, f) in d.funcs.iter().enumerate() {
            let index = us.funcs.get(k).copied().unwrap_or(u32::MAX);
            v.push(SymbolEntry {
                space: Space::Func,
                name: f.name.clone(),
                symbol: Symbol::Func { index, ty: f.ty.clone() },
                public: f.public,
                span: f.span,
            });
        }
        for (k, g) in d.globals.iter().enumerate() {
            let index = us.globals.get(k).copied().unwrap_or(u32::MAX);
            v.push(SymbolEntry {
                space: Space::Global,
                name: g.name.clone(),
                symbol: Symbol::Global { index, ty: g.ty, mutable: g.mutable },
                public: g.public,
                span: g.span,
            });
        }
        for c in &d.consts {
            v.push(SymbolEntry {
                space: Space::Value,
                name: c.name.clone(),
                symbol: Symbol::Value(c.value.clone()),
                public: c.public,
                span: c.span,
            });
        }
        for (k, x) in d.data.iter().enumerate() {
            let addr = us.data.get(k).copied().unwrap_or(0);
            v.push(SymbolEntry {
                space: Space::Value,
                name: x.name.clone(),
                symbol: Symbol::Value(ConstValue::Int(i128::from(addr))),
                public: x.public,
                span: x.span,
            });
            v.push(SymbolEntry {
                space: Space::Value,
                name: format!("{}.len", x.name),
                symbol: Symbol::Value(ConstValue::Int(i128::from(x.size))),
                public: x.public,
                span: x.span,
            });
        }
        for s in &d.structs {
            v.push(SymbolEntry {
                space: Space::Struct,
                name: s.desc.name.clone(),
                symbol: Symbol::Struct(s.desc.clone()),
                public: s.public || s.export,
                span: s.span,
            });
            for (name, val) in struct_consts(&s.desc) {
                v.push(SymbolEntry {
                    space: Space::Value,
                    name,
                    symbol: Symbol::Value(ConstValue::Int(val)),
                    public: s.public || s.export,
                    span: s.span,
                });
            }
        }
        v
    }

    /// `name` in `space` of unit `target`, used from unit `from`: any item of the same unit,
    /// only public items of another unit. The first declaration of a name wins.
    pub fn lookup(&self, from: usize, target: usize, space: Space, name: &str) -> Result<Symbol, LookupError> {
        let found = self.symbols(target).into_iter().find(|e| e.space == space && e.name == name);
        match found {
            None => Err(LookupError::Unknown),
            Some(e) if e.public || from == target => Ok(e.symbol),
            Some(e) => Err(LookupError::Private {
                file: self.units.get(target).map(|u| u.file.clone()).unwrap_or_default(),
                span: e.span,
            }),
        }
    }
}

/// `S.size`, `S.align` and `S.<field>` (offset) of a struct; `size`/`align` win over fields.
pub fn struct_consts(s: &StructDesc) -> Vec<(String, i128)> {
    let mut v = vec![
        (format!("{}.size", s.name), i128::from(s.size)),
        (format!("{}.align", s.name), i128::from(s.align)),
    ];
    if let Some(id) = s.component {
        v.push((format!("{}.id", s.name), i128::from(id)));
    }
    for f in &s.fields {
        let n = format!("{}.{}", s.name, f.name);
        if !v.iter().any(|(m, _)| *m == n) {
            v.push((n, i128::from(f.offset)));
        }
    }
    v
}

/// What phase 2a hands a unit.
pub struct ResolveCx<'a> {
    pub syms: &'a SymbolMap,
    /// The index of the unit being resolved.
    pub unit: usize,
}

/// What phase 2c hands a unit: the one shared builder, the layout, and collectors for the
/// statement spans (to map verification errors back) and register names (written after the
/// function names, as efa-syntax section 7 step 9 requires).
pub struct DefineCx<'a> {
    pub builder: &'a mut ModuleBuilder,
    pub syms: &'a SymbolMap,
    pub unit: usize,
    /// The unit's file id in the lines section (`ModuleBuilder::file`).
    pub file_id: u32,
    funcs: &'a mut HashMap<u32, FuncRecord>,
    regs: &'a mut Vec<(u32, u16, String)>,
}

/// Where the instructions of one defined function came from.
struct FuncRecord {
    unit: usize,
    spans: Vec<Span>,
    name_span: Span,
}

impl DefineCx<'_> {
    /// Supplies the body of function `func` (an index from the [`SymbolMap`]). `spans` holds
    /// the source position of every instruction; `name_span` is used for errors about the
    /// whole function.
    pub fn define(&mut self, func: u32, body: FunctionBuilder, spans: Vec<Span>, name_span: Span) {
        self.builder.define(func, body);
        self.funcs.insert(func, FuncRecord { unit: self.unit, spans, name_span });
    }

    /// Names register `reg` of function `func`; written in call order after all function names.
    pub fn reg_name(&mut self, func: u32, reg: u16, name: &str) {
        self.regs.push((func, reg, name.to_string()));
    }
}

/// Options of [`link`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkOptions<'a> {
    /// The API index the units were checked against: with it the module gets an `api`
    /// section (index hash and the version of every imported namespace).
    pub api: Option<&'a ApiIndex>,
}

/// Declares every unit, the units a unit depends on first (depth-first from the root; a cycle
/// is cut at the back edge). Returns the declarations in unit order.
pub(crate) fn declare_all(units: &mut [&mut dyn Unit], diags: &mut Vec<Diagnostic>) -> Vec<Decls> {
    let n = units.len();
    let files: Vec<String> = units.iter().map(|u| u.file().to_string()).collect();
    let deps: Vec<Vec<usize>> = units.iter().map(|u| u.deps()).collect();
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for root in 0..n {
        let mut stack = vec![(root, 0usize)];
        while let Some((u, k)) = stack.pop() {
            if k == 0 {
                if seen[u] {
                    continue;
                }
                seen[u] = true;
            }
            match deps[u].get(k) {
                Some(&d) => {
                    stack.push((u, k + 1));
                    if d < n && !seen[d] {
                        stack.push((d, 0));
                    }
                }
                None => order.push(u),
            }
        }
    }
    let mut decls: Vec<Option<Decls>> = vec![None; n];
    for u in order {
        let d = units[u].declare(&Declared { decls: &decls, files: &files }, diags);
        decls[u] = Some(d);
    }
    decls.into_iter().map(Option::unwrap_or_default).collect()
}

/// Links `units` (unit 0 is the root) into one verified module. The diagnostics are not
/// sorted; the caller orders them.
pub fn link(units: &mut [&mut dyn Unit], opts: &LinkOptions) -> Result<efevm::Module, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // ---- Phase 1: declare and lay out. ----
    let decls = declare_all(units, &mut diags);
    let views: Vec<&dyn Unit> = units.iter().map(|u| &**u as &dyn Unit).collect();
    let syms = layout::layout(&views, decls, &mut diags);
    let iface = layout::interface(&views, &syms, &mut diags);
    drop(views);

    // ---- Phase 2a: layout-dependent values. ----
    let mut resolved: Vec<Resolved> = Vec::new();
    for (k, u) in units.iter_mut().enumerate() {
        let cx = ResolveCx { syms: &syms, unit: k };
        resolved.push(u.resolve(&cx, &mut diags));
    }

    // ---- Phase 2b: the module-level builder calls, in the order of efa-syntax section 7. ----
    let mut mb = ModuleBuilder::new();
    for (ns, name, ty) in &syms.imports {
        mb.import(ns, name, ty.clone());
    }
    for (ns, name, payload) in &syms.events {
        mb.event_ref(ns, name, payload.as_ref().map(|(s, n)| (s.as_str(), *n)));
    }
    for us in &syms.units {
        for f in &us.decls.funcs {
            mb.declare_func(f.ty.clone());
        }
    }
    for (us, r) in syms.units.iter().zip(&resolved) {
        for (k, g) in us.decls.globals.iter().enumerate() {
            mb.global(g.ty, g.mutable, r.globals.get(k).copied().unwrap_or(0));
        }
    }
    if let Some(t) = resolved.iter().find_map(|r| r.table.clone()) {
        mb.table(t);
    }
    mb.memory(syms.memory.0, syms.memory.1);
    for (us, r) in syms.units.iter().zip(&resolved) {
        for (k, addr) in us.data.iter().enumerate() {
            if let Some(bytes) = r.data.get(k).filter(|b| !b.is_empty()) {
                mb.data(*addr, bytes);
            }
        }
    }

    // ---- Phase 2c: function bodies, one `file` per unit. ----
    let mut records: HashMap<u32, FuncRecord> = HashMap::new();
    let mut regs: Vec<(u32, u16, String)> = Vec::new();
    for (k, u) in units.iter_mut().enumerate() {
        let file_id = mb.file(u.file());
        let mut cx = DefineCx {
            builder: &mut mb,
            syms: &syms,
            unit: k,
            file_id,
            funcs: &mut records,
            regs: &mut regs,
        };
        u.define(&mut cx, &mut diags);
    }

    // ---- Phase 2d: api reference, exports, names, interface. ----
    if let Some(api) = opts.api {
        let used: std::collections::BTreeMap<&str, u32> = syms
            .imports
            .iter()
            .map(|(ns, _, _)| ns)
            .chain(syms.events.iter().map(|(ns, _, _)| ns))
            .filter_map(|ns| api.namespace(ns).map(|n| (n.name.as_str(), n.version)))
            .collect();
        mb.api_ref(api.hash(), used);
    }
    for us in &syms.units {
        for (k, f) in us.decls.funcs.iter().enumerate() {
            if let (Some(name), Some(i)) = (&f.export, us.funcs.get(k)) {
                mb.export_func(name, *i);
            }
        }
    }
    for us in &syms.units {
        for (k, g) in us.decls.globals.iter().enumerate() {
            if let (Some(name), Some(i)) = (&g.export, us.globals.get(k)) {
                mb.export_global(name, *i);
            }
        }
    }
    for (i, (ns, name, _)) in syms.imports.iter().enumerate() {
        mb.func_name(layout::idx(i), &format!("{ns}.{name}"));
    }
    for us in &syms.units {
        for (k, f) in us.decls.funcs.iter().enumerate() {
            let Some(i) = us.funcs.get(k) else { continue };
            if us.namespace.is_empty() {
                mb.func_name(*i, &f.name);
            } else {
                mb.func_name(*i, &format!("{}.{}", us.namespace, f.name));
            }
        }
    }
    for (f, r, name) in &regs {
        mb.reg_name(*f, *r, name);
    }
    if let Some(iface) = iface {
        mb.interface(iface);
    }

    if !diags.is_empty() {
        return Err(diags);
    }
    match mb.finish() {
        Ok(m) => Ok(m),
        Err(e) => Err(vec![map_load_error(units, &records, &e)]),
    }
}

/// Maps a load error back to the source: a verification error to the statement that
/// produced the instruction, everything else to a location-less diagnostic in the root file.
fn map_load_error(units: &[&mut dyn Unit], records: &HashMap<u32, FuncRecord>, e: &LoadError) -> Diagnostic {
    if let LoadError::Verify { func: Some(f), inst, error } = e {
        if let Some(rec) = records.get(f) {
            let span = match inst {
                Some(i) => rec.spans.get(*i as usize).copied(),
                None => Some(rec.name_span),
            };
            if let (Some(span), Some(u)) = (span, units.get(rec.unit)) {
                return u.diagnostic(span, error.to_string());
            }
        }
    }
    let file = units.first().map(|u| u.file().to_string()).unwrap_or_default();
    Diagnostic::without_location(&file, e.to_string())
}

/// The lowered signature of a provide (`ApiType::lower` of the parameters, then results).
pub(crate) fn provide_type(params: &[Param], results: &[Param]) -> FuncType {
    Provide { params: params.to_vec(), results: results.to_vec(), ..Default::default() }.func_type()
}

pub(crate) fn interface_value(
    scratch: Option<(u64, u32)>,
    structs: Vec<StructDesc>,
    provides: Vec<Provide>,
    events: Vec<efevm::ProvidedEvent>,
) -> Interface {
    Interface { scratch, structs, provides, events, ..Default::default() }
}
