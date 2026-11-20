//! The language-neutral model the binding generators (`rustgen`, `cgen`) are built on.
//!
//! It is built from a merged [`ApiIndex`] and holds what every backend needs: the namespace
//! tree with its functions and events, the struct layouts with explicit padding, and the
//! lowering of API types to wasm values (the ABI of `docs/e7.4-wasm-design.md`). A backend
//! only decides spelling: names, escapes, how a pointer or a string looks in its language.

use efevm::api::{ApiIndex, ApiType, EventDesc, HostFn, NamespaceInfo, Prim, StructDesc};
use std::collections::BTreeMap;
use std::fmt;

/// Why bindings could not be generated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindError(pub String);

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BindError {}

pub(crate) fn err<T>(m: String) -> Result<T, BindError> {
    Err(BindError(m))
}

/// One node of the namespace tree: `engine.world` is `engine` -> `world`.
#[derive(Default)]
pub struct Node<'a> {
    pub children: BTreeMap<String, Node<'a>>,
    /// Present when the index lists this namespace itself (not just a prefix of one).
    pub info: Option<&'a NamespaceInfo>,
    pub funcs: Vec<&'a HostFn>,
    pub events: Vec<&'a EventDesc>,
}

fn node<'n, 'a>(root: &'n mut Node<'a>, ns: &str) -> &'n mut Node<'a> {
    let mut n = root;
    for seg in ns.split('.') {
        n = n.children.entry(seg.to_string()).or_default();
    }
    n
}

/// Validates `ix` and builds its namespace tree (sorted, so output is deterministic).
pub fn build_tree(ix: &ApiIndex) -> Result<Node<'_>, BindError> {
    ix.validate().map_err(|e| BindError(format!("invalid API index: {e}")))?;
    let mut root = Node::default();
    for n in &ix.namespaces {
        node(&mut root, &n.name).info = Some(n);
    }
    for f in &ix.functions {
        node(&mut root, &f.namespace).funcs.push(f);
    }
    for e in &ix.events {
        node(&mut root, &e.namespace).events.push(e);
    }
    Ok(root)
}

/// A wasm value type of the ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wasm {
    I32,
    I64,
    F32,
    F64,
}

/// The wasm values an API type occupies on wasm32, in order (design doc, section 3): pointers,
/// `&Struct` and both halves of `str`/`bytes` are `i32`, `bool` is `i32`, `entity` is `i64`.
pub fn wasm_lower(t: &ApiType) -> &'static [Wasm] {
    match t {
        ApiType::I32 | ApiType::Bool | ApiType::Ptr | ApiType::Struct(_) => &[Wasm::I32],
        ApiType::I64 | ApiType::Entity => &[Wasm::I64],
        ApiType::F32 => &[Wasm::F32],
        ApiType::F64 => &[Wasm::F64],
        ApiType::Str | ApiType::Bytes => &[Wasm::I32, Wasm::I32],
    }
}

/// The single result of `f`, none, or an error when there are several (extern "C" cannot
/// return several values).
pub fn single_result(f: &HostFn) -> Result<Option<&ApiType>, BindError> {
    match f.results.as_slice() {
        [] => Ok(None),
        [r] => Ok(Some(&r.ty)),
        more => err(format!(
            "{}: {} results; the wasm bindings support at most one (extern \"C\" cannot return several values)",
            f.qualified(),
            more.len()
        )),
    }
}

/// One field of a struct as laid out in memory, including explicit padding.
pub struct LField {
    /// The index name, or `_pad<offset>` for padding.
    pub name: String,
    pub doc: String,
    /// `None` for padding.
    pub ty: Option<Prim>,
    /// Array length; for padding the number of bytes.
    pub count: u32,
    pub offset: u32,
}

impl LField {
    pub fn is_pad(&self) -> bool {
        self.ty.is_none()
    }
}

/// The fields of a struct in offset order with padding where the index has a gap or the struct
/// is larger than its last field, and the alignment a plain C layout would reach.
pub struct Layout {
    pub fields: Vec<LField>,
    pub natural_align: u32,
}

pub fn layout(t: &StructDesc) -> Result<Layout, BindError> {
    let mut fields: Vec<_> = t.fields.iter().collect();
    fields.sort_by_key(|f| f.offset);
    let mut out = Vec::new();
    let mut cur = 0u32;
    let mut natural_align = 1u32;
    let pad = |out: &mut Vec<LField>, at: u32, n: u32| -> Result<(), BindError> {
        let name = format!("_pad{at}");
        if t.fields.iter().any(|f| f.name == name) {
            return err(format!("struct `{}`: field `{name}` collides with the padding field of the same name", t.name));
        }
        out.push(LField { name, doc: String::new(), ty: None, count: n, offset: at });
        Ok(())
    };
    for f in fields {
        if f.offset > cur {
            pad(&mut out, cur, f.offset - cur)?;
        }
        out.push(LField { name: f.name.clone(), doc: f.doc.clone(), ty: Some(f.ty), count: f.count, offset: f.offset });
        cur = f.offset + f.size();
        natural_align = natural_align.max(f.ty.size());
    }
    if t.size > cur {
        pad(&mut out, cur, t.size - cur)?;
    }
    Ok(Layout { fields: out, natural_align })
}
