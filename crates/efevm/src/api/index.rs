//! `ApiIndex`: a snapshot of a `HostRegistry` as plain data, and the `.efapi` file
//! (crates/efevm/API_INDEX.md). Writing and reading live in `json.rs`.

use crate::api::types::{ApiType, EventDesc, HostFn, NamespaceInfo, StructDesc, fnv1a64, is_ident, is_namespace};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Everything a script can call or see: namespaces, struct layouts, host functions and events.
///
/// [`HostRegistry::index`](crate::api::HostRegistry::index) produces it sorted (namespaces
/// and types by name, functions and events by `(namespace, name)`), and [`ApiIndex::from_json`] sorts
/// what it reads the same way, so `from_json(&x.to_json()) == x` for every registry index.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApiIndex {
    /// Informational; not part of the hash.
    pub game: String,
    pub namespaces: Vec<NamespaceInfo>,
    pub types: Vec<StructDesc>,
    pub functions: Vec<HostFn>,
    /// Script events (API_INDEX.md "events"), sorted by `(namespace, name)`.
    pub events: Vec<EventDesc>,
}

/// Why an index file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// Not well-formed JSON. `offset` is a byte offset into the text.
    Syntax { offset: usize, message: String },
    /// A required key is missing or has the wrong JSON type (`path` like `functions[3].name`).
    Schema { path: String, message: String },
    /// `format` is not `"efapi"`.
    Format(String),
    /// `version` is not a version this reader supports (only `2`).
    Version(String),
    /// The `hash` key does not match the content.
    Hash { file: String, computed: u64 },
    /// A function's `signature` does not match its `params`/`results`.
    Signature { function: String, file: String, computed: String },
    /// A name, type or layout rule is broken (API_INDEX.md).
    Invalid(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Syntax { offset, message } => write!(f, "JSON syntax error at byte {offset}: {message}"),
            ApiError::Schema { path, message } => write!(f, "{path}: {message}"),
            ApiError::Format(s) => write!(f, "not an API index (format {s:?}, expected \"efapi\")"),
            ApiError::Version(v) => write!(f, "unsupported API index version {v} (this reader supports 2)"),
            ApiError::Hash { file, computed } => {
                write!(f, "hash mismatch: file says {file}, content is fnv1a64:{computed:016x} (edited or truncated?)")
            }
            ApiError::Signature { function, file, computed } => {
                write!(f, "{function}: signature {file:?} does not match the parameters ({computed})")
            }
            ApiError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiIndex {
    /// The `.efapi` text: keys in the documented order, sorted entries, two-space
    /// indentation, `\n` line ends and a final newline.
    pub fn to_json(&self) -> String {
        crate::api::json::write(self, Some(self.hash()))
    }

    /// 64-bit FNV-1a over the canonical body (the file without its `game` and `hash` lines).
    pub fn hash(&self) -> u64 {
        fnv1a64(crate::api::json::write(self, None).as_bytes())
    }

    /// Parses and checks an index file: `format`, `version`, `hash`, every `signature`, and
    /// the rules of [`ApiIndex::validate`]. Unknown keys are ignored. Never panics.
    pub fn from_json(text: &str) -> Result<ApiIndex, ApiError> {
        crate::api::json::read(text)
    }

    /// Sorts namespaces and types by name and functions and events by `(namespace, name)`.
    pub fn sort(&mut self) {
        self.events.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
        self.namespaces.sort_by(|a, b| a.name.cmp(&b.name));
        self.types.sort_by(|a, b| a.name.cmp(&b.name));
        self.functions.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    }

    /// Checks the API_INDEX.md rules: valid and unique namespace, struct, field, function and
    /// parameter names; valid struct layouts with unique component ids; every function in a
    /// listed namespace; `str`/`bytes`/`&S` only as parameters; every `&S` a listed struct;
    /// every event in a listed namespace with an identifier name, unique, not the full name of
    /// a function, and with a listed payload struct.
    pub fn validate(&self) -> Result<(), ApiError> {
        let bad = |m: String| Err(ApiError::Invalid(m));
        let mut seen = HashSet::new();
        for n in &self.namespaces {
            if !is_namespace(&n.name) {
                return bad(format!("namespace `{}` is not a valid name", n.name));
            }
            if !seen.insert(n.name.as_str()) {
                return bad(format!("duplicate namespace `{}`", n.name));
            }
        }
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for t in &self.types {
            t.validate().map_err(ApiError::Invalid)?;
            if !names.insert(t.name.as_str()) {
                return bad(format!("duplicate struct `{}`", t.name));
            }
            if let Some(id) = t.component
                && !ids.insert(id)
            {
                return bad(format!("struct `{}`: duplicate component id {id}", t.name));
            }
        }
        let mut fns = HashSet::new();
        for f in &self.functions {
            let q = f.qualified();
            if !seen.contains(f.namespace.as_str()) {
                return bad(format!("{q}: namespace `{}` is not listed", f.namespace));
            }
            if !is_ident(&f.name) {
                return bad(format!("{q}: function name is not an identifier"));
            }
            if !fns.insert((f.namespace.as_str(), f.name.as_str())) {
                return bad(format!("duplicate function {q}"));
            }
            let all: Vec<_> = f.params.iter().chain(&f.results).collect();
            for (i, p) in all.iter().enumerate() {
                if !is_ident(&p.name) {
                    return bad(format!("{q}: `{}` is not an identifier", p.name));
                }
                if all[..i].iter().any(|o| o.name == p.name) {
                    return bad(format!("{q}: duplicate parameter or result `{}`", p.name));
                }
                if let ApiType::Struct(s) = &p.ty
                    && !names.contains(s.as_str())
                {
                    return bad(format!("{q}: struct `{s}` is not listed"));
                }
            }
            if let Some(r) = f.results.iter().find(|r| !r.ty.is_scalar()) {
                return bad(format!("{q}: result `{}` must be a scalar type", r.name));
            }
        }
        let mut evs = HashSet::new();
        for e in &self.events {
            let q = e.qualified();
            if !seen.contains(e.namespace.as_str()) {
                return bad(format!("event {q}: namespace `{}` is not listed", e.namespace));
            }
            if !is_ident(&e.name) {
                return bad(format!("event {q}: name is not an identifier"));
            }
            if !evs.insert((e.namespace.as_str(), e.name.as_str())) {
                return bad(format!("duplicate event {q}"));
            }
            if fns.contains(&(e.namespace.as_str(), e.name.as_str())) {
                return bad(format!("event {q} has the full name of a function"));
            }
            if let Some(s) = &e.payload
                && !names.contains(s.as_str())
            {
                return bad(format!("event {q}: payload struct `{s}` is not listed"));
            }
        }
        Ok(())
    }

    /// Combines several indices into one. Namespaces are keyed by name, structs by name and
    /// functions and events by `(namespace, name)`. An entry that appears in more than one input with
    /// equal content is kept once; the same key with different content is an error that names
    /// the item and the `game` fields of the two inputs. The merged `game` is the first input's
    /// (an empty slice gives `ApiIndex::default()`). The result is sorted and validated.
    pub fn merge(indices: &[&ApiIndex]) -> Result<ApiIndex, ApiError> {
        let Some(first) = indices.first() else { return Ok(ApiIndex::default()) };
        let mut out = ApiIndex { game: first.game.clone(), ..ApiIndex::default() };
        // Each entry remembers the game of the input that defined it, for the conflict message.
        let mut namespaces: HashMap<&str, (&NamespaceInfo, &str)> = HashMap::new();
        let mut types: HashMap<&str, (&StructDesc, &str)> = HashMap::new();
        let mut functions: HashMap<(&str, &str), (&HostFn, &str)> = HashMap::new();
        let mut events: HashMap<(&str, &str), (&EventDesc, &str)> = HashMap::new();
        let conflict = |what: String, a: &str, b: &str| Err(ApiError::Invalid(format!("{what} differs between `{a}` and `{b}`")));
        for ix in indices {
            let game = ix.game.as_str();
            for n in &ix.namespaces {
                match namespaces.get(n.name.as_str()) {
                    Some(&(prev, _)) if prev == n => {}
                    Some(&(_, prev_game)) => return conflict(format!("namespace `{}`", n.name), prev_game, game),
                    None => {
                        namespaces.insert(n.name.as_str(), (n, game));
                        out.namespaces.push(n.clone());
                    }
                }
            }
            for t in &ix.types {
                match types.get(t.name.as_str()) {
                    Some(&(prev, _)) if prev == t => {}
                    Some(&(_, prev_game)) => return conflict(format!("struct `{}`", t.name), prev_game, game),
                    None => {
                        types.insert(t.name.as_str(), (t, game));
                        out.types.push(t.clone());
                    }
                }
            }
            for f in &ix.functions {
                let key = (f.namespace.as_str(), f.name.as_str());
                match functions.get(&key) {
                    Some(&(prev, _)) if prev == f => {}
                    Some(&(_, prev_game)) => return conflict(format!("function `{}`", f.qualified()), prev_game, game),
                    None => {
                        functions.insert(key, (f, game));
                        out.functions.push(f.clone());
                    }
                }
            }
            for e in &ix.events {
                let key = (e.namespace.as_str(), e.name.as_str());
                match events.get(&key) {
                    Some(&(prev, _)) if prev == e => {}
                    Some(&(_, prev_game)) => return conflict(format!("event `{}`", e.qualified()), prev_game, game),
                    None => {
                        events.insert(key, (e, game));
                        out.events.push(e.clone());
                    }
                }
            }
        }
        out.sort();
        out.validate()?;
        Ok(out)
    }

    pub fn function(&self, namespace: &str, name: &str) -> Option<&HostFn> {
        self.functions.iter().find(|f| f.namespace == namespace && f.name == name)
    }

    /// `namespace.name` of every function, in index order.
    pub fn qualified_names(&self) -> Vec<String> {
        self.functions.iter().map(HostFn::qualified).collect()
    }

    /// The event `namespace.name`.
    pub fn event(&self, namespace: &str, name: &str) -> Option<&EventDesc> {
        self.events.iter().find(|e| e.namespace == namespace && e.name == name)
    }

    pub fn struct_type(&self, name: &str) -> Option<&StructDesc> {
        self.types.iter().find(|t| t.name == name)
    }

    pub fn namespace(&self, name: &str) -> Option<&NamespaceInfo> {
        self.namespaces.iter().find(|n| n.name == name)
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::api::types::{FieldDesc, Prim};

    fn ns(name: &str, version: u32) -> NamespaceInfo {
        NamespaceInfo { name: name.into(), version, owner: "std".into(), doc: "doc".into() }
    }

    fn vec2(size: u32) -> StructDesc {
        StructDesc::new("Vec2", size, 4)
            .field(FieldDesc::new("x", 0, Prim::F32, 1))
            .field(FieldDesc::new("y", 4, Prim::F32, 1))
    }

    fn add(doc: &str) -> HostFn {
        HostFn::new("util", "add").doc(doc).param("a", ApiType::I32).result("r", ApiType::I32)
    }

    fn index(game: &str, namespaces: Vec<NamespaceInfo>, types: Vec<StructDesc>, functions: Vec<HostFn>) -> ApiIndex {
        ApiIndex { game: game.into(), namespaces, types, functions, events: vec![] }
    }

    fn invalid(r: Result<ApiIndex, ApiError>) -> String {
        match r {
            Err(ApiError::Invalid(m)) => m,
            other => panic!("expected ApiError::Invalid, got {other:?}"),
        }
    }

    #[test]
    fn identical_entries_are_deduplicated() {
        let a = index("gameA", vec![ns("util", 1)], vec![vec2(8)], vec![add("adds")]);
        let b = index(
            "gameB",
            vec![ns("util", 1), ns("math", 2)],
            vec![vec2(8)],
            vec![add("adds"), HostFn::new("math", "neg").doc("negates").param("v", ApiType::F32).result("r", ApiType::F32)],
        );
        let m = ApiIndex::merge(&[&a, &b]).expect("merge");
        let names: Vec<&str> = m.namespaces.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["math", "util"]);
        assert_eq!(m.types, vec![vec2(8)]);
        assert_eq!(m.functions.len(), 2);
        assert_eq!(m.functions[0].qualified(), "math.neg");
        assert_eq!(m.functions[1].qualified(), "util.add");
        // Merging an index with itself changes nothing.
        assert_eq!(ApiIndex::merge(&[&a, &a]).expect("self merge"), {
            let mut x = a.clone();
            x.sort();
            x
        });
    }

    #[test]
    fn conflicting_namespace_names_both_games() {
        let a = index("gameA", vec![ns("util", 1)], vec![], vec![]);
        let b = index("gameB", vec![ns("util", 2)], vec![], vec![]);
        let m = invalid(ApiIndex::merge(&[&a, &b]));
        assert_eq!(m, "namespace `util` differs between `gameA` and `gameB`");
    }

    #[test]
    fn conflicting_struct_names_both_games() {
        let a = index("gameA", vec![], vec![vec2(8)], vec![]);
        let b = index("gameB", vec![], vec![vec2(16)], vec![]);
        let m = invalid(ApiIndex::merge(&[&a, &b]));
        assert_eq!(m, "struct `Vec2` differs between `gameA` and `gameB`");
    }

    #[test]
    fn conflicting_function_names_both_games() {
        let a = index("gameA", vec![ns("util", 1)], vec![], vec![add("adds")]);
        let b = index("gameB", vec![ns("util", 1)], vec![], vec![add("sums")]);
        let m = invalid(ApiIndex::merge(&[&a, &b]));
        assert_eq!(m, "function `util.add` differs between `gameA` and `gameB`");
    }

    #[test]
    fn conflict_is_reported_for_a_later_input_against_the_first_definer() {
        let a = index("gameA", vec![ns("util", 1)], vec![], vec![]);
        let b = index("gameB", vec![ns("util", 1)], vec![], vec![]);
        let c = index("gameC", vec![ns("util", 3)], vec![], vec![]);
        let m = invalid(ApiIndex::merge(&[&a, &b, &c]));
        assert_eq!(m, "namespace `util` differs between `gameA` and `gameC`");
    }

    #[test]
    fn game_is_taken_from_the_first_input() {
        let a = index("gameA", vec![ns("util", 1)], vec![], vec![]);
        let b = index("gameB", vec![ns("math", 1)], vec![], vec![]);
        assert_eq!(ApiIndex::merge(&[&a, &b]).expect("merge").game, "gameA");
        assert_eq!(ApiIndex::merge(&[&b, &a]).expect("merge").game, "gameB");
    }

    #[test]
    fn empty_input_gives_default() {
        assert_eq!(ApiIndex::merge(&[]).expect("empty"), ApiIndex::default());
    }

    #[test]
    fn merged_result_is_validated() {
        // A function whose namespace is listed by no input is rejected by validate().
        let a = index("gameA", vec![], vec![], vec![add("adds")]);
        let m = invalid(ApiIndex::merge(&[&a]));
        assert!(m.contains("namespace `util` is not listed"), "{m}");
    }
}
