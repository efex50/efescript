//! Lowering: the AST of one `.efa` file as a [`link::Unit`](crate::link::Unit), linked into a
//! verified `efevm::Module` by [`crate::link::link`].
//!
//! The rules (name resolution, immediates, hidden temporaries, call windows, nregs, builder
//! call order, data layout, names and lines) are those of `docs/efa-syntax.md` sections 4 to 9
//! and 16. The work is split along the linker's phases:
//! - [`Unit::declare`](crate::link::Unit::declare): constants, structs, imports, functions,
//!   data sizes, globals, table, memory, scratch, `use` aliases and provides; everything that
//!   decides indices and layout;
//! - `resolve`: patches the indices and addresses the linker assigned, makes the public items
//!   of the used files visible as `alias.item`, and evaluates global initial values, data bytes
//!   and table entries;
//! - `define`: function bodies (`func.rs`).
//!
//! `value.rs` types immediates. It never panics; it collects as many diagnostics as it can,
//! and reports each problem in exactly one phase.

use std::collections::{HashMap, HashSet};

use efevm::api::{ApiIndex, ApiType, Prim, StructDesc};

use efevm::{FuncType, Op, ValType};

use super::ast::{
    self, AType, DataDecl, DataKind, EventForm, EventImportDecl, FuncDecl, GlobalDecl, Ident, ImportDecl,
    Item, Operand, ProvideEventDecl, Span, TableDecl, TableEntry, Ty,
};
use super::diag::{did_you_mean, Diagnostic};
use crate::link::{self, layout::MAX_SCRATCH, DefineCx, ProvideType, ResolveCx, Resolved, Space, Symbol, SymRef};
use consts::{Fail, Fold};
use value::{raw_reg, type_val, Konst, Val};

mod consts;
mod func;
mod value;

/// Reserved words: they cannot be declared as names of anything.
pub(crate) const RESERVED: &[&str] = &[
    "import", "memory", "const", "global", "table", "fn", "export", "mut", "reg", "nregs", "null",
    "db", "dw", "dd", "dq", "resb", "i32", "i64", "f32", "f64", "inf", "nan", "use", "as", "pub",
    "provide", "struct", "scratch",
];

type Res<T> = Result<T, Diagnostic>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum VKind {
    Const,
    /// A const that depends on a data address or an event id: its value is computed in
    /// `resolve` (a placeholder until then).
    LateConst,
    Data,
    DataLen,
    /// `ns.name.id` of an `import event`: known only after the linker numbered the events.
    Event,
}

/// A `const`, a data symbol or a `NAME.len`.
struct ValueInfo {
    pub val: Val,
    pub span: Span,
    pub kind: VKind,
}

/// Where a callable's final index comes from.
#[derive(Clone, Copy)]
enum Slot {
    Import(usize),
    Func(usize),
    /// Another file's function; `index` is already final.
    Ext,
}

/// A callable: an import or a function.
struct Sig {
    pub index: u32,
    pub params: Vec<Ty>,
    pub results: Vec<Ty>,
    pub span: Span,
    slot: Slot,
}

struct GlobalInfo {
    pub index: u32,
    pub ty: Ty,
    pub span: Span,
    /// Position among the file's globals (`None` for another file's global).
    local: Option<usize>,
}

/// Module-level name tables shared by every function body.
struct Env<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    pub funcs: HashMap<String, Sig>,
    pub func_names: Vec<String>,
    pub globals: HashMap<String, GlobalInfo>,
    pub global_names: Vec<String>,
    pub values: HashMap<String, ValueInfo>,
    pub value_names: Vec<String>,
    pub ops: HashMap<&'static str, Op>,
    /// Names of the constants that come from the API index (`Transform.size`, ...).
    pub api_consts: HashSet<String>,
    /// Struct names of this file (they share the value name space with consts and data).
    pub structs: HashMap<String, Span>,
    /// `alias.item` of private items of used files: (file, declaration).
    pub private: HashMap<(Space, String), (String, Span)>,
}

fn idx(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn val_type(t: Ty) -> ValType {
    match t {
        Ty::I32 => ValType::I32,
        Ty::I64 => ValType::I64,
        Ty::F32 => ValType::F32,
        Ty::F64 => ValType::F64,
    }
}

fn func_type(params: &[ast::TypeRef], results: &[ast::TypeRef]) -> FuncType {
    FuncType::new(
        params.iter().map(|t| val_type(t.ty)),
        results.iter().map(|t| val_type(t.ty)),
    )
}

fn ty_of_val(t: ValType) -> Ty {
    match t {
        ValType::I32 => Ty::I32,
        ValType::I64 => Ty::I64,
        ValType::F32 => Ty::F32,
        ValType::F64 => Ty::F64,
    }
}

fn func_type_of(params: &[Ty], results: &[Ty]) -> FuncType {
    FuncType::new(params.iter().map(|t| val_type(*t)), results.iter().map(|t| val_type(*t)))
}

fn split_lines(src: &str) -> Vec<&str> {
    src.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

impl<'a> Env<'a> {
    fn new(file: &'a str, src: &'a str) -> Env<'a> {
        Env {
            file,
            lines: split_lines(src),
            funcs: HashMap::new(),
            func_names: Vec::new(),
            globals: HashMap::new(),
            global_names: Vec::new(),
            values: HashMap::new(),
            value_names: Vec::new(),
            ops: Op::ALL.iter().map(|o| (o.name(), *o)).collect(),
            api_consts: HashSet::new(),
            structs: HashMap::new(),
            private: HashMap::new(),
        }
    }

    fn err(&self, span: Span, msg: impl Into<String>) -> Diagnostic {
        let text = if span.line == 0 {
            ""
        } else {
            self.lines.get((span.line as usize).saturating_sub(1)).copied().unwrap_or("")
        };
        Diagnostic::with_line(self.file, text, span, msg)
    }

    fn first_defined(&self, d: Diagnostic, first: Span) -> Diagnostic {
        if first.line == 0 {
            d
        } else {
            d.with_note(format!("first defined at line {}", first.line))
        }
    }

    /// Checks a declared name: no dots, not reserved, not `r<digits>`.
    fn check_name(&self, id: &Ident, what: &str, diags: &mut Vec<Diagnostic>) -> bool {
        let bad = if id.name.contains('.') {
            Some(format!("`{}` is not a valid {what} name: declared names have no dots", id.name))
        } else if RESERVED.contains(&id.name.as_str()) {
            Some(format!("`{}` is a reserved word and cannot be a {what} name", id.name))
        } else if raw_reg(&id.name).is_some() {
            Some(format!("`{}` is a raw register name and cannot be a {what} name", id.name))
        } else {
            None
        };
        match bad {
            Some(m) => {
                diags.push(self.err(id.span, m));
                false
            }
            None => true,
        }
    }

    fn define_value(
        &mut self,
        name: &str,
        span: Span,
        val: Val,
        kind: VKind,
        diags: &mut Vec<Diagnostic>,
    ) -> bool {
        if let Some(prev) = self.values.get(name).map(|p| p.span).or_else(|| self.structs.get(name).copied()) {
            let d = self.err(span, format!("duplicate definition of `{name}`"));
            diags.push(self.first_defined(d, prev));
            return false;
        }
        self.value_names.push(name.to_string());
        self.values.insert(name.to_string(), ValueInfo { val, span, kind });
        true
    }

    fn define_func(&mut self, name: &str, sig: Sig, diags: &mut Vec<Diagnostic>) {
        if let Some(prev) = self.funcs.get(name) {
            let d = self.err(sig.span, format!("duplicate definition of function `{name}`"));
            diags.push(self.first_defined(d, prev.span));
            return;
        }
        self.func_names.push(name.to_string());
        self.funcs.insert(name.to_string(), sig);
    }

    /// The ``x is private to `file` `` error for `alias.x` in `space`, if `id` names one.
    fn private_err(&self, space: Space, id: &Ident) -> Option<Diagnostic> {
        let (file, decl) = self.private.get(&(space, id.name.clone()))?;
        let short = id.name.split_once('.').map_or(id.name.as_str(), |(_, n)| n);
        let (msg, note) = link::private_message(short, file, *decl);
        Some(self.err(id.span, msg).with_note(note))
    }

    fn unknown_value(&self, id: &Ident) -> Diagnostic {
        if let Some(d) = self.private_err(Space::Value, id) {
            return d;
        }
        let d = self.err(id.span, format!("unknown constant `{}`", id.name));
        match did_you_mean(&id.name, self.value_names.iter()) {
            Some(s) => d.with_note(format!("did you mean `{s}`?")),
            None => d,
        }
    }

    fn unknown_function(&self, id: &Ident) -> Diagnostic {
        if let Some(d) = self.private_err(Space::Func, id) {
            return d;
        }
        let d = self.err(id.span, format!("unknown function `{}`", id.name));
        match did_you_mean(&id.name, self.func_names.iter()) {
            Some(s) => d.with_note(format!("did you mean `{s}`?")),
            None => d,
        }
    }

    fn unknown_global(&self, id: &Ident) -> Diagnostic {
        if let Some(d) = self.private_err(Space::Global, id) {
            return d;
        }
        let d = self.err(id.span, format!("unknown global `{}`", id.name));
        match did_you_mean(&id.name, self.global_names.iter()) {
            Some(s) => d.with_note(format!("did you mean `{s}`?")),
            None => d,
        }
    }

    /// An integer-like or float value of a data / global / table position.
    fn value_of(&self, op: &Operand) -> Res<Val> {
        match op {
            Operand::Int(v, _) => Ok(Val::Int(*v)),
            Operand::Char(c, _) => Ok(Val::Int(*c as i128)),
            Operand::Float(t, _) => Ok(Val::Float(t.clone())),
            Operand::Name(id) => match self.values.get(&id.name) {
                Some(v) => Ok(v.val.clone()),
                None => Err(self.unknown_value(id)),
            },
            Operand::Expr(ce) => self.eval_known(ce),
            other => Err(self.err(
                other.span(),
                format!("expected a literal or a constant, found {}", value::describe(other)),
            )),
        }
    }
}

/// The bytes of a non-`resb` data item.
fn data_bytes(env: &Env, d: &DataDecl, diags: &mut Vec<Diagnostic>) -> Vec<u8> {
    let mut out = Vec::new();
    for v in &d.values {
        if let Err(e) = data_value(env, d.kind, v, &mut out) {
            diags.push(e);
        }
    }
    out
}

fn data_value(env: &Env, kind: DataKind, op: &Operand, out: &mut Vec<u8>) -> Res<()> {
    if let Operand::Str(bytes, span) = op {
        if kind != DataKind::Db {
            return Err(env.err(*span, format!("a string is only valid in `db`, not `{}`", kind.name())));
        }
        out.extend_from_slice(bytes);
        return Ok(());
    }
    let val = env.value_of(op)?;
    let span = op.span();
    let ranged = |lo: i128, hi: i128, what: &str| -> Res<i128> {
        match &val {
            Val::Float(_) => Err(env.err(span, "float literal in an integer position")),
            Val::Int(n) if (lo..=hi).contains(n) => Ok(*n),
            Val::Int(n) => Err(env.err(span, format!("integer {n} does not fit in {what}"))),
        }
    };
    match kind {
        DataKind::Db => out.push(ranged(-128, 255, "a byte (-128..=255)")? as u8),
        DataKind::Dw => {
            out.extend_from_slice(&(ranged(-32768, 65535, "16 bits")? as u16).to_le_bytes())
        }
        DataKind::Dd | DataKind::Dq => {
            let wide = kind == DataKind::Dq;
            let ty = match (&val, wide) {
                (Val::Float(_), false) => Ty::F32,
                (Val::Float(_), true) => Ty::F64,
                (Val::Int(_), false) => Ty::I32,
                (Val::Int(_), true) => Ty::I64,
            };
            match type_val(ty, &val).map_err(|m| env.err(span, m))? {
                Konst::I32(v) => out.extend_from_slice(&(v as u32).to_le_bytes()),
                Konst::F32(f) => out.extend_from_slice(&f.to_bits().to_le_bytes()),
                Konst::I64(v) => out.extend_from_slice(&(v as u64).to_le_bytes()),
                Konst::F64(f) => out.extend_from_slice(&f.to_bits().to_le_bytes()),
            }
        }
        DataKind::Resb => {}
    }
    Ok(())
}

/// The initial value of a global of type `ty`.
fn global_init(env: &Env, g: &GlobalDecl) -> Res<Konst> {
    let val = env.value_of(&g.init)?;
    type_val(g.ty.ty, &val).map_err(|m| env.err(g.init.span(), m))
}

/// The bits of a typed constant, as `ModuleBuilder::global` takes them.
fn konst_bits(k: Konst) -> u64 {
    match k {
        Konst::I32(v) => u64::from(v as u32),
        Konst::I64(v) => v as u64,
        Konst::F32(v) => u64::from(v.to_bits()),
        Konst::F64(v) => v.to_bits(),
    }
}

/// Defines `S.size`, `S.align`, `S.id` (components) and `S.<field>` (byte offset) for every
/// struct of the index. A field named `size`, `align` or `id` loses against the structural
/// constant of the same name.
fn define_api_consts(env: &mut Env, api: &ApiIndex) {
    for t in &api.types {
        let mut consts: Vec<(String, i128)> = vec![
            (format!("{}.size", t.name), i128::from(t.size)),
            (format!("{}.align", t.name), i128::from(t.align)),
        ];
        if let Some(id) = t.component {
            consts.push((format!("{}.id", t.name), i128::from(id)));
        }
        for f in &t.fields {
            consts.push((format!("{}.{}", t.name, f.name), i128::from(f.offset)));
        }
        for (name, v) in consts {
            if env.values.contains_key(&name) {
                continue;
            }
            env.value_names.push(name.clone());
            env.api_consts.insert(name.clone());
            env.values.insert(name, ValueInfo { val: Val::Int(v), span: Span::none(), kind: VKind::Const });
        }
    }
}

/// The diagnostic for `import ns.name(...)` whose declaration differs from the index.
fn import_mismatch(env: &Env, imp: &ImportDecl, f: &efevm::api::HostFn, declared: &FuncType) -> Diagnostic {
    env.err(
        imp.qualified.span,
        format!(
            "signature mismatch for `{}`: declared {declared}, but the API index has {}{} which lowers to {}",
            imp.qualified.name,
            imp.qualified.name,
            f.signature_text(),
            f.func_type()
        ),
    )
}

fn prim_of(name: &str) -> Option<Prim> {
    Prim::ALL.iter().copied().find(|p| p.name() == name)
}

/// The struct of the index called `name`, as a section-13 copy (with its owner).
fn index_struct(api: Option<&ApiIndex>, name: &str) -> Option<StructDesc> {
    let api = api?;
    let mut d = api.types.iter().find(|t| t.name == name)?.clone();
    if d.owner.is_empty() {
        d.owner = api.game.clone();
    }
    Some(d)
}

/// The state kept between the phases.
struct State<'a> {
    env: Env<'a>,
    funcs: Vec<&'a FuncDecl>,
    globals: Vec<&'a GlobalDecl>,
    datas: Vec<&'a DataDecl>,
    /// Data items whose symbol this file defines: (name, position among the data items).
    data_syms: Vec<(String, usize)>,
    table: Option<&'a TableDecl>,
    /// `alias -> unit index`.
    aliases: Vec<(String, usize)>,
    /// The consts that depend on the layout (computed in `resolve`).
    late: Vec<&'a ast::ConstDecl>,
}

/// One `.efa` file of a program, as a [`link::Unit`]. The driver ([`crate::asm::assemble_program`])
/// parses every file of the use graph, builds one `Unit` per file and links them.
pub struct Unit<'a> {
    file: &'a str,
    namespace: String,
    index: usize,
    src: &'a str,
    lines: Vec<&'a str>,
    module: &'a ast::Module,
    uses: Vec<Option<usize>>,
    api: Option<&'a ApiIndex>,
    state: Option<State<'a>>,
}

impl<'a> Unit<'a> {
    /// The unit for `module` (parsed from `src`), named `file` in diagnostics and the lines
    /// section, with namespace `namespace` ("" for the root). `index` is its position in the
    /// program (0 is the root); `uses` holds the unit index of every `use` item of the file
    /// in source order (`None` when the driver could not resolve it and reported why).
    pub fn new(
        file: &'a str,
        namespace: &str,
        index: usize,
        src: &'a str,
        module: &'a ast::Module,
        uses: Vec<Option<usize>>,
        api: Option<&'a ApiIndex>,
    ) -> Unit<'a> {
        Unit {
            file,
            namespace: namespace.to_string(),
            index,
            src,
            lines: split_lines(src),
            module,
            uses,
            api,
            state: None,
        }
    }

    fn declare_all(&self, prior: &link::Declared<'_>, diags: &mut Vec<Diagnostic>) -> (State<'a>, link::Decls) {
        let m: &'a ast::Module = self.module;
        let mut env = Env::new(self.file, self.src);
        let mut out = link::Decls::default();
        let api = self.api;
        if let Some(api) = api {
            define_api_consts(&mut env, api);
        }

        let mut consts = Vec::new();
        let mut imports: Vec<&ImportDecl> = Vec::new();
        let mut funcs: Vec<&'a FuncDecl> = Vec::new();
        let mut globals: Vec<&'a GlobalDecl> = Vec::new();
        let mut tables: Vec<&'a TableDecl> = Vec::new();
        let mut datas: Vec<&'a DataDecl> = Vec::new();
        let mut mems = Vec::new();
        let mut uses = Vec::new();
        let mut provides = Vec::new();
        let mut structs = Vec::new();
        let mut scratches = Vec::new();
        let mut event_imports: Vec<&EventImportDecl> = Vec::new();
        let mut provide_events: Vec<&ProvideEventDecl> = Vec::new();
        for item in &m.items {
            match item {
                Item::Import(x) => imports.push(x),
                Item::Memory(x) => mems.push(x),
                Item::Const(x) => consts.push(x),
                Item::Global(x) => globals.push(x),
                Item::Table(x) => tables.push(x),
                Item::Data(x) => datas.push(x),
                Item::Func(x) => funcs.push(x),
                Item::Use(x) => uses.push(x),
                Item::Provide(x) => provides.push(x),
                Item::Struct(x) => structs.push(x),
                Item::Scratch(x) => scratches.push(x),
                Item::EventImport(x) => event_imports.push(x),
                Item::ProvideEvent(x) => provide_events.push(x),
            }
        }

        // The values of the used files that are known before the layout (`alias.NAME`).
        let mut early_aliases: Vec<(String, usize)> = Vec::new();
        for (k, u) in uses.iter().enumerate() {
            let alias = match &u.alias {
                Some(a) => a.name.clone(),
                None => String::from_utf8_lossy(&u.path)
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(".efa")
                    .to_string(),
            };
            if let Some(Some(unit)) = self.uses.get(k) {
                if !early_aliases.iter().any(|(a, _)| *a == alias) {
                    early_aliases.push((alias, *unit));
                }
            }
        }
        for (alias, unit) in &early_aliases {
            let Some(Some(decls)) = prior.decls.get(*unit) else { continue };
            let file = prior.files.get(*unit).cloned().unwrap_or_default();
            for v in link::static_values(decls) {
                let key = format!("{alias}.{}", v.name);
                if !v.public {
                    env.private.entry((Space::Value, key)).or_insert((file.clone(), v.span));
                    continue;
                }
                if env.values.contains_key(&key) {
                    continue;
                }
                let (val, kind) = match v.value {
                    Some(link::ConstValue::Int(n)) => (Val::Int(n), VKind::Const),
                    Some(link::ConstValue::Float(t)) => (Val::Float(t), VKind::Const),
                    None => (Val::Int(0), VKind::Data),
                };
                env.value_names.push(key.clone());
                env.values.insert(key, ValueInfo { val, span: v.span, kind });
            }
        }

        // Event ids (`ns.name.id`) are known only after linking; until then the names exist as
        // placeholders, so a use in a layout position gets a precise error.
        for ev in &event_imports {
            if let Some((ns, name)) = ev.split() {
                if efevm::api::types::is_namespace(ns) && efevm::api::types::is_ident(name) {
                    let key = format!("{}.id", ev.qualified.name);
                    if !env.values.contains_key(&key) {
                        env.value_names.push(key.clone());
                        env.values.insert(key, ValueInfo { val: Val::Int(0), span: ev.qualified.span, kind: VKind::Event });
                    }
                }
            }
        }

        // Consts, structs and data sizes can read each other in any order: `fold` evaluates
        // each on demand (efa-syntax 18), the passes below only record the results in source
        // order.
        let mut fold = Fold::new(&consts, &structs, &datas);
        let mut late: Vec<&'a ast::ConstDecl> = Vec::new();

        // Constants.
        for (k, c) in consts.iter().enumerate() {
            env.check_name(&c.name, "constant", diags);
            match fold.const_value(&env, diags, k) {
                Ok(val) => {
                    let value = match &val {
                        Val::Int(v) => link::ConstValue::Int(*v),
                        Val::Float(t) => link::ConstValue::Float(t.clone()),
                    };
                    if env.define_value(&c.name.name, c.name.span, val, VKind::Const, diags) {
                        out.consts.push(link::ConstDecl {
                            name: c.name.name.clone(),
                            value,
                            public: c.public,
                            span: c.name.span,
                        });
                    }
                }
                Err(Fail::Late(l)) => {
                    if c.public {
                        diags.push(env.err(
                            c.name.span,
                            format!("`pub const {}` must be known when the file is assembled", c.name.name),
                        ).with_note(l.message().replace("; it cannot be used here", "")));
                    } else if env.define_value(&c.name.name, c.name.span, Val::Int(0), VKind::LateConst, diags) {
                        late.push(c);
                    }
                }
                Err(Fail::Reported) => {}
            }
        }

        // Structs (16.7): C layout, constants `S.size`, `S.align`, `S.<field>`.
        for (k, s) in structs.iter().enumerate() {
            if !env.check_name(&s.name, "struct", diags) {
                continue;
            }
            let name = &s.name.name;
            if api.is_some_and(|a| a.types.iter().any(|t| &t.name == name)) {
                diags.push(env.err(
                    s.name.span,
                    format!("struct `{name}` has the same name as a struct of the API index"),
                ));
                continue;
            }
            if let Some(prev) = env.structs.get(name).copied().or_else(|| env.values.get(name).map(|v| v.span)) {
                let d = env.err(s.name.span, format!("duplicate definition of `{name}`"));
                diags.push(env.first_defined(d, prev));
                continue;
            }
            let (desc, registered) = fold.struct_layout(&env, diags, k);
            if !registered {
                continue;
            }
            env.structs.insert(name.clone(), s.name.span);
            let Some(desc) = desc else { continue };
            for (cname, v) in link::struct_consts(&desc) {
                if !env.values.contains_key(&cname) {
                    env.value_names.push(cname.clone());
                    env.values.insert(cname, ValueInfo { val: Val::Int(v), span: s.name.span, kind: VKind::Const });
                }
            }
            out.structs.push(link::StructDecl {
                desc,
                public: s.public || s.exported,
                export: s.exported,
                span: s.name.span,
            });
        }

        // Imports, then functions: one index space.
        let mut import_spaces: Vec<String> = Vec::new();
        for (k, imp) in imports.iter().enumerate() {
            let (ns, name) = match imp.split() {
                Some((a, b)) if !a.is_empty() && !b.is_empty() => (a.to_string(), b.to_string()),
                _ => {
                    diags.push(env.err(
                        imp.qualified.span,
                        format!(
                            "import `{}` needs a namespace: write `namespace.name`",
                            imp.qualified.name
                        ),
                    ));
                    (String::new(), imp.qualified.name.clone())
                }
            };
            let declared = func_type(&imp.params, &imp.results);
            // With an index: the function must exist and match; a missing signature comes from it.
            let mut ty = declared.clone();
            if let Some(api) = api.filter(|_| !ns.is_empty()) {
                match api.function(&ns, &name) {
                    None => {
                        let q = &imp.qualified.name;
                        let mut d = env.err(imp.qualified.span, format!("unknown host function `{q}`"));
                        if let Some(s) = did_you_mean(q, api.qualified_names()) {
                            d = d.with_note(format!("did you mean `{s}`?"));
                        }
                        diags.push(d);
                    }
                    Some(f) if !imp.has_sig => ty = f.func_type(),
                    Some(f) if f.func_type() != declared => {
                        diags.push(import_mismatch(&env, imp, f, &declared));
                    }
                    Some(_) => {}
                }
            } else if !imp.has_sig {
                diags.push(env.err(
                    imp.qualified.span,
                    format!(
                        "import `{}` has no signature: write the parameter list, or assemble against an API index (`--api FILE.efapi`)",
                        imp.qualified.name
                    ),
                ));
            }
            let sig = Sig {
                index: idx(k),
                params: ty.params.iter().map(|t| ty_of_val(*t)).collect(),
                results: ty.results.iter().map(|t| ty_of_val(*t)).collect(),
                span: imp.qualified.span,
                slot: Slot::Import(k),
            };
            env.define_func(&imp.qualified.name, sig, diags);
            if let Some(first) = ns.split('.').next().filter(|s| !s.is_empty()) {
                import_spaces.push(first.to_string());
            }
            out.imports.push(link::ImportDecl { namespace: ns, name, ty, span: imp.qualified.span });
        }
        // The namespaces of the events count like imported ones for alias conflicts (17.1).
        for ev in &event_imports {
            if let Some((ns, _)) = ev.split() {
                if let Some(first) = ns.split('.').next().filter(|s| !s.is_empty()) {
                    import_spaces.push(first.to_string());
                }
            }
        }
        let nimports = imports.len();
        for (k, f) in funcs.iter().enumerate() {
            env.check_name(&f.name, "function", diags);
            let sig = Sig {
                index: idx(nimports + k),
                params: f.params.iter().map(|p| p.ty.ty).collect(),
                results: f.results.iter().map(|t| t.ty).collect(),
                span: f.name.span,
                slot: Slot::Func(k),
            };
            env.define_func(&f.name.name, sig, diags);
        }

        // Data sizes (addresses come from the linker).
        let mut data_syms = Vec::new();
        for (k, d) in datas.iter().enumerate() {
            env.check_name(&d.name, "data", diags);
            let len = fold.data_len(&env, diags, k);
            let span = d.name.span;
            if env.define_value(&d.name.name, span, Val::Int(0), VKind::Data, diags) {
                data_syms.push((d.name.name.clone(), k));
            }
            let len_name = format!("{}.len", d.name.name);
            if env.api_consts.contains(&len_name) {
                diags.push(env.err(
                    span,
                    format!(
                        "data symbol `{0}` collides with the API index: `{0}.len` is already the offset of a field of struct `{0}`",
                        d.name.name
                    ),
                ));
            } else if !env.values.contains_key(&len_name) {
                env.value_names.push(len_name.clone());
                env.values.insert(
                    len_name,
                    ValueInfo { val: Val::Int(i128::from(len)), span, kind: VKind::DataLen },
                );
            }
            out.data.push(link::DataDecl {
                name: d.name.name.clone(),
                size: len,
                align: 8,
                public: d.public,
                span,
            });
        }

        // Globals (initial values come in `resolve`).
        for (k, g) in globals.iter().enumerate() {
            env.check_name(&g.name, "global", diags);
            let first = if let Some(prev) = env.globals.get(&g.name.name) {
                let d = env.err(g.name.span, format!("duplicate definition of global `{}`", g.name.name));
                diags.push(env.first_defined(d, prev.span));
                false
            } else {
                env.global_names.push(g.name.name.clone());
                env.globals.insert(
                    g.name.name.clone(),
                    GlobalInfo { index: idx(k), ty: g.ty.ty, span: g.name.span, local: Some(k) },
                );
                true
            };
            out.globals.push(link::GlobalDecl {
                name: g.name.name.clone(),
                ty: val_type(g.ty.ty),
                mutable: g.mutable,
                public: g.public || g.exported,
                export: (g.exported && first).then(|| g.name.name.clone()),
                span: g.name.span,
            });
        }

        // Table (entries come in `resolve`).
        for (n, t) in tables.iter().enumerate() {
            if n > 0 {
                let d = env.err(t.span, "duplicate `table` declaration");
                let first = tables.first().map(|f| f.span).unwrap_or_default();
                diags.push(env.first_defined(d, first));
            }
        }
        out.table = tables.first().map(|t| t.span);

        // Memory (the size check against the data is the linker's).
        for (n, mem) in mems.iter().enumerate() {
            if n > 0 {
                let d = env.err(mem.span, "duplicate `memory` declaration");
                let first = mems.first().map(|f| f.span).unwrap_or_default();
                diags.push(env.first_defined(d, first));
                continue;
            }
            let mut to_u32 = |op: &Operand, diags: &mut Vec<Diagnostic>| -> Option<u32> {
                let v = fold.int_now(&env, diags, op)?;
                match u32::try_from(v) {
                    Ok(v) => Some(v),
                    Err(_) => {
                        diags.push(env.err(
                            op.span(),
                            format!("memory size {v} must be a non-negative number that fits in u32"),
                        ));
                        None
                    }
                }
            };
            let Some(min) = to_u32(&mem.min, diags) else {
                out.memory = Some(link::MemoryDecl { min: None, max: None, span: mem.span });
                continue;
            };
            let max = match &mem.max {
                Some(op) => match to_u32(op, diags) {
                    Some(v) if v < min => {
                        diags.push(env.err(op.span(), format!("memory maximum {v} is below the minimum {min}")));
                        None
                    }
                    other => other,
                },
                None => None,
            };
            out.memory = Some(link::MemoryDecl { min: Some(min), max, span: mem.span });
        }

        // Scratch (16.8).
        for (n, s) in scratches.iter().enumerate() {
            if n > 0 {
                let d = env.err(s.span, "duplicate `scratch` declaration");
                let first = scratches.first().map(|f| f.span).unwrap_or_default();
                diags.push(env.first_defined(d, first));
                continue;
            }
            match fold.int_now(&env, diags, &s.size) {
                Some(v) if (0..=i128::from(MAX_SCRATCH)).contains(&v) => {
                    out.scratch = Some(link::ScratchDecl { size: v as u32, span: s.span });
                }
                Some(v) => diags.push(env.err(
                    s.size.span(),
                    format!("scratch size {v} is out of range (0 ..= {MAX_SCRATCH} bytes)"),
                )),
                None => {}
            }
        }

        // `use` aliases (16.1, 16.3).
        let mut aliases: Vec<(String, usize)> = Vec::new();
        let mut alias_spans: HashMap<String, Span> = HashMap::new();
        for (k, u) in uses.iter().enumerate() {
            let alias = match &u.alias {
                Some(a) => {
                    if !env.check_name(a, "alias", diags) {
                        continue;
                    }
                    a.clone()
                }
                None => {
                    let path = String::from_utf8_lossy(&u.path);
                    let stem = path.rsplit('/').next().unwrap_or("").trim_end_matches(".efa").to_string();
                    let id = Ident { name: stem.clone(), span: u.path_span };
                    if !efevm::api::types::is_ident(&stem) {
                        diags.push(
                            env.err(u.path_span, format!("the file name `{stem}` is not an identifier, so it cannot be the alias"))
                                .with_note("give the file an alias: `use \"...\" as name`"),
                        );
                        continue;
                    }
                    if !env.check_name(&id, "alias", diags) {
                        continue;
                    }
                    id
                }
            };
            if let Some(prev) = alias_spans.get(&alias.name) {
                let d = env.err(alias.span, format!("duplicate alias `{}`", alias.name));
                diags.push(env.first_defined(d, *prev));
                continue;
            }
            alias_spans.insert(alias.name.clone(), alias.span);
            let conflict = if env.funcs.contains_key(&alias.name) {
                Some("a function")
            } else if env.globals.contains_key(&alias.name) {
                Some("a global")
            } else if env.structs.contains_key(&alias.name)
                || api.is_some_and(|a| a.types.iter().any(|t| t.name == alias.name))
            {
                Some("a struct")
            } else if env.values.get(&alias.name).is_some_and(|v| matches!(v.kind, VKind::Const | VKind::LateConst)) {
                Some("a constant")
            } else if env.values.contains_key(&alias.name) {
                Some("a data item")
            } else if import_spaces.contains(&alias.name) {
                Some("the first segment of an imported namespace")
            } else {
                None
            };
            if let Some(what) = conflict {
                diags.push(env.err(
                    alias.span,
                    format!("alias `{}` conflicts with {what} of the same name in this file", alias.name),
                ).with_note("choose another alias with `as`, so that every dotted name has one meaning"));
                continue;
            }
            if let Some(Some(unit)) = self.uses.get(k) {
                aliases.push((alias.name.clone(), *unit));
            }
        }
        let alias_unit = |name: &str| aliases.iter().find(|(a, _)| a == name).map(|(_, u)| *u);

        // Provides (16.6).
        for p in &provides {
            env.check_name(&p.name, "provide", diags);
            let doc = String::from_utf8_lossy(&p.doc).into_owned();
            if doc.trim().is_empty() {
                diags.push(env.err(p.doc_span, format!("the doc of provide `{}` must not be blank", p.name.name)));
            }
            let ptype = |q: &ast::AParam, diags: &mut Vec<Diagnostic>| -> Option<link::ProvideParam> {
                env.check_name(&q.name, "parameter", diags);
                let ty = match &q.ty {
                    AType::Word(w) => match w.name.as_str() {
                        "i32" => ProvideType::Api(ApiType::I32),
                        "i64" => ProvideType::Api(ApiType::I64),
                        "f32" => ProvideType::Api(ApiType::F32),
                        "f64" => ProvideType::Api(ApiType::F64),
                        "bool" => ProvideType::Api(ApiType::Bool),
                        "entity" => ProvideType::Api(ApiType::Entity),
                        "str" => ProvideType::Api(ApiType::Str),
                        "bytes" => ProvideType::Api(ApiType::Bytes),
                        "ptr" => ProvideType::Api(ApiType::Ptr),
                        other => {
                            diags.push(
                                env.err(w.span, format!("unknown provide type `{other}`"))
                                    .with_note("use i32 i64 f32 f64 bool entity str bytes or &Struct"),
                            );
                            return None;
                        }
                    },
                    AType::Ref(s, span) => match s.name.split_once('.') {
                        Some((a, rest)) => match alias_unit(a) {
                            Some(u) => ProvideType::Struct(SymRef { unit: u, name: rest.to_string(), span: *span }),
                            None => {
                                diags.push(env.err(*span, format!("unknown struct `{}`", s.name)));
                                return None;
                            }
                        },
                        None if env.structs.contains_key(&s.name) => {
                            ProvideType::Struct(SymRef { unit: self.index, name: s.name.clone(), span: *span })
                        }
                        None => match index_struct(api, &s.name) {
                            Some(d) => ProvideType::IndexStruct(d),
                            None => {
                                diags.push(env.err(*span, format!("unknown struct `{}`", s.name)));
                                return None;
                            }
                        },
                    },
                };
                Some(link::ProvideParam { name: q.name.name.clone(), ty, span: q.ty.span() })
            };
            let mut ok = true;
            let mut params = Vec::new();
            for q in &p.params {
                match ptype(q, diags) {
                    Some(x) => params.push(x),
                    None => ok = false,
                }
            }
            let mut results = Vec::new();
            for q in &p.results {
                match ptype(q, diags) {
                    Some(x) => results.push(x),
                    None => ok = false,
                }
            }
            let target = match &p.target {
                None => Some(SymRef { unit: self.index, name: p.name.name.clone(), span: p.name.span }),
                Some(t) => match t.name.split_once('.') {
                    None => Some(SymRef { unit: self.index, name: t.name.clone(), span: t.span }),
                    Some((a, rest)) => match alias_unit(a) {
                        Some(u) => Some(SymRef { unit: u, name: rest.to_string(), span: t.span }),
                        None => {
                            diags.push(
                                env.err(t.span, format!("unknown function `{}`", t.name))
                                    .with_note("a provide's function is a function of this file or `alias.f` of a used file"),
                            );
                            None
                        }
                    },
                },
            };
            if let (true, Some(target)) = (ok, target) {
                out.provides.push(link::ProvideDecl {
                    name: p.name.name.clone(),
                    params,
                    results,
                    doc,
                    target,
                    span: p.name.span,
                });
            }
        }

        // Events (efa-syntax 17): `import event` and `provide event`.
        let struct_ty = |env: &Env, s: &Ident, span: Span, diags: &mut Vec<Diagnostic>| -> Option<ProvideType> {
            match s.name.split_once('.') {
                Some((a, rest)) => match alias_unit(a) {
                    Some(u) => Some(ProvideType::Struct(SymRef { unit: u, name: rest.to_string(), span })),
                    None => {
                        diags.push(env.err(span, format!("unknown struct `{}`", s.name)));
                        None
                    }
                },
                None if env.structs.contains_key(&s.name) => {
                    Some(ProvideType::Struct(SymRef { unit: self.index, name: s.name.clone(), span }))
                }
                None => match index_struct(api, &s.name) {
                    Some(d) => Some(ProvideType::IndexStruct(d)),
                    None => {
                        diags.push(env.err(span, format!("unknown struct `{}`", s.name)));
                        None
                    }
                },
            }
        };
        let payload_name = |t: &Option<ProvideType>| -> Option<String> {
            match t {
                Some(ProvideType::Struct(r)) => Some(r.name.clone()),
                Some(ProvideType::IndexStruct(d)) => Some(d.name.clone()),
                _ => None,
            }
        };
        let show_payload = |n: &Option<String>| n.as_ref().map_or("no payload".to_string(), |n| format!("`&{n}`"));
        let mut event_seen: HashMap<String, Span> = HashMap::new();
        for ev in &event_imports {
            let Some((ns, name)) = ev.split().filter(|(a, b)| !a.is_empty() && !b.is_empty()) else {
                continue; // the parser reported the missing namespace
            };
            let (ns, name) = (ns.to_string(), name.to_string());
            let full = format!("{ns}.{name}");
            if !efevm::api::types::is_namespace(&ns) || !efevm::api::types::is_ident(&name) {
                diags.push(env.err(
                    ev.qualified.span,
                    format!("`{full}` is not a valid event name: `namespace.name` with an identifier as the name"),
                ));
                continue;
            }
            if let Some(first) = event_seen.get(&full) {
                let d = env.err(ev.qualified.span, format!("duplicate import of event `{full}`"));
                diags.push(env.first_defined(d, *first));
                continue;
            }
            event_seen.insert(full.clone(), ev.qualified.span);
            let explicit: Option<Option<ProvideType>> = match &ev.form {
                EventForm::FromIndex => None,
                EventForm::NoPayload => Some(None),
                EventForm::Struct(s, span) => match struct_ty(&env, s, *span, diags) {
                    Some(t) => Some(Some(t)),
                    None => continue,
                },
            };
            let hint = api.and_then(|a| {
                did_you_mean(&full, a.events.iter().map(|e| format!("{}.{}", e.namespace, e.name)))
            });
            let (payload, unindexed) = match (api, api.and_then(|a| a.event(&ns, &name)), explicit) {
                (Some(_), Some(e), exp) => {
                    let from_index = match &e.payload {
                        None => None,
                        Some(n) => match index_struct(api, n) {
                            Some(d) => Some(ProvideType::IndexStruct(d)),
                            None => {
                                diags.push(env.err(
                                    ev.qualified.span,
                                    format!("the API index lists event `{full}` with payload `{n}`, which it does not define"),
                                ));
                                continue;
                            }
                        },
                    };
                    if let Some(declared) = exp {
                        let (a, b) = (payload_name(&declared), payload_name(&from_index));
                        if a != b {
                            let span = match &ev.form {
                                EventForm::Struct(_, s) => *s,
                                _ => ev.qualified.span,
                            };
                            diags.push(env.err(
                                span,
                                format!(
                                    "payload mismatch for event `{full}`: declared {}, but the API index has {}",
                                    show_payload(&a),
                                    show_payload(&b)
                                ),
                            ));
                            continue;
                        }
                    }
                    (from_index, false)
                }
                (Some(_), None, None) => {
                    let mut d = env.err(ev.qualified.span, format!("unknown event `{full}`"));
                    if let Some(h) = hint {
                        d = d.with_note(format!("did you mean `{h}`?"));
                    }
                    diags.push(d);
                    continue;
                }
                (Some(_), None, Some(p)) => (p, true),
                (None, _, None) => {
                    diags.push(env.err(
                        ev.qualified.span,
                        format!(
                            "event `{full}` has no payload: write `import event {full}(&Struct)` or `import event {full}()`, or assemble against an API index (`--api FILE.efapi`)"
                        ),
                    ));
                    continue;
                }
                (None, _, Some(p)) => (p, false),
            };
            out.event_imports.push(link::EventImportDecl {
                namespace: ns,
                name,
                payload,
                unindexed,
                hint: if unindexed { hint } else { None },
                span: ev.qualified.span,
            });
        }
        for pe in &provide_events {
            env.check_name(&pe.name, "event", diags);
            let doc = String::from_utf8_lossy(&pe.doc).into_owned();
            if doc.trim().is_empty() {
                diags.push(env.err(pe.doc_span, format!("the doc of provided event `{}` must not be blank", pe.name.name)));
            }
            let payload = match &pe.payload {
                None => None,
                Some((s, span)) => match struct_ty(&env, s, *span, diags) {
                    Some(t) => Some(t),
                    None => continue,
                },
            };
            out.provided_events.push(link::ProvideEventDecl {
                name: pe.name.name.clone(),
                payload,
                doc,
                span: pe.name.span,
            });
        }

        // Functions, with their exports (only the first definition of a name is exported).
        for f in &funcs {
            let first = env.funcs.get(&f.name.name).is_some_and(|s| s.span == f.name.span);
            let params: Vec<Ty> = f.params.iter().map(|p| p.ty.ty).collect();
            let results: Vec<Ty> = f.results.iter().map(|t| t.ty).collect();
            out.funcs.push(link::FuncDecl {
                name: f.name.name.clone(),
                ty: func_type_of(&params, &results),
                public: f.public || f.exported,
                export: (f.exported && first).then(|| f.name.name.clone()),
                span: f.name.span,
            });
        }

        let state = State { env, funcs, globals, datas, data_syms, table: tables.first().copied(), aliases, late };
        (state, out)
    }
}

impl<'a> link::Unit for Unit<'a> {
    fn file(&self) -> &str {
        self.file
    }

    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn deps(&self) -> Vec<usize> {
        self.uses.iter().flatten().copied().collect()
    }

    fn declare(&mut self, prior: &link::Declared<'_>, diags: &mut Vec<Diagnostic>) -> link::Decls {
        let (state, decls) = self.declare_all(prior, diags);
        self.state = Some(state);
        decls
    }

    fn resolve(&mut self, cx: &ResolveCx<'_>, diags: &mut Vec<Diagnostic>) -> Resolved {
        let Some(st) = self.state.as_mut() else { return Resolved::default() };
        let Some(us) = cx.syms.unit(cx.unit) else { return Resolved::default() };
        let env = &mut st.env;

        // The final indices and addresses.
        for sig in env.funcs.values_mut() {
            let real = match sig.slot {
                Slot::Import(k) => us.imports.get(k),
                Slot::Func(k) => us.funcs.get(k),
                Slot::Ext => None,
            };
            if let Some(i) = real {
                sig.index = *i;
            }
        }
        for g in env.globals.values_mut() {
            if let Some(i) = g.local.and_then(|k| us.globals.get(k)) {
                g.index = *i;
            }
        }
        for (name, k) in &st.data_syms {
            if let (Some(v), Some(addr)) = (env.values.get_mut(name), us.data.get(*k)) {
                v.val = Val::Int(i128::from(*addr));
            }
        }

        // The ids of the events (section 14 indices), `ns.name.id`.
        for (ev, id) in us.decls.event_imports.iter().zip(&us.events) {
            if *id != u32::MAX {
                let key = format!("{}.{}.id", ev.namespace, ev.name);
                if let Some(v) = env.values.get_mut(&key) {
                    v.val = Val::Int(i128::from(*id));
                }
            }
        }

        // `alias.item` for the items of the used files.
        for (alias, unit) in &st.aliases {
            let file = cx.syms.unit(*unit).map(|u| u.file.clone()).unwrap_or_default();
            for e in cx.syms.symbols(*unit) {
                let key = format!("{alias}.{}", e.name);
                if !e.public {
                    env.private.entry((e.space, key)).or_insert((file.clone(), e.span));
                    continue;
                }
                match e.symbol {
                    Symbol::Func { index, ty } => {
                        if !env.funcs.contains_key(&key) {
                            env.func_names.push(key.clone());
                            env.funcs.insert(
                                key,
                                Sig {
                                    index,
                                    params: ty.params.iter().map(|t| ty_of_val(*t)).collect(),
                                    results: ty.results.iter().map(|t| ty_of_val(*t)).collect(),
                                    span: e.span,
                                    slot: Slot::Ext,
                                },
                            );
                        }
                    }
                    Symbol::Global { index, ty, .. } => {
                        if !env.globals.contains_key(&key) {
                            env.global_names.push(key.clone());
                            env.globals.insert(key, GlobalInfo { index, ty: ty_of_val(ty), span: e.span, local: None });
                        }
                    }
                    Symbol::Value(v) => {
                        let val = match v {
                            link::ConstValue::Int(n) => Val::Int(n),
                            link::ConstValue::Float(t) => Val::Float(t),
                        };
                        match env.values.get_mut(&key) {
                            // A data address known since the declare phase as a placeholder.
                            Some(known) if known.kind == VKind::Data => known.val = val,
                            Some(_) => {}
                            None => {
                                env.value_names.push(key.clone());
                                env.values.insert(key, ValueInfo { val, span: e.span, kind: VKind::Const });
                            }
                        }
                    }
                    Symbol::Struct(_) => {}
                }
            }
        }

        // The consts that depend on the layout.
        if !st.late.is_empty() {
            let mut fold = Fold::after_layout(&st.late);
            let mut results = Vec::new();
            for (k, c) in st.late.iter().enumerate() {
                if let Ok(v) = fold.const_value(env, diags, k) {
                    results.push((c.name.name.clone(), v));
                }
            }
            for (name, v) in results {
                if let Some(slot) = env.values.get_mut(&name) {
                    if slot.kind == VKind::LateConst {
                        slot.val = v;
                    }
                }
            }
        }

        let env = &st.env;
        let mut r = Resolved::default();
        for g in &st.globals {
            let k = match global_init(env, g) {
                Ok(k) => k,
                Err(e) => {
                    diags.push(e);
                    Konst::zero(g.ty.ty)
                }
            };
            r.globals.push(konst_bits(k));
        }
        if let Some(t) = st.table {
            let mut entries = Vec::new();
            for e in &t.entries {
                match e {
                    TableEntry::Null(_) => entries.push(None),
                    TableEntry::Func(id) => match env.funcs.get(&id.name) {
                        Some(s) => entries.push(Some(s.index)),
                        None => {
                            diags.push(env.unknown_function(id));
                            entries.push(None);
                        }
                    },
                }
            }
            r.table = Some(entries);
        }
        for d in &st.datas {
            if d.kind == DataKind::Resb {
                r.data.push(Vec::new());
            } else {
                r.data.push(data_bytes(env, d, diags));
            }
        }
        r
    }

    fn define(&mut self, cx: &mut DefineCx<'_>, diags: &mut Vec<Diagnostic>) {
        let Some(st) = self.state.as_ref() else { return };
        let indices: Vec<u32> = cx.syms.unit(cx.unit).map(|u| u.funcs.clone()).unwrap_or_default();
        for (k, f) in st.funcs.iter().enumerate() {
            let out = func::lower_func(&st.env, cx.builder, cx.file_id, f, diags);
            let index = indices.get(k).copied().unwrap_or(u32::MAX);
            cx.define(index, out.body, out.spans, f.name.span);
            for (r, name) in &out.reg_names {
                cx.reg_name(index, *r, name);
            }
        }
    }

    fn diagnostic(&self, span: Span, message: String) -> Diagnostic {
        let text = if span.line == 0 {
            ""
        } else {
            self.lines.get((span.line as usize).saturating_sub(1)).copied().unwrap_or("")
        };
        Diagnostic::with_line(self.file, text, span, message)
    }
}
