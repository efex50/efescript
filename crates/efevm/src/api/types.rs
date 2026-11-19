//! Type descriptions of the API index (crates/efevm/API_INDEX.md, crates/efescript/docs/e3-design.md).
//!
//! These are plain data. The registry (`registry.rs`) stores them next to the host functions,
//! the index (`index.rs`) is a snapshot of them, and the engine's type registry builds
//! [`StructDesc`]s for its components, so there is one set of descriptions for all three.

use crate::types::{FuncType, ValType};
use std::fmt;

/// The type of one parameter or result of a host function, as scripts see it.
///
/// Every `ApiType` lowers to one or two VM value types ([`ApiType::lower`]); the lowered
/// form is the import signature a module must declare.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApiType {
    I32,
    I64,
    F32,
    F64,
    /// `i32`, 0 = false, anything else = true.
    Bool,
    /// An entity handle (`hecs::Entity::to_bits`), `i64`. A stale handle fails cleanly.
    Entity,
    /// An untyped guest address, `i64`.
    Ptr,
    /// A UTF-8 string in guest memory: `(ptr: i64, len: i64)`. Parameters only.
    Str,
    /// A byte buffer in guest memory: `(ptr: i64, len: i64)`. Parameters only.
    Bytes,
    /// A pointer (`i64`) to a struct of the registered layout with this name.
    Struct(String),
}

impl ApiType {
    /// The VM value types this type occupies, in order.
    pub fn lower(&self) -> &'static [ValType] {
        match self {
            ApiType::I32 | ApiType::Bool => &[ValType::I32],
            ApiType::I64 | ApiType::Entity | ApiType::Ptr | ApiType::Struct(_) => &[ValType::I64],
            ApiType::F32 => &[ValType::F32],
            ApiType::F64 => &[ValType::F64],
            ApiType::Str | ApiType::Bytes => &[ValType::I64, ValType::I64],
        }
    }

    /// `true` if this type may be a result (one value slot, not a guest-memory view).
    pub fn is_scalar(&self) -> bool {
        !matches!(self, ApiType::Str | ApiType::Bytes | ApiType::Struct(_))
    }

    /// The index-file spelling: `i32 i64 f32 f64 bool entity ptr str bytes`, or `&Name`.
    pub fn spelling(&self) -> String {
        match self {
            ApiType::I32 => "i32".into(),
            ApiType::I64 => "i64".into(),
            ApiType::F32 => "f32".into(),
            ApiType::F64 => "f64".into(),
            ApiType::Bool => "bool".into(),
            ApiType::Entity => "entity".into(),
            ApiType::Ptr => "ptr".into(),
            ApiType::Str => "str".into(),
            ApiType::Bytes => "bytes".into(),
            ApiType::Struct(n) => format!("&{n}"),
        }
    }

    /// Inverse of [`ApiType::spelling`].
    pub fn parse(s: &str) -> Option<ApiType> {
        Some(match s {
            "i32" => ApiType::I32,
            "i64" => ApiType::I64,
            "f32" => ApiType::F32,
            "f64" => ApiType::F64,
            "bool" => ApiType::Bool,
            "entity" => ApiType::Entity,
            "ptr" => ApiType::Ptr,
            "str" => ApiType::Str,
            "bytes" => ApiType::Bytes,
            _ => {
                let n = s.strip_prefix('&')?;
                if !is_ident(n) {
                    return None;
                }
                ApiType::Struct(n.to_string())
            }
        })
    }

    /// The scalar `ApiType` for a VM value type.
    pub fn from_val(v: ValType) -> ApiType {
        match v {
            ValType::I32 => ApiType::I32,
            ValType::I64 => ApiType::I64,
            ValType::F32 => ApiType::F32,
            ValType::F64 => ApiType::F64,
        }
    }
}

impl fmt::Display for ApiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spelling())
    }
}

/// A named parameter or result.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Param {
    pub name: String,
    pub ty: ApiType,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: ApiType) -> Self {
        Param { name: name.into(), ty }
    }
}

/// The description of one host function: everything the index records about it.
///
/// Build it with [`HostFn::new`] and the chained setters, then hand it to
/// `HostRegistry::register` together with the function itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostFn {
    /// Dot-separated namespace, e.g. `engine.world`. Must be defined in the registry first.
    pub namespace: String,
    /// A plain identifier, e.g. `spawn`.
    pub name: String,
    /// One line at least; the registry rejects an empty doc.
    pub doc: String,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    /// Filled by the registry from the namespace's owner.
    pub owner: String,
}

impl HostFn {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        HostFn {
            namespace: namespace.into(),
            name: name.into(),
            doc: String::new(),
            params: Vec::new(),
            results: Vec::new(),
            owner: String::new(),
        }
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn param(mut self, name: impl Into<String>, ty: ApiType) -> Self {
        self.params.push(Param::new(name, ty));
        self
    }

    pub fn result(mut self, name: impl Into<String>, ty: ApiType) -> Self {
        self.results.push(Param::new(name, ty));
        self
    }

    /// `namespace.name`.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    /// The VM signature a module must import this function with.
    pub fn func_type(&self) -> FuncType {
        FuncType::new(
            self.params.iter().flat_map(|p| p.ty.lower().iter().copied()),
            self.results.iter().flat_map(|p| p.ty.lower().iter().copied()),
        )
    }

    /// For each parameter, the index of its first value slot in the lowered argument list.
    pub fn param_slots(&self) -> Vec<usize> {
        let mut at = 0;
        self.params
            .iter()
            .map(|p| {
                let s = at;
                at += p.ty.lower().len();
                s
            })
            .collect()
    }

    /// `(text: str, n: i64) -> (ok: i32)` style signature for diagnostics and hover text.
    pub fn signature_text(&self) -> String {
        let list = |ps: &[Param]| ps.iter().map(|p| format!("{}: {}", p.name, p.ty)).collect::<Vec<_>>().join(", ");
        let mut s = format!("({})", list(&self.params));
        match self.results.len() {
            0 => {}
            _ => s.push_str(&format!(" -> ({})", list(&self.results))),
        }
        s
    }
}

/// A script event: a name in a namespace with an optional struct payload (API_INDEX.md
/// "events"). Registering it with `HostRegistry::register_event` puts it in the index; a
/// module refers to it in section 14 `events` and the reference is checked at link time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventDesc {
    /// Dot-separated namespace, e.g. `game`. Must be defined in the registry first.
    pub namespace: String,
    /// A plain identifier, e.g. `score_changed`.
    pub name: String,
    /// The name of a registered struct whose bytes are the payload; `None` = no payload.
    pub payload: Option<String>,
    /// One line at least; the registry rejects an empty doc.
    pub doc: String,
    /// Filled by the registry from the namespace's owner.
    pub owner: String,
}

impl EventDesc {
    /// An event without payload, doc or owner; set them with the chained setters.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        EventDesc { namespace: namespace.into(), name: name.into(), payload: None, doc: String::new(), owner: String::new() }
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    /// Sets the payload struct.
    pub fn payload(mut self, name: impl Into<String>) -> Self {
        self.payload = Some(name.into());
        self
    }

    /// `namespace.name`.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

/// A namespace of host functions with a version and an owner (`std`, `engine`, or a mod name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamespaceInfo {
    pub name: String,
    /// Bumped by the owner on an incompatible change; modules record the version they used.
    pub version: u32,
    pub owner: String,
    pub doc: String,
}

/// The primitive type of a struct field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Prim {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    /// One byte, 0 or 1.
    Bool,
    /// An entity handle stored as `u64`.
    Entity,
}

impl Prim {
    pub const ALL: [Prim; 12] = [
        Prim::U8,
        Prim::I8,
        Prim::U16,
        Prim::I16,
        Prim::U32,
        Prim::I32,
        Prim::U64,
        Prim::I64,
        Prim::F32,
        Prim::F64,
        Prim::Bool,
        Prim::Entity,
    ];

    /// Size in bytes; also the required alignment.
    pub fn size(self) -> u32 {
        match self {
            Prim::U8 | Prim::I8 | Prim::Bool => 1,
            Prim::U16 | Prim::I16 => 2,
            Prim::U32 | Prim::I32 | Prim::F32 => 4,
            Prim::U64 | Prim::I64 | Prim::F64 | Prim::Entity => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Prim::U8 => "u8",
            Prim::I8 => "i8",
            Prim::U16 => "u16",
            Prim::I16 => "i16",
            Prim::U32 => "u32",
            Prim::I32 => "i32",
            Prim::U64 => "u64",
            Prim::I64 => "i64",
            Prim::F32 => "f32",
            Prim::F64 => "f64",
            Prim::Bool => "bool",
            Prim::Entity => "entity",
        }
    }

    pub fn parse(s: &str) -> Option<Prim> {
        Prim::ALL.into_iter().find(|p| p.name() == s)
    }
}

impl fmt::Display for Prim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One field of a struct layout: `count` consecutive values of `ty` at `offset`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldDesc {
    pub name: String,
    pub offset: u32,
    pub ty: Prim,
    /// 1 for a scalar, N for `[ty; N]` (a `Vec2` is `f32` × 2).
    pub count: u32,
    pub doc: String,
}

impl FieldDesc {
    pub fn new(name: impl Into<String>, offset: u32, ty: Prim, count: u32) -> Self {
        FieldDesc { name: name.into(), offset, ty, count, doc: String::new() }
    }

    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    /// `ty.size() * count`.
    pub fn size(&self) -> u32 {
        self.ty.size().saturating_mul(self.count)
    }
}

/// The byte layout of a `#[repr(C)]` struct or component that crosses the script boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StructDesc {
    /// Stable name, a plain identifier (`Transform`). Unique in a registry.
    pub name: String,
    pub size: u32,
    /// A power of two; `size` is a multiple of it.
    pub align: u32,
    pub fields: Vec<FieldDesc>,
    pub doc: String,
    /// `std`, `engine`, a game name or a mod name; filled by whoever registers it.
    pub owner: String,
    /// `Some(id)` for an engine component (the numeric id scripts pass to `engine.world.*`),
    /// `None` for a plain struct used only in signatures.
    pub component: Option<u32>,
    /// A read-only component: scripts may read it but `engine.world.set/insert/remove` fail
    /// (status 5). Written to the `.efapi` only when `true`; not part of the module binary.
    pub readonly: bool,
}

impl StructDesc {
    pub fn new(name: impl Into<String>, size: u32, align: u32) -> Self {
        StructDesc {
            name: name.into(),
            size,
            align,
            fields: Vec::new(),
            doc: String::new(),
            owner: String::new(),
            component: None,
            readonly: false,
        }
    }

    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn field(mut self, f: FieldDesc) -> Self {
        self.fields.push(f);
        self
    }

    pub fn doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = doc.into();
        self
    }

    pub fn field_named(&self, name: &str) -> Option<&FieldDesc> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Checks the layout rules of API_INDEX.md section "types": identifier names, unique
    /// field names, `align` a power of two dividing `size`, every field inside `size`,
    /// aligned to its primitive size, `count >= 1`, and no two fields overlapping.
    pub fn validate(&self) -> Result<(), String> {
        if !is_type_name(&self.name) {
            return Err(format!("struct name `{}` is not an identifier", self.name));
        }
        if self.align == 0 || !self.align.is_power_of_two() {
            return Err(format!("struct `{}`: alignment {} is not a power of two", self.name, self.align));
        }
        if self.size % self.align != 0 {
            return Err(format!("struct `{}`: size {} is not a multiple of alignment {}", self.name, self.size, self.align));
        }
        let mut spans: Vec<(u32, u32, &str)> = Vec::new();
        for (i, f) in self.fields.iter().enumerate() {
            if !is_ident(&f.name) {
                return Err(format!("struct `{}`: field name `{}` is not an identifier", self.name, f.name));
            }
            if self.fields[..i].iter().any(|g| g.name == f.name) {
                return Err(format!("struct `{}`: duplicate field `{}`", self.name, f.name));
            }
            if f.count == 0 {
                return Err(format!("struct `{}`: field `{}` has count 0", self.name, f.name));
            }
            if f.offset % f.ty.size() != 0 {
                return Err(format!("struct `{}`: field `{}` at offset {} is not aligned to {}", self.name, f.name, f.offset, f.ty.size()));
            }
            if f.ty.size() > self.align {
                return Err(format!("struct `{}`: field `{}` needs alignment {} but the struct has {}", self.name, f.name, f.ty.size(), self.align));
            }
            let end = match (f.ty.size() as u64).checked_mul(f.count as u64).map(|s| s + f.offset as u64) {
                Some(e) if e <= self.size as u64 => e as u32,
                _ => return Err(format!("struct `{}`: field `{}` ends past size {}", self.name, f.name, self.size)),
            };
            if let Some((_, _, other)) = spans.iter().find(|(s, e, _)| f.offset < *e && *s < end) {
                return Err(format!("struct `{}`: field `{}` overlaps field `{}`", self.name, f.name, other));
            }
            spans.push((f.offset, end, &f.name));
        }
        Ok(())
    }
}

/// `true` for `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    match c.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    c.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// `true` for a struct name: an identifier, or the form of a mod-defined component,
/// `mod:<mod>/<Name>` (both parts identifiers). The second form cannot be used in a `&Name`
/// parameter; only the engine registers it.
pub fn is_type_name(s: &str) -> bool {
    match s.strip_prefix("mod:").and_then(|r| r.split_once('/')) {
        Some((m, n)) => is_ident(m) && is_ident(n),
        None => is_ident(s),
    }
}

/// `true` for one or more identifiers joined by single dots (`std`, `engine.world`).
pub fn is_namespace(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_ident)
}

/// 64-bit FNV-1a, the content hash of an index (API_INDEX.md section "hash").
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 32-bit FNV-1a; the engine derives component ids from component names with it.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowering_and_spelling() {
        let f = HostFn::new("std", "println").doc("x").param("text", ApiType::Str).result("ok", ApiType::Bool);
        assert_eq!(f.func_type(), FuncType::new([ValType::I64, ValType::I64], [ValType::I32]));
        assert_eq!(f.param_slots(), vec![0]);
        assert_eq!(f.signature_text(), "(text: str) -> (ok: bool)");
        for t in [ApiType::I32, ApiType::Entity, ApiType::Str, ApiType::Struct("Transform".into())] {
            assert_eq!(ApiType::parse(&t.spelling()), Some(t));
        }
        assert_eq!(ApiType::parse("&1x"), None);
        for p in Prim::ALL {
            assert_eq!(Prim::parse(p.name()), Some(p));
        }
    }

    #[test]
    fn struct_validation() {
        let ok = StructDesc::new("T", 12, 4)
            .field(FieldDesc::new("pos", 0, Prim::F32, 2))
            .field(FieldDesc::new("rot", 8, Prim::F32, 1));
        assert!(ok.validate().is_ok());
        let overlap = StructDesc::new("T", 8, 4).field(FieldDesc::new("a", 0, Prim::F32, 2)).field(FieldDesc::new("b", 4, Prim::U32, 1));
        assert!(overlap.validate().unwrap_err().contains("overlaps"));
        assert!(StructDesc::new("T", 6, 4).validate().is_err());
        assert!(StructDesc::new("T", 4, 4).field(FieldDesc::new("a", 2, Prim::U32, 1)).validate().is_err());
        assert!(StructDesc::new("T", 4, 4).field(FieldDesc::new("a", 0, Prim::U32, 2)).validate().is_err());
        assert!(is_namespace("engine.world") && !is_namespace("engine..world") && !is_namespace(""));
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a32(b"a"), 0xe40c292c);
    }
}
