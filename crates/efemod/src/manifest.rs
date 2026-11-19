//! The package manifest, `manifest.json` (MOD_PACKAGE.md section 3).

use std::collections::HashSet;

use efevm::api::json::{self, Value};
use efevm::api::is_namespace;

use crate::error::PackageError;
use crate::resolve::Dependency;
use crate::version::{Version, VersionReq};

/// What the mod asks the game to grant. These are requests; the game's policy decides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    /// Namespace prefixes (`is_namespace`), unique, at most 64.
    pub namespaces: Vec<String>,
    /// Requested VM units per frame, `1 ..= 10^12`.
    pub budget: Option<u64>,
    /// Requested maximum memory in 64 KiB pages, `1 ..= 65536`.
    pub memory_pages: Option<u32>,
}

/// Largest canonical JSON text of [`Manifest::meta`], in bytes.
pub const MAX_META_BYTES: usize = 64 * 1024;
/// Deepest nesting of [`Manifest::meta`]; the object itself is depth 1.
pub const MAX_META_DEPTH: usize = 16;

/// The parsed and validated `manifest.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub version: Version,
    pub name: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub game: Option<String>,
    pub api_version: u32,
    pub dependencies: Vec<Dependency>,
    pub permissions: Permissions,
    /// Free-form package metadata (`"meta"` in `manifest.json`, `[meta]` in `mod.toml`).
    /// Must be a JSON object within [`MAX_META_BYTES`] and [`MAX_META_DEPTH`].
    pub meta: Option<Value>,
}

const MAX_DEPS: usize = 64;
const MAX_NAMESPACES: usize = 64;
const MAX_AUTHORS: usize = 32;
const MAX_BUDGET: u64 = 1_000_000_000_000;
const MAX_PAGES: u64 = 65536;

fn err<T>(m: impl Into<String>) -> Result<T, PackageError> {
    Err(PackageError::Manifest(m.into()))
}

/// Shows a string value in a message, shortened.
fn show(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= 40 {
            out.push_str("...");
            break;
        }
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    format!("`{out}`")
}

/// `[a-z][a-z0-9_]{0,63}`, not a reserved name.
fn check_id(id: &str, path: &str) -> Result<(), PackageError> {
    let b = id.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
    if !ok {
        return err(format!("{path}: {} is not a mod id (`[a-z][a-z0-9_]{{0,63}}`)", show(id)));
    }
    if matches!(id, "engine" | "std" | "efe") {
        return err(format!("{path}: `{id}` is a reserved name"));
    }
    Ok(())
}

fn check_text(s: &str, path: &str, min: usize, max: usize, controls: bool) -> Result<(), PackageError> {
    let n = s.chars().count();
    if n < min || n > max {
        return err(format!("{path}: length {n} is not in {min}..={max} characters"));
    }
    if controls && s.chars().any(|c| c.is_control()) {
        return err(format!("{path}: contains a control character"));
    }
    Ok(())
}

impl Manifest {
    /// A manifest with the given id and version and everything else at its default.
    pub fn new(id: impl Into<String>, version: Version) -> Self {
        Manifest {
            id: id.into(),
            version,
            name: None,
            description: None,
            authors: Vec::new(),
            license: None,
            game: None,
            api_version: 1,
            dependencies: Vec::new(),
            permissions: Permissions::default(),
            meta: None,
        }
    }

    /// The display name: `name`, or the id.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// The doc of the mod's namespace in the index: the description, or the default doc.
    pub fn namespace_doc(&self) -> String {
        match &self.description {
            Some(d) if !d.trim().is_empty() => d.clone(),
            _ => format!("Functions provided by the mod `{}`.", self.id),
        }
    }

    /// Checks every rule of section 3 that does not need the module.
    pub fn validate(&self) -> Result<(), PackageError> {
        check_id(&self.id, "id")?;
        if let Some(n) = &self.name {
            check_text(n, "name", 1, 128, true)?;
        }
        if let Some(d) = &self.description {
            if d.is_empty() {
                return err("description: must not be empty (omit it instead)");
            }
            check_text(d, "description", 1, 2048, false)?;
        }
        if self.authors.len() > MAX_AUTHORS {
            return err(format!("authors: {} entries (at most {MAX_AUTHORS})", self.authors.len()));
        }
        for (i, a) in self.authors.iter().enumerate() {
            check_text(a, &format!("authors[{i}]"), 1, 128, false)?;
        }
        if let Some(l) = &self.license {
            if l.is_empty() {
                return err("license: must not be empty (omit it instead)");
            }
            check_text(l, "license", 1, 64, false)?;
        }
        if let Some(g) = &self.game {
            check_text(g, "game", 1, 64, false)?;
        }
        if self.api_version == 0 {
            return err("api_version: must be at least 1");
        }
        if self.dependencies.len() > MAX_DEPS {
            return err(format!("dependencies: {} entries (at most {MAX_DEPS})", self.dependencies.len()));
        }
        let mut seen = HashSet::new();
        for (i, d) in self.dependencies.iter().enumerate() {
            check_id(&d.id, &format!("dependencies[{i}].id"))?;
            if d.id == self.id {
                return err(format!("dependencies[{i}].id: `{}` is the mod itself", d.id));
            }
            if !seen.insert(d.id.as_str()) {
                return err(format!("dependencies[{i}].id: `{}` is listed twice", d.id));
            }
        }
        if let Some(meta) = &self.meta {
            check_meta(meta)?;
        }
        let p = &self.permissions;
        if p.namespaces.len() > MAX_NAMESPACES {
            return err(format!(
                "permissions.namespaces: {} entries (at most {MAX_NAMESPACES})",
                p.namespaces.len()
            ));
        }
        let mut seen = HashSet::new();
        for (i, n) in p.namespaces.iter().enumerate() {
            if !is_namespace(n) {
                return err(format!("permissions.namespaces[{i}]: {} is not a namespace", show(n)));
            }
            if !seen.insert(n.as_str()) {
                return err(format!("permissions.namespaces[{i}]: `{n}` is listed twice"));
            }
        }
        if let Some(b) = p.budget {
            if !(1..=MAX_BUDGET).contains(&b) {
                return err(format!("permissions.budget: {b} is not in 1..={MAX_BUDGET}"));
            }
        }
        if let Some(m) = p.memory_pages {
            if !(1..=MAX_PAGES as u32).contains(&m) {
                return err(format!("permissions.memory_pages: {m} is not in 1..={MAX_PAGES}"));
            }
        }
        Ok(())
    }

    /// Parses and validates a `manifest.json`. Unknown keys are ignored.
    pub fn from_json(text: &str) -> Result<Self, PackageError> {
        let root = json::parse(text).map_err(|e| PackageError::Manifest(format!("not valid JSON: {e}")))?;
        if root.entries().is_none() {
            return err("the manifest must be a JSON object");
        }
        match root.get("format") {
            None => return err("format: required"),
            Some(v) => match v.as_str() {
                Some("efmod") => {}
                Some(other) => return err(format!("format: {} is not `efmod`", show(other))),
                None => return err("format: must be a string"),
            },
        }
        match root.get("format_version") {
            None => return err("format_version: required"),
            Some(v) => match v.as_u64() {
                Some(1) => {}
                Some(n) => return err(format!("format_version: {n} is not supported (only 1)")),
                None => return err("format_version: must be an integer"),
            },
        }
        let id = req_str(&root, "id")?;
        let vs = req_str(&root, "version")?;
        let version: Version = vs
            .parse()
            .map_err(|e| PackageError::Manifest(format!("version: {} is not a version ({e})", show(&vs))))?;
        let mut m = Manifest::new(id, version);
        m.name = opt_str(&root, "name")?;
        m.description = opt_str(&root, "description")?.filter(|s| !s.is_empty());
        m.license = opt_str(&root, "license")?.filter(|s| !s.is_empty());
        m.game = opt_str(&root, "game")?;
        if let Some(v) = root.get("authors") {
            let a = v.as_array().ok_or_else(|| PackageError::Manifest("authors: must be an array".into()))?;
            for (i, x) in a.iter().enumerate() {
                match x.as_str() {
                    Some(s) => m.authors.push(s.to_string()),
                    None => return err(format!("authors[{i}]: must be a string")),
                }
            }
        }
        if let Some(v) = root.get("api_version") {
            match v.as_u64() {
                Some(n) if (1..=u32::MAX as u64).contains(&n) => m.api_version = n as u32,
                Some(n) => return err(format!("api_version: {n} is not in 1..={}", u32::MAX)),
                None => return err("api_version: must be an integer"),
            }
        }
        if let Some(v) = root.get("dependencies") {
            let a = v.as_array().ok_or_else(|| PackageError::Manifest("dependencies: must be an array".into()))?;
            for (i, d) in a.iter().enumerate() {
                if d.entries().is_none() {
                    return err(format!("dependencies[{i}]: must be an object"));
                }
                let did = match d.get("id") {
                    None => return err(format!("dependencies[{i}].id: required")),
                    Some(x) => x
                        .as_str()
                        .ok_or_else(|| PackageError::Manifest(format!("dependencies[{i}].id: must be a string")))?,
                };
                let rs = match d.get("req") {
                    None => return err(format!("dependencies[{i}].req: required")),
                    Some(x) => x
                        .as_str()
                        .ok_or_else(|| PackageError::Manifest(format!("dependencies[{i}].req: must be a string")))?,
                };
                let req: VersionReq = rs.parse().map_err(|_| {
                    PackageError::Manifest(format!("dependencies[{i}].req: {} is not a range", show(rs)))
                })?;
                m.dependencies.push(Dependency { id: did.to_string(), req });
            }
        }
        if let Some(v) = root.get("permissions") {
            if v.entries().is_none() {
                return err("permissions: must be an object");
            }
            if let Some(n) = v.get("namespaces") {
                let a = n
                    .as_array()
                    .ok_or_else(|| PackageError::Manifest("permissions.namespaces: must be an array".into()))?;
                for (i, x) in a.iter().enumerate() {
                    match x.as_str() {
                        Some(s) => m.permissions.namespaces.push(s.to_string()),
                        None => return err(format!("permissions.namespaces[{i}]: must be a string")),
                    }
                }
            }
            if let Some(b) = v.get("budget") {
                m.permissions.budget = Some(
                    b.as_u64().ok_or_else(|| PackageError::Manifest("permissions.budget: must be an integer".into()))?,
                );
            }
            if let Some(b) = v.get("memory_pages") {
                let n = b
                    .as_u64()
                    .ok_or_else(|| PackageError::Manifest("permissions.memory_pages: must be an integer".into()))?;
                if n > u32::MAX as u64 {
                    return err(format!("permissions.memory_pages: {n} is not in 1..={MAX_PAGES}"));
                }
                m.permissions.memory_pages = Some(n as u32);
            }
        }
        m.meta = root.get("meta").cloned();
        m.validate()?;
        Ok(m)
    }

    /// The canonical `manifest.json` text of section 3.
    pub fn to_json(&self) -> String {
        let mut fields: Vec<String> = Vec::new();
        let mut kv = |k: &str, v: String| fields.push(format!("  {}: {v}", quote(k)));
        kv("format", quote("efmod"));
        kv("format_version", "1".into());
        kv("id", quote(&self.id));
        kv("version", quote(&self.version.to_string()));
        if let Some(n) = &self.name {
            kv("name", quote(n));
        }
        if let Some(d) = self.description.as_ref().filter(|s| !s.is_empty()) {
            kv("description", quote(d));
        }
        if !self.authors.is_empty() {
            let a: Vec<String> = self.authors.iter().map(|s| quote(s)).collect();
            kv("authors", format!("[{}]", a.join(", ")));
        }
        if let Some(l) = self.license.as_ref().filter(|s| !s.is_empty()) {
            kv("license", quote(l));
        }
        if let Some(g) = &self.game {
            kv("game", quote(g));
        }
        kv("api_version", self.api_version.to_string());
        if !self.dependencies.is_empty() {
            let mut s = String::from("[\n");
            for (i, d) in self.dependencies.iter().enumerate() {
                s.push_str(&format!(
                    "    {{ \"id\": {}, \"req\": {} }}{}\n",
                    quote(&d.id),
                    quote(&d.req.to_string()),
                    if i + 1 < self.dependencies.len() { "," } else { "" }
                ));
            }
            s.push_str("  ]");
            kv("dependencies", s);
        }
        let p = &self.permissions;
        let mut pf: Vec<String> = Vec::new();
        if !p.namespaces.is_empty() {
            let a: Vec<String> = p.namespaces.iter().map(|s| quote(s)).collect();
            pf.push(format!("    \"namespaces\": [{}]", a.join(", ")));
        }
        if let Some(b) = p.budget {
            pf.push(format!("    \"budget\": {b}"));
        }
        if let Some(m) = p.memory_pages {
            pf.push(format!("    \"memory_pages\": {m}"));
        }
        if !pf.is_empty() {
            kv("permissions", format!("{{\n{}\n  }}", pf.join(",\n")));
        }
        if let Some(meta) = &self.meta {
            kv("meta", canonical(meta));
        }
        format!("{{\n{}\n}}\n", fields.join(",\n"))
    }
}

/// The rules of `meta`: an object, nesting at most [`MAX_META_DEPTH`], canonical text at
/// most [`MAX_META_BYTES`].
fn check_meta(meta: &Value) -> Result<(), PackageError> {
    if meta.entries().is_none() {
        return err("meta: must be a JSON object");
    }
    let depth = depth(meta);
    if depth > MAX_META_DEPTH {
        return err(format!("meta: nesting depth {depth} is more than {MAX_META_DEPTH}"));
    }
    let n = canonical(meta).len();
    if n > MAX_META_BYTES {
        return err(format!("meta: {n} bytes of JSON (at most {MAX_META_BYTES})"));
    }
    Ok(())
}

/// Nesting depth: a scalar is 0, an array or object is 1 + its deepest element.
fn depth(v: &Value) -> usize {
    match v {
        Value::Arr(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        Value::Obj(o) => 1 + o.iter().map(|(_, x)| depth(x)).max().unwrap_or(0),
        _ => 0,
    }
}

/// Canonical JSON text of `v`: no whitespace, object keys in stored order, strings as
/// [`quote`] writes them. Numbers are written as stored (they are already valid JSON).
pub(crate) fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => out.push_str(n),
        Value::Str(s) => out.push_str(&quote(s)),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&quote(k));
                out.push(':');
                write_canonical(x, out);
            }
            out.push('}');
        }
    }
}

fn req_str(root: &Value, key: &str) -> Result<String, PackageError> {
    match root.get(key) {
        None => err(format!("{key}: required")),
        Some(v) => match v.as_str() {
            Some(s) => Ok(s.to_string()),
            None => err(format!("{key}: must be a string")),
        },
    }
}

fn opt_str(root: &Value, key: &str) -> Result<Option<String>, PackageError> {
    match root.get(key) {
        None => Ok(None),
        Some(v) => match v.as_str() {
            Some(s) => Ok(Some(s.to_string())),
            None => err(format!("{key}: must be a string")),
        },
    }
}

/// A JSON string literal. Non-ASCII text is written as is; control characters as escapes.
fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

