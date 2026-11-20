//! Reads the module-level structure of a wasm module and rejects what the translator does not
//! support. Function bodies are kept as unparsed `FunctionBody`s for `func.rs`.

use crate::error::{Loc, TranslateError, unsupported};
use efevm::{FuncType, Global, ValType};
use std::collections::HashMap;
use wasmparser::{
    CompositeInnerType, Encoding, ConstExpr, DataKind, ElementItems, ElementKind, ExternalKind, FunctionBody, KnownCustom, Name,
    Operator, Parser, Payload, TypeRef,
};

/// Largest table the translator accepts (entries).
pub(crate) const MAX_TABLE: u64 = 1 << 20;

pub(crate) struct WImport {
    pub module: String,
    pub name: String,
    pub type_idx: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WExport {
    Func(u32),
    Global(u32),
}

pub(crate) struct Parsed<'a> {
    pub types: Vec<FuncType>,
    /// Type index of every function in the function index space (imports first).
    pub func_types: Vec<u32>,
    pub imports: Vec<WImport>,
    pub bodies: Vec<FunctionBody<'a>>,
    /// Table 0: its size, if the module has a table.
    pub table_size: Option<u32>,
    /// `(min, max)` pages of memory 0.
    pub memory: Option<(u32, Option<u32>)>,
    pub globals: Vec<Global>,
    pub exports: Vec<(String, WExport)>,
    pub start: Option<u32>,
    /// Active element segments: offset and function indices (`None` = null).
    pub elems: Vec<(u32, Vec<Option<u32>>, usize)>,
    /// Active data segments: offset, bytes.
    pub data: Vec<(u64, &'a [u8])>,
    pub api_sections: Vec<&'a [u8]>,
    pub provide_sections: Vec<&'a [u8]>,
    /// Function names from the `name` section.
    pub names: HashMap<u32, String>,
}

pub(crate) fn invalid(offset: usize, message: impl Into<String>) -> TranslateError {
    TranslateError::Invalid { offset, message: message.into() }
}

fn val_type(t: wasmparser::ValType, at: usize, what: &str) -> Result<ValType, TranslateError> {
    match t {
        wasmparser::ValType::I32 => Ok(ValType::I32),
        wasmparser::ValType::I64 => Ok(ValType::I64),
        wasmparser::ValType::F32 => Ok(ValType::F32),
        wasmparser::ValType::F64 => Ok(ValType::F64),
        wasmparser::ValType::V128 => Err(unsupported(format!("SIMD (`v128` in {what})"), Loc::at(at))),
        wasmparser::ValType::Ref(_) => {
            Err(unsupported(format!("reference types beyond one funcref table 0 (`{t}` in {what})"), Loc::at(at)))
        }
    }
}

pub(crate) enum Const {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    RefFunc(u32),
    RefNull,
}

/// Evaluates a constant expression of exactly one constant instruction.
pub(crate) fn const_expr(e: &ConstExpr<'_>, at: usize, what: &str) -> Result<Const, TranslateError> {
    let mut r = e.get_operators_reader();
    let first = r.read().map_err(|x| invalid(x.offset(), x.message()))?;
    let v = match first {
        Operator::I32Const { value } => Const::I32(value),
        Operator::I64Const { value } => Const::I64(value),
        Operator::F32Const { value } => Const::F32(value.bits()),
        Operator::F64Const { value } => Const::F64(value.bits()),
        Operator::RefFunc { function_index } => Const::RefFunc(function_index),
        Operator::RefNull { .. } => Const::RefNull,
        Operator::GlobalGet { .. } => {
            return Err(unsupported(format!("`global.get` in the constant expression of {what} (imported globals)"), Loc::at(at)));
        }
        other => return Err(unsupported(format!("constant expression `{other:?}` in {what} (extended-const)"), Loc::at(at))),
    };
    match r.read() {
        Ok(Operator::End) => Ok(v),
        _ => Err(unsupported(format!("constant expression of more than one instruction in {what} (extended-const)"), Loc::at(at))),
    }
}

const MEMORY_FLAGS_HINT: &str = "the translator supports a plain single-threaded module whose memory is defined by the module; \
    browser-style flags (`-C target-feature=+atomics`, `--shared-memory`, `--import-memory`) make the module import a shared memory, \
    so build the guest with RUSTFLAGS that override them (design doc e7.4, section 5)";

pub(crate) fn parse(wasm: &[u8]) -> Result<Parsed<'_>, TranslateError> {
    let mut p = Parsed {
        types: Vec::new(),
        func_types: Vec::new(),
        imports: Vec::new(),
        bodies: Vec::new(),
        table_size: None,
        memory: None,
        globals: Vec::new(),
        exports: Vec::new(),
        start: None,
        elems: Vec::new(),
        data: Vec::new(),
        api_sections: Vec::new(),
        provide_sections: Vec::new(),
        names: HashMap::new(),
    };
    let e = |x: wasmparser::BinaryReaderError| invalid(x.offset(), x.message());
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(e)? {
            Payload::Version { encoding: Encoding::Component, .. } => {
                return Err(unsupported("WebAssembly components (only core modules)", Loc::at(0)));
            }
            Payload::Version { .. } => {}
            Payload::TypeSection(r) => {
                let at = r.range().start;
                for g in r {
                    let g = g.map_err(e)?;
                    if g.is_explicit_rec_group() || g.types().len() != 1 {
                        return Err(unsupported("GC type groups (`rec`)", Loc::at(at)));
                    }
                    for t in g.into_types() {
                        if !t.is_final || t.supertype_idx.is_some() {
                            return Err(unsupported("GC subtyping (`sub`, non-final types)", Loc::at(at)));
                        }
                        if t.composite_type.shared || t.composite_type.descriptor_idx.is_some() || t.composite_type.describes_idx.is_some() {
                            return Err(unsupported("shared or descriptor types", Loc::at(at)));
                        }
                        match t.composite_type.inner {
                            CompositeInnerType::Func(f) => {
                                let params = f.params().iter().map(|t| val_type(*t, at, "a function type")).collect::<Result<Vec<_>, _>>()?;
                                let results = f.results().iter().map(|t| val_type(*t, at, "a function type")).collect::<Result<Vec<_>, _>>()?;
                                p.types.push(FuncType::new(params, results));
                            }
                            _ => return Err(unsupported("GC types (struct, array or continuation type)", Loc::at(at))),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.map_err(e)?;
                    match i.ty {
                        TypeRef::Func(t) => {
                            p.func_types.push(t);
                            p.imports.push(WImport { module: i.module.to_string(), name: i.name.to_string(), type_idx: t });
                        }
                        TypeRef::FuncExact(_) => {
                            return Err(unsupported(format!("exact function type reference (import `{}::{}`)", i.module, i.name), Loc::default()));
                        }
                        TypeRef::Memory(_) => {
                            return Err(unsupported(
                                format!("imported memory `{}::{}`: {MEMORY_FLAGS_HINT}", i.module, i.name),
                                Loc::default(),
                            ));
                        }
                        TypeRef::Table(_) => return Err(unsupported(format!("imported table `{}::{}`", i.module, i.name), Loc::default())),
                        TypeRef::Global(_) => return Err(unsupported(format!("imported global `{}::{}`", i.module, i.name), Loc::default())),
                        TypeRef::Tag(_) => return Err(unsupported(format!("exceptions (imported tag `{}::{}`)", i.module, i.name), Loc::default())),
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    p.func_types.push(t.map_err(e)?);
                }
            }
            Payload::TableSection(r) => {
                let mut n = 0;
                for t in r {
                    let t = t.map_err(e)?;
                    n += 1;
                    if n > 1 {
                        return Err(unsupported("more than one table (reference types)", Loc::default()));
                    }
                    if t.ty.element_type != wasmparser::RefType::FUNCREF || t.ty.table64 {
                        return Err(unsupported("a table that is not a 32-bit funcref table", Loc::default()));
                    }
                    if !matches!(t.init, wasmparser::TableInit::RefNull) {
                        return Err(unsupported("a table with an initialiser expression (reference types)", Loc::default()));
                    }
                    if t.ty.initial > MAX_TABLE {
                        return Err(unsupported(format!("a table of {} entries (the limit is {MAX_TABLE})", t.ty.initial), Loc::default()));
                    }
                    p.table_size = Some(t.ty.initial as u32);
                }
            }
            Payload::MemorySection(r) => {
                let mut n = 0;
                for m in r {
                    let m = m.map_err(e)?;
                    n += 1;
                    if n > 1 {
                        return Err(unsupported("more than one memory (multi-memory)", Loc::default()));
                    }
                    if m.memory64 {
                        return Err(unsupported("memory64", Loc::default()));
                    }
                    if m.shared {
                        return Err(unsupported(format!("shared memory (atomics): {MEMORY_FLAGS_HINT}"), Loc::default()));
                    }
                    if m.page_size_log2.is_some() {
                        return Err(unsupported("custom page sizes", Loc::default()));
                    }
                    // wasm32 memories never exceed 65536 pages, whatever the embedder's cap.
                    p.memory = Some((m.initial as u32, Some(m.maximum.map_or(65536, |x| x.min(65536) as u32))));
                }
            }
            Payload::TagSection(_) => return Err(unsupported("exceptions (tag section)", Loc::default())),
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g.map_err(e)?;
                    let at = g.init_expr.get_binary_reader().original_position();
                    if g.ty.shared {
                        return Err(unsupported("a shared global", Loc::at(at)));
                    }
                    let ty = val_type(g.ty.content_type, at, "a global")?;
                    let init = match const_expr(&g.init_expr, at, "a global initialiser")? {
                        Const::I32(v) => u64::from(v as u32),
                        Const::I64(v) => v as u64,
                        Const::F32(b) => u64::from(b),
                        Const::F64(b) => b,
                        _ => return Err(unsupported("a reference-typed global", Loc::at(at))),
                    };
                    p.globals.push(Global { ty, mutable: g.ty.mutable, init });
                }
            }
            Payload::ExportSection(r) => {
                for x in r {
                    let x = x.map_err(e)?;
                    match x.kind {
                        ExternalKind::Func => p.exports.push((x.name.to_string(), WExport::Func(x.index))),
                        ExternalKind::Global => p.exports.push((x.name.to_string(), WExport::Global(x.index))),
                        // Not visible to efevm modules: the memory and the table are the module's own.
                        ExternalKind::Memory | ExternalKind::Table => {}
                        ExternalKind::FuncExact => {
                            return Err(unsupported(format!("exact function type reference (export `{}`)", x.name), Loc::default()));
                        }
                        ExternalKind::Tag => return Err(unsupported(format!("exceptions (exported tag `{}`)", x.name), Loc::default())),
                    }
                }
            }
            Payload::StartSection { func, .. } => p.start = Some(func),
            Payload::ElementSection(r) => {
                for el in r {
                    let el = el.map_err(e)?;
                    let at = el.range.start;
                    let ElementKind::Active { table_index, offset_expr } = el.kind else {
                        return Err(unsupported("passive or declarative element segments (reference types)", Loc::at(at)));
                    };
                    if table_index.unwrap_or(0) != 0 {
                        return Err(unsupported("an element segment for a table other than table 0", Loc::at(at)));
                    }
                    let Const::I32(off) = const_expr(&offset_expr, at, "an element segment offset")? else {
                        return Err(unsupported("an element segment offset that is not an i32 constant", Loc::at(at)));
                    };
                    let mut items = Vec::new();
                    match el.items {
                        ElementItems::Functions(fr) => {
                            for f in fr {
                                items.push(Some(f.map_err(e)?));
                            }
                        }
                        ElementItems::Expressions(_, er) => {
                            for x in er {
                                match const_expr(&x.map_err(e)?, at, "an element")? {
                                    Const::RefFunc(f) => items.push(Some(f)),
                                    Const::RefNull => items.push(None),
                                    _ => return Err(unsupported("an element that is not `ref.func` or `ref.null`", Loc::at(at))),
                                }
                            }
                        }
                    }
                    p.elems.push((off as u32, items, at));
                }
            }
            Payload::DataSection(r) => {
                for d in r {
                    let d = d.map_err(e)?;
                    let at = d.range.start;
                    match d.kind {
                        DataKind::Passive => {
                            return Err(unsupported("passive data segments (bulk-memory `memory.init`/`data.drop`)", Loc::at(at)));
                        }
                        DataKind::Active { memory_index, offset_expr } => {
                            if memory_index != 0 {
                                return Err(unsupported("a data segment for a memory other than memory 0 (multi-memory)", Loc::at(at)));
                            }
                            let Const::I32(off) = const_expr(&offset_expr, at, "a data segment offset")? else {
                                return Err(unsupported("a data segment offset that is not an i32 constant", Loc::at(at)));
                            };
                            p.data.push((u64::from(off as u32), d.data));
                        }
                    }
                }
            }
            Payload::CodeSectionEntry(b) => p.bodies.push(b),
            Payload::CustomSection(c) => match c.name() {
                "efe.api" => p.api_sections.push(c.data()),
                "efe.provide" => p.provide_sections.push(c.data()),
                "name" => {
                    if let KnownCustom::Name(reader) = c.as_known() {
                        for n in reader {
                            // A broken name section is not worth failing the translation.
                            let Ok(n) = n else { break };
                            if let Name::Function(map) = n {
                                for naming in map.into_iter().flatten() {
                                    p.names.insert(naming.index, naming.name.to_string());
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            Payload::DataCountSection { .. } | Payload::CodeSectionStart { .. } | Payload::End(_) => {}
            other => {
                return Err(unsupported(format!("module part `{}`", payload_name(&other)), Loc::default()));
            }
        }
    }
    if p.bodies.len() + p.imports.len() != p.func_types.len() {
        return Err(invalid(0, "the function and code sections disagree"));
    }
    Ok(p)
}

fn payload_name(p: &Payload<'_>) -> String {
    let d = format!("{p:?}");
    d.split([' ', '{', '(']).next().unwrap_or("").to_string()
}
