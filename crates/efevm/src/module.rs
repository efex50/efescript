//! In-memory module form: `ModuleData` (unverified) and `Module` (verified, cheap to clone).

use crate::api::{Param, StructDesc};
use crate::error::LoadError;
use crate::op::Op;
use crate::types::{FuncType, ValType};
use std::sync::Arc;

/// A fixed-size decoded instruction (16 bytes).
///
/// Field mapping per `Shape` (unused fields are 0):
/// - None: all 0
/// - D: a=d
/// - DA: a=d, b=a
/// - DAB: a=d, b=a, c=b
/// - ConstI32: a=d, b=imm bits (i32 as u32); ConstF32: a=d, b=f32 bits
/// - ConstI64 / ConstF64: a=d, b=low 32 bits, c=high 32 bits
/// - Select: a=d, b=cond, c = ra | (rb << 16)
/// - Load: a=d, b=base, c=offset; Store: a=src, b=base, c=offset
/// - ABC: a, b, c = the three registers
/// - GlobalGet: a=d, b=global index; GlobalSet: a=global index, b=source register
/// - Jmp: a=L; Br: a=register, b=L
/// - BrTable: a=register, b=index into the function's `br_tables`
/// - Call: a=function index, b=base; CallIndirect: a=type index, b=register, c=base
/// - Ret: a=base
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inst {
    pub op: Op,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

const _: () = assert!(std::mem::size_of::<Inst>() == 16);

impl Inst {
    pub const fn new(op: Op, a: u32, b: u32, c: u32) -> Inst {
        Inst { op, a, b, c }
    }

    pub const fn const_i32(d: u32, v: i32) -> Inst {
        Inst::new(Op::ConstI32, d, v as u32, 0)
    }

    pub const fn const_f32(d: u32, v: f32) -> Inst {
        Inst::new(Op::ConstF32, d, v.to_bits(), 0)
    }

    pub const fn const_i64(d: u32, v: i64) -> Inst {
        let u = v as u64;
        Inst::new(Op::ConstI64, d, u as u32, (u >> 32) as u32)
    }

    pub const fn const_f64(d: u32, v: f64) -> Inst {
        let u = v.to_bits();
        Inst::new(Op::ConstF64, d, u as u32, (u >> 32) as u32)
    }

    /// The 64-bit constant of a ConstI64 / ConstF64 instruction.
    pub const fn const_i64_value(&self) -> u64 {
        (self.b as u64) | ((self.c as u64) << 32)
    }

    pub const fn select(d: u32, cond: u32, ra: u32, rb: u32) -> Inst {
        Inst::new(Op::Select, d, cond, ra | (rb << 16))
    }

    /// `(ra, rb)` of a Select instruction.
    pub const fn select_regs(&self) -> (u32, u32) {
        (self.c & 0xFFFF, self.c >> 16)
    }
}

/// `br_table` targets; `default` is the label used when the index is out of range.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BrTable {
    pub targets: Vec<u32>,
    pub default: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Function {
    pub type_idx: u32,
    pub nregs: u16,
    pub code: Vec<Inst>,
    pub br_tables: Vec<BrTable>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub namespace: String,
    pub name: String,
    pub type_idx: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MemoryType {
    pub min_pages: u32,
    pub max_pages: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Global {
    pub ty: ValType,
    pub mutable: bool,
    /// Raw bits as the register would hold them.
    pub init: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Func(u32),
    Global(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DataSegment {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Names {
    /// (function index, name)
    pub funcs: Vec<(u32, String)>,
    /// (function index, register, name)
    pub regs: Vec<(u32, u32, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LineEntry {
    pub inst: u32,
    pub file: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FuncLines {
    pub func: u32,
    pub entries: Vec<LineEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lines {
    pub files: Vec<String>,
    pub funcs: Vec<FuncLines>,
}

/// Section 12 `api`: the API index a module was assembled against (SPEC.md 9.1,
/// API_INDEX.md "How modules use the hash").
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApiRef {
    /// `ApiIndex::hash()` of the index.
    pub hash: u64,
    /// `(namespace, version)` for every namespace the module imports from; names are unique.
    pub namespaces: Vec<(String, u32)>,
}

/// One provide of section 13: a name in the mod's namespace bound to one of the module's own
/// functions, with the typed and named signature and the doc of the API index (SPEC.md 9.2).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Provide {
    /// A plain identifier, unique among the provides.
    pub name: String,
    /// Function index in the whole index space (imports first); must be a defined function.
    pub func: u32,
    /// Not blank.
    pub doc: String,
    pub params: Vec<Param>,
    /// Scalars only.
    pub results: Vec<Param>,
}

/// One event a mod declares in its own namespace (section 13, SPEC.md 9.2): registered as
/// `<mod id>.name` when the mod loads, so it is in the index like every other event.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProvidedEvent {
    /// A plain identifier, unique among the provided events and not a provide's name.
    pub name: String,
    /// The name of a struct listed in the interface's `structs`; `None` = no payload.
    pub payload: Option<String>,
    /// Not blank.
    pub doc: String,
}

/// Section 13 `interface`: what a module provides when it is loaded as a mod (SPEC.md 9.2).
///
/// More fields may be added; construct it with `..Default::default()`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Interface {
    /// `(offset, size)` of the scratch region in linear memory that `str`, `bytes` and struct
    /// arguments are copied into (e8-design.md section 7).
    pub scratch: Option<(u64, u32)>,
    /// Own structs (empty `owner`) and copies of structs defined elsewhere (non-empty `owner`).
    pub structs: Vec<StructDesc>,
    pub provides: Vec<Provide>,
    /// Events the mod declares in its own namespace.
    pub events: Vec<ProvidedEvent>,
}

/// One entry of section 14 `events` (SPEC.md 9.3): a script event the module sends or reads.
/// The event's id inside the module is its index in `ModuleData::events`; the engine's
/// `engine.events.*` functions take that id. Resolved against the registry at link time.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EventRef {
    /// Dot-separated namespace of the event (`game`, `engine.input`, a mod id).
    pub namespace: String,
    /// A plain identifier.
    pub name: String,
    /// `(struct name, size in bytes)` of the payload as the module was built against it;
    /// `None` = the event has no payload. Must equal the registered payload at link time.
    pub payload: Option<(String, u32)>,
}

impl EventRef {
    /// `namespace.name`.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    /// The payload size in bytes (0 without payload): how many bytes `engine.events.send`
    /// copies from guest memory and `engine.events.read` copies to it.
    pub fn payload_size(&self) -> u32 {
        self.payload.as_ref().map_or(0, |(_, n)| *n)
    }
}

/// Unverified module contents.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ModuleData {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    pub functions: Vec<Function>,
    pub table: Vec<Option<u32>>,
    pub memory: MemoryType,
    pub globals: Vec<Global>,
    pub exports: Vec<Export>,
    pub data: Vec<DataSegment>,
    pub names: Names,
    pub lines: Lines,
    /// `None` for modules assembled without an index (no section 12).
    pub api: Option<ApiRef>,
    /// `None` for modules that provide nothing (no section 13).
    pub interface: Option<Interface>,
    /// Section 14: the script events the module refers to; event id = index.
    pub events: Vec<EventRef>,
}

impl ModuleData {
    /// Imports plus module functions.
    pub fn num_funcs(&self) -> u32 {
        (self.imports.len() + self.functions.len()) as u32
    }

    /// The distinct namespaces of the events of section 14, sorted. The engine checks each
    /// against the module's namespace allow-list (the same prefix rule as imports) before it
    /// instantiates the module.
    pub fn event_namespaces(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.events.iter().map(|e| e.namespace.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Type of a function in the whole index space (imports first).
    pub fn func_type(&self, idx: u32) -> Option<&FuncType> {
        let i = idx as usize;
        let type_idx = if i < self.imports.len() {
            self.imports.get(i)?.type_idx
        } else {
            self.functions.get(i - self.imports.len())?.type_idx
        };
        self.types.get(type_idx as usize)
    }
}

/// A verified module; cloning is cheap.
#[derive(Clone, Debug)]
pub struct Module {
    data: Arc<ModuleData>,
}

impl Module {
    /// Verifies and wraps.
    pub fn new(data: ModuleData) -> Result<Module, LoadError> {
        crate::verify::verify(&data)?;
        Ok(Module { data: Arc::new(data) })
    }

    /// Decodes `.efvm` bytes, then verifies.
    pub fn decode(bytes: &[u8]) -> Result<Module, LoadError> {
        Module::new(crate::binary::decode(bytes)?)
    }

    pub fn encode(&self) -> Vec<u8> {
        crate::binary::encode(&self.data)
    }

    pub fn data(&self) -> &ModuleData {
        &self.data
    }

    pub fn func_type(&self, func_index: u32) -> Option<&FuncType> {
        self.data.func_type(func_index)
    }

    pub fn export_func(&self, name: &str) -> Option<u32> {
        self.data.exports.iter().find_map(|e| match e.kind {
            ExportKind::Func(i) if e.name == name => Some(i),
            _ => None,
        })
    }

    pub fn export_global(&self, name: &str) -> Option<u32> {
        self.data.exports.iter().find_map(|e| match e.kind {
            ExportKind::Global(i) if e.name == name => Some(i),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inst_layout() {
        assert_eq!(std::mem::size_of::<Inst>(), 16);
        let i = Inst::const_i64(3, -2);
        assert_eq!(i.const_i64_value(), (-2i64) as u64);
        let f = Inst::const_f64(1, 1.5);
        assert_eq!(f2(f.const_i64_value()), 1.5);
        assert_eq!(Inst::select(0, 1, 2, 3).select_regs(), (2, 3));
    }

    fn f2(b: u64) -> f64 {
        f64::from_bits(b)
    }

    #[test]
    fn func_type_lookup() {
        let m = ModuleData {
            types: vec![FuncType::new([ValType::I32], [])],
            imports: vec![Import { namespace: "a".into(), name: "b".into(), type_idx: 0 }],
            functions: vec![Function::default()],
            ..Default::default()
        };
        assert_eq!(m.num_funcs(), 2);
        assert!(m.func_type(1).is_some());
        assert!(m.func_type(2).is_none());
    }
}
