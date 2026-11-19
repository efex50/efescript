//! The `.efapi` JSON writer and a small, dependency-free JSON reader (API_INDEX.md).
//!
//! The reader never panics: nesting is limited to [`MAX_DEPTH`], every index is checked and
//! numbers are kept as text until a field asks for a `u32`.

use crate::api::index::{ApiError, ApiIndex};
use crate::api::types::{ApiType, EventDesc, FieldDesc, HostFn, NamespaceInfo, Param, Prim, StructDesc, fnv1a64};
use std::fmt::Write as _;

/// Maximum nesting of arrays and objects the reader accepts (an index needs 4).
pub const MAX_DEPTH: usize = 64;

// ---------------------------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------------------------

/// A JSON string literal.
fn q(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn param(p: &Param) -> String {
    format!("{{ \"name\": {}, \"type\": {} }}", q(&p.name), q(&p.ty.spelling()))
}

/// Writes the file. `hash = None` writes the canonical body (no `game` and `hash` lines).
pub(crate) fn write(ix: &ApiIndex, hash: Option<u64>) -> String {
    let mut ns: Vec<&NamespaceInfo> = ix.namespaces.iter().collect();
    ns.sort_by(|a, b| a.name.cmp(&b.name));
    let mut ty: Vec<&StructDesc> = ix.types.iter().collect();
    ty.sort_by(|a, b| a.name.cmp(&b.name));
    let mut fs: Vec<&HostFn> = ix.functions.iter().collect();
    fs.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    let mut ev: Vec<&EventDesc> = ix.events.iter().collect();
    ev.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));

    let mut o = String::new();
    o.push_str("{\n  \"format\": \"efapi\",\n  \"version\": 2,\n");
    if let Some(h) = hash {
        let _ = writeln!(o, "  \"game\": {},", q(&ix.game));
        let _ = writeln!(o, "  \"hash\": \"fnv1a64:{h:016x}\",");
    }

    o.push_str("  \"namespaces\": [");
    for (i, n) in ns.iter().enumerate() {
        o.push_str(if i == 0 { "\n" } else { ",\n" });
        let _ = write!(
            o,
            "    {{ \"name\": {}, \"version\": {}, \"owner\": {}, \"doc\": {} }}",
            q(&n.name),
            n.version,
            q(&n.owner),
            q(&n.doc)
        );
    }
    o.push_str(if ns.is_empty() { "],\n" } else { "\n  ],\n" });

    o.push_str("  \"types\": [");
    for (i, t) in ty.iter().enumerate() {
        o.push_str(if i == 0 { "\n" } else { ",\n" });
        o.push_str("    {\n");
        let _ = writeln!(o, "      \"name\": {}, \"owner\": {}, \"doc\": {},", q(&t.name), q(&t.owner), q(&t.doc));
        let comp = t.component.map_or_else(|| "null".to_string(), |c| c.to_string());
        let ro = if t.readonly { ", \"readonly\": true" } else { "" };
        let _ = writeln!(o, "      \"size\": {}, \"align\": {}, \"component\": {comp}{ro},", t.size, t.align);
        o.push_str("      \"fields\": [");
        for (j, f) in t.fields.iter().enumerate() {
            o.push_str(if j == 0 { "\n" } else { ",\n" });
            let _ = write!(
                o,
                "        {{ \"name\": {}, \"type\": {}, \"count\": {}, \"offset\": {}, \"doc\": {} }}",
                q(&f.name),
                q(f.ty.name()),
                f.count,
                f.offset,
                q(&f.doc)
            );
        }
        o.push_str(if t.fields.is_empty() { "]\n" } else { "\n      ]\n" });
        o.push_str("    }");
    }
    o.push_str(if ty.is_empty() { "],\n" } else { "\n  ],\n" });

    o.push_str("  \"functions\": [");
    for (i, f) in fs.iter().enumerate() {
        o.push_str(if i == 0 { "\n" } else { ",\n" });
        o.push_str("    {\n");
        let _ = writeln!(
            o,
            "      \"namespace\": {}, \"name\": {}, \"owner\": {},",
            q(&f.namespace),
            q(&f.name),
            q(&f.owner)
        );
        let _ = writeln!(o, "      \"doc\": {},", q(&f.doc));
        o.push_str("      \"params\": [");
        for (j, p) in f.params.iter().enumerate() {
            o.push_str(if j == 0 { "\n        " } else { ",\n        " });
            o.push_str(&param(p));
        }
        o.push_str(if f.params.is_empty() { "],\n" } else { "\n      ],\n" });
        if f.results.is_empty() {
            o.push_str("      \"results\": [],\n");
        } else {
            let rs: Vec<String> = f.results.iter().map(param).collect();
            let _ = writeln!(o, "      \"results\": [ {} ],", rs.join(", "));
        }
        let _ = writeln!(o, "      \"signature\": {}", q(&f.func_type().to_string()));
        o.push_str("    }");
    }
    o.push_str(if fs.is_empty() { "],\n" } else { "\n  ],\n" });

    o.push_str("  \"events\": [");
    for (i, e) in ev.iter().enumerate() {
        o.push_str(if i == 0 { "\n" } else { ",\n" });
        let payload = e.payload.as_deref().map_or_else(|| "null".to_string(), q);
        let _ = write!(
            o,
            "    {{ \"namespace\": {}, \"name\": {}, \"owner\": {}, \"payload\": {payload}, \"doc\": {} }}",
            q(&e.namespace),
            q(&e.name),
            q(&e.owner),
            q(&e.doc)
        );
    }
    o.push_str(if ev.is_empty() { "]\n" } else { "\n  ]\n" });
    o.push_str("}\n");
    o
}

// ---------------------------------------------------------------------------------------------
// Reader: JSON values
// ---------------------------------------------------------------------------------------------

/// A parsed JSON value, as produced by [`parse`].
///
/// Accepts exactly RFC 8259 JSON: no comments, no trailing commas, no single quotes, no
/// `NaN`/`Infinity`. Nesting of arrays and objects is limited to [`MAX_DEPTH`] (64). Object
/// keys are kept in file order and duplicate keys are rejected. Every error is an
/// [`ApiError::Syntax`] with the byte `offset` and a `message`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// The number as written (validated JSON number syntax, e.g. `"-0.5e+3"`).
    Num(String),
    /// A string with escapes already decoded.
    Str(String),
    /// An array.
    Arr(Vec<Value>),
    /// An object: keys in order; duplicate keys are rejected by the parser.
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// The string, if this is a [`Value::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value as a `u64`, if this is a number written as an unsigned integer (no `-`, no
    /// fraction, no exponent) that fits in `u64`. Floats such as `1.0` give `None`.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Num(n) if n.bytes().all(|c| c.is_ascii_digit()) => n.parse().ok(),
            _ => None,
        }
    }

    /// The value as an `i64`, if this is a number written as an integer (an optional leading
    /// `-`, no fraction, no exponent) that fits in `i64`. Floats such as `1.0` give `None`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.bytes().all(|c| c.is_ascii_digit() || c == b'-') => n.parse().ok(),
            _ => None,
        }
    }

    /// The flag, if this is a [`Value::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements, if this is a [`Value::Arr`].
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// The `(key, value)` pairs in file order, if this is a [`Value::Obj`].
    pub fn entries(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(items) => Some(items),
            _ => None,
        }
    }

    /// The value stored under `key`, if this is a [`Value::Obj`] that has that key.
    /// Keys are unique (the parser rejects duplicates), so the result is unambiguous.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    depth: usize,
}

type PResult<T> = Result<T, ApiError>;

impl<'a> Parser<'a> {
    fn err<T>(&self, message: &str) -> PResult<T> {
        Err(ApiError::Syntax { offset: self.pos, message: message.to_string() })
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> PResult<Value> {
        if self.b.get(self.pos..self.pos + word.len()) == Some(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            self.err("invalid literal")
        }
    }

    fn value(&mut self) -> PResult<Value> {
        self.ws();
        match self.peek() {
            None => self.err("unexpected end of input"),
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> PResult<Value>) -> PResult<Value> {
        if self.depth >= MAX_DEPTH {
            return self.err("nesting too deep");
        }
        self.depth += 1;
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn object(&mut self) -> PResult<Value> {
        self.pos += 1; // '{'
        let mut items: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Obj(items));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return self.err("expected a key string");
            }
            let at = self.pos;
            let k = self.string()?;
            if items.iter().any(|(e, _)| *e == k) {
                return Err(ApiError::Syntax { offset: at, message: format!("duplicate key {k:?}") });
            }
            self.ws();
            if self.peek() != Some(b':') {
                return self.err("expected ':'");
            }
            self.pos += 1;
            let v = self.value()?;
            items.push((k, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Obj(items));
                }
                _ => return self.err("expected ',' or '}'"),
            }
        }
    }

    fn array(&mut self) -> PResult<Value> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return self.err("expected ',' or ']'"),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> PResult<Value> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.err("invalid number"),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return self.err("invalid number");
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return self.err("invalid number");
            }
        }
        match self.s.get(start..self.pos) {
            Some(t) => Ok(Value::Num(t.to_string())),
            None => self.err("invalid number"),
        }
    }

    fn hex4(&mut self) -> PResult<u32> {
        let Some(h) = self.s.get(self.pos..self.pos + 4) else {
            return self.err("truncated \\u escape");
        };
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return self.err("invalid \\u escape");
        }
        self.pos += 4;
        u32::from_str_radix(h, 16).or_else(|_| self.err("invalid \\u escape"))
    }

    fn string(&mut self) -> PResult<String> {
        self.pos += 1; // '"'
        let mut out = String::new();
        loop {
            let run = self.pos;
            while matches!(self.peek(), Some(c) if c != b'"' && c != b'\\' && c >= 0x20) {
                self.pos += 1;
            }
            // `"`, `\` and control bytes are ASCII, so `run..pos` is on char boundaries.
            match self.s.get(run..self.pos) {
                Some(t) => out.push_str(t),
                None => return self.err("invalid string"),
            }
            match self.peek() {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let Some(e) = self.peek() else { return self.err("unterminated string") };
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.b.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return self.err("unpaired high surrogate");
                                }
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("invalid low surrogate");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return self.err("unpaired low surrogate");
                            } else {
                                hi
                            };
                            match char::from_u32(cp) {
                                Some(c) => out.push(c),
                                None => return self.err("invalid code point"),
                            }
                        }
                        _ => {
                            self.pos -= 1;
                            return self.err("invalid escape");
                        }
                    }
                }
                Some(_) => return self.err("control character in string"),
            }
        }
    }
}

/// Parses one JSON value. Surrounding whitespace (space, tab, CR, LF) is allowed, anything
/// else after the value is an error. The grammar and limits are those of [`Value`]: nesting
/// deeper than [`MAX_DEPTH`] (64), duplicate object keys, and malformed text are rejected with
/// [`ApiError::Syntax`] (`offset` is the byte position of the problem). Never panics.
pub fn parse(text: &str) -> Result<Value, ApiError> {
    let mut p = Parser { s: text, b: text.as_bytes(), pos: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.pos != p.b.len() {
        return p.err("trailing characters after the JSON value");
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// Reader: index schema
// ---------------------------------------------------------------------------------------------

fn schema<T>(path: &str, message: &str) -> Result<T, ApiError> {
    Err(ApiError::Schema { path: path.to_string(), message: message.to_string() })
}

/// An object being read; `path` is used in error messages.
struct Obj<'v> {
    items: &'v [(String, Value)],
    path: String,
}

impl<'v> Obj<'v> {
    fn of(v: &'v Value, path: String) -> Result<Obj<'v>, ApiError> {
        match v {
            Value::Obj(items) => Ok(Obj { items, path }),
            _ => schema(&path, "expected an object"),
        }
    }

    fn get(&self, key: &str) -> Result<&'v Value, ApiError> {
        match self.items.iter().find(|(k, _)| k == key) {
            Some((_, v)) => Ok(v),
            None => schema(&self.path, &format!("missing key \"{key}\"")),
        }
    }

    fn sub(&self, key: &str) -> String {
        if self.path.is_empty() { key.to_string() } else { format!("{}.{key}", self.path) }
    }

    fn str(&self, key: &str) -> Result<String, ApiError> {
        match self.get(key)? {
            Value::Str(s) => Ok(s.clone()),
            _ => schema(&self.sub(key), "expected a string"),
        }
    }

    fn u32(&self, key: &str) -> Result<u32, ApiError> {
        match self.get(key)? {
            Value::Num(n) => match n.parse::<u32>() {
                Ok(v) if n.bytes().all(|c| c.is_ascii_digit()) => Ok(v),
                _ => schema(&self.sub(key), "expected an integer from 0 to 4294967295"),
            },
            _ => schema(&self.sub(key), "expected a number"),
        }
    }

    fn arr(&self, key: &str) -> Result<&'v [Value], ApiError> {
        match self.get(key)? {
            Value::Arr(a) => Ok(a),
            _ => schema(&self.sub(key), "expected an array"),
        }
    }

    fn each(&self, key: &str) -> Result<Vec<Obj<'v>>, ApiError> {
        let path = self.sub(key);
        self.arr(key)?.iter().enumerate().map(|(i, v)| Obj::of(v, format!("{path}[{i}]"))).collect()
    }
}

fn params(o: &Obj<'_>, key: &str) -> Result<Vec<Param>, ApiError> {
    o.each(key)?
        .iter()
        .map(|p| {
            let t = p.str("type")?;
            match ApiType::parse(&t) {
                Some(ty) => Ok(Param { name: p.str("name")?, ty }),
                None => schema(&p.sub("type"), &format!("unknown type {t:?}")),
            }
        })
        .collect()
}

/// `fnv1a64` of `text` with its top-level `game` and `hash` lines removed: the hash of a
/// file exactly as the writer formats it, keys this reader does not know included.
fn raw_body_hash(text: &str) -> u64 {
    let mut h = Vec::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with("  \"game\":") || line.starts_with("  \"hash\":") {
            continue;
        }
        h.extend_from_slice(line.as_bytes());
    }
    fnv1a64(&h)
}

fn parse_hash(s: &str) -> Option<u64> {
    let hex = s.strip_prefix("fnv1a64:")?;
    if hex.len() != 16 || !hex.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

pub(crate) fn read(text: &str) -> Result<ApiIndex, ApiError> {
    let root = parse(text)?;
    let top = Obj::of(&root, String::new())?;
    match top.get("format")? {
        Value::Str(s) if s == "efapi" => {}
        Value::Str(s) => return Err(ApiError::Format(s.clone())),
        _ => return Err(ApiError::Format("(not a string)".into())),
    }
    match top.get("version")? {
        Value::Num(n) if n == "2" => {}
        Value::Num(n) => return Err(ApiError::Version(n.clone())),
        _ => return schema("version", "expected a number"),
    }
    let game = top.str("game")?;
    let hash_text = top.str("hash")?;
    let Some(file_hash) = parse_hash(&hash_text) else {
        return schema("hash", "expected \"fnv1a64:\" and 16 lower-case hex digits");
    };

    let mut ix = ApiIndex { game, ..ApiIndex::default() };
    for n in top.each("namespaces")? {
        ix.namespaces.push(NamespaceInfo {
            name: n.str("name")?,
            version: n.u32("version")?,
            owner: n.str("owner")?,
            doc: n.str("doc")?,
        });
    }
    for t in top.each("types")? {
        let component = match t.get("component")? {
            Value::Null => None,
            _ => Some(t.u32("component")?),
        };
        let mut fields = Vec::new();
        for f in t.each("fields")? {
            let ty = f.str("type")?;
            let Some(prim) = Prim::parse(&ty) else {
                return schema(&f.sub("type"), &format!("unknown field type {ty:?}"));
            };
            fields.push(FieldDesc {
                name: f.str("name")?,
                offset: f.u32("offset")?,
                ty: prim,
                count: f.u32("count")?,
                doc: f.str("doc")?,
            });
        }
        ix.types.push(StructDesc {
            name: t.str("name")?,
            size: t.u32("size")?,
            align: t.u32("align")?,
            fields,
            doc: t.str("doc")?,
            owner: t.str("owner")?,
            component,
            readonly: matches!(t.get("readonly"), Ok(Value::Bool(true))),
        });
    }
    let mut sigs = Vec::new();
    for f in top.each("functions")? {
        let h = HostFn {
            namespace: f.str("namespace")?,
            name: f.str("name")?,
            doc: f.str("doc")?,
            params: params(&f, "params")?,
            results: params(&f, "results")?,
            owner: f.str("owner")?,
        };
        sigs.push((h.qualified(), f.str("signature")?, h.func_type().to_string()));
        ix.functions.push(h);
    }
    for e in top.each("events")? {
        let payload = match e.get("payload")? {
            Value::Null => None,
            _ => Some(e.str("payload")?),
        };
        ix.events.push(EventDesc {
            namespace: e.str("namespace")?,
            name: e.str("name")?,
            payload,
            doc: e.str("doc")?,
            owner: e.str("owner")?,
        });
    }
    ix.sort();
    ix.validate()?;
    for (function, file, computed) in sigs {
        if file != computed {
            return Err(ApiError::Signature { function, file, computed });
        }
    }
    let computed = ix.hash();
    if file_hash != computed && file_hash != raw_body_hash(text) {
        return Err(ApiError::Hash { file: hash_text, computed });
    }
    Ok(ix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ApiIndex {
        let mut t = StructDesc::new("Transform", 20, 4)
            .doc("Position, rotation and scale.")
            .field(FieldDesc::new("position", 0, Prim::F32, 2))
            .field(FieldDesc::new("rotation", 8, Prim::F32, 1).with_doc("Radians."))
            .field(FieldDesc::new("scale", 12, Prim::F32, 2));
        t.owner = "engine".into();
        t.component = Some(2166136261);
        let mut e = StructDesc::new("Empty", 0, 1).doc("Nothing \"quoted\"\n\u{1}\u{1F600}");
        e.owner = "engine".into();
        let mut get = HostFn::new("engine.world", "get")
            .doc("Copy component `component` of `entity` to `out`; 0 ok, 1 stale entity, 2 no such component.")
            .param("entity", ApiType::Entity)
            .param("component", ApiType::I32)
            .param("out", ApiType::Ptr)
            .result("status", ApiType::I32);
        get.owner = "engine".into();
        let mut set = HostFn::new("engine.world", "set")
            .doc("Set.")
            .param("t", ApiType::Struct("Transform".into()))
            .param("name", ApiType::Str);
        set.owner = "engine".into();
        let mut two = HostFn::new("engine.time", "both").doc("Two results.").result("a", ApiType::F32).result("b", ApiType::Bool);
        two.owner = "engine".into();
        ApiIndex {
            game: "script_demo".into(),
            namespaces: vec![
                NamespaceInfo { name: "engine.time".into(), version: 1, owner: "engine".into(), doc: "Time.".into() },
                NamespaceInfo {
                    name: "engine.world".into(),
                    version: 1,
                    owner: "engine".into(),
                    doc: "Entities and components.".into(),
                },
            ],
            types: vec![e, t],
            functions: vec![two, get, set],
            events: vec![
                EventDesc {
                    namespace: "engine.world".into(),
                    name: "moved".into(),
                    payload: Some("Transform".into()),
                    doc: "An entity moved.".into(),
                    owner: "engine".into(),
                },
                EventDesc::new("engine.time", "tick").doc("A frame began."),
            ],
        }
    }

    #[test]
    fn layout_matches_api_index_md() {
        let j = sample().to_json();
        let want_fn = r#"    {
      "namespace": "engine.world", "name": "get", "owner": "engine",
      "doc": "Copy component `component` of `entity` to `out`; 0 ok, 1 stale entity, 2 no such component.",
      "params": [
        { "name": "entity", "type": "entity" },
        { "name": "component", "type": "i32" },
        { "name": "out", "type": "ptr" }
      ],
      "results": [ { "name": "status", "type": "i32" } ],
      "signature": "(i64, i32, i64) -> i32"
    }"#;
        assert!(j.contains(want_fn), "{j}");
        let want_ty = r#"    {
      "name": "Transform", "owner": "engine", "doc": "Position, rotation and scale.",
      "size": 20, "align": 4, "component": 2166136261,
      "fields": [
        { "name": "position", "type": "f32", "count": 2, "offset": 0, "doc": "" },
        { "name": "rotation", "type": "f32", "count": 1, "offset": 8, "doc": "Radians." },
        { "name": "scale", "type": "f32", "count": 2, "offset": 12, "doc": "" }
      ]
    }"#;
        assert!(j.contains(want_ty), "{j}");
        assert!(j.starts_with("{\n  \"format\": \"efapi\",\n  \"version\": 2,\n  \"game\": \"script_demo\",\n  \"hash\": \"fnv1a64:"));
        assert!(j.contains("\n    { \"name\": \"engine.world\", \"version\": 1, \"owner\": \"engine\", \"doc\": \"Entities and components.\" }\n  ],\n"));
        assert!(j.contains("\"fields\": []\n"));
        assert!(j.contains("\"params\": [],\n      \"results\": [ { \"name\": \"a\", \"type\": \"f32\" }, { \"name\": \"b\", \"type\": \"bool\" } ],"));
        assert!(j.contains("\"signature\": \"(i64, i64, i64)\"\n"));
        assert!(j.contains("\\\"quoted\\\"\\n\\u0001\u{1F600}"));
        let want_ev = "  \"events\": [\n    { \"namespace\": \"engine.time\", \"name\": \"tick\", \"owner\": \"\", \"payload\": null, \"doc\": \"A frame began.\" },\n    { \"namespace\": \"engine.world\", \"name\": \"moved\", \"owner\": \"engine\", \"payload\": \"Transform\", \"doc\": \"An entity moved.\" }\n  ]\n}\n";
        assert!(j.ends_with(want_ev), "{j}");
        assert!(j.contains("\"signature\": \"(i64, i64, i64)\"\n    }\n  ],\n  \"events\""));
        let empty = ApiIndex::default().to_json();
        assert!(empty.ends_with("  \"namespaces\": [],\n  \"types\": [],\n  \"functions\": [],\n  \"events\": []\n}\n"), "{empty}");
    }

    #[test]
    fn round_trip_and_hash() {
        let mut x = sample();
        x.sort();
        let j = x.to_json();
        let y = ApiIndex::from_json(&j).unwrap();
        assert_eq!(y, x);
        assert_eq!(y.hash(), x.hash());
        assert_eq!(y.to_json(), j);
        // The game name is not hashed.
        let mut g = x.clone();
        g.game = "other".into();
        assert_eq!(g.hash(), x.hash());
        assert_ne!(g.to_json(), j);
        // Any API change changes the hash.
        let mut d = x.clone();
        d.functions[0].doc.push('!');
        assert_ne!(d.hash(), x.hash());
        let mut d = x.clone();
        assert_eq!(d.events[1].name, "moved");
        d.events[1].payload = None;
        assert_ne!(d.hash(), x.hash());
        assert_eq!(x.event("engine.world", "moved").unwrap().payload.as_deref(), Some("Transform"));
        assert!(x.event("engine.world", "nope").is_none());
        assert_eq!(ApiIndex::from_json(&ApiIndex::default().to_json()), Ok(ApiIndex::default()));
        assert_eq!(x.function("engine.world", "get").unwrap().name, "get");
        assert_eq!(x.qualified_names(), ["engine.time.both", "engine.world.get", "engine.world.set"]);
        assert_eq!(x.struct_type("Transform").unwrap().size, 20);
        assert!(x.namespace("engine.time").is_some() && x.struct_type("Nope").is_none());
    }

    #[test]
    fn reader_checks() {
        let j = sample().to_json();
        let edit = |from: &str, to: &str| ApiIndex::from_json(&j.replacen(from, to, 1));
        assert!(matches!(edit("\"efapi\"", "\"efapo\""), Err(ApiError::Format(_))));
        assert!(matches!(edit("\"version\": 2", "\"version\": 1"), Err(ApiError::Version(_))));
        assert!(matches!(edit("\"version\": 2", "\"version\": 3"), Err(ApiError::Version(_))));
        assert!(matches!(edit("\"payload\": \"Transform\"", "\"payload\": \"Nope\""), Err(ApiError::Invalid(_))));
        assert!(matches!(edit("\"payload\": null", "\"payload\": 1"), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("\"name\": \"tick\"", "\"name\": \"both\""), Err(ApiError::Invalid(_))));
        assert!(matches!(edit("\"An entity moved.\"", "\"An entity jumped.\""), Err(ApiError::Hash { .. })));
        // `events` is a required key in version 2.
        let no_events = j.replacen(",\n  \"events\": [", ",\n  \"evts\": [", 1);
        assert!(matches!(ApiIndex::from_json(&no_events), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("Radians.", "Degrees."), Err(ApiError::Hash { .. })));
        assert!(matches!(edit("-> i32\"", "-> i64\""), Err(ApiError::Signature { .. })));
        assert!(matches!(edit("\"fnv1a64:", "\"fnv1a32:"), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("\"type\": \"ptr\"", "\"type\": \"pointer\""), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("\"name\": \"get\"", "\"name\": \"g-t\""), Err(ApiError::Invalid(_))));
        assert!(matches!(edit("\"size\": 20", "\"size\": 20.0"), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("\"size\": 20", "\"size\": -20"), Err(ApiError::Schema { .. })));
        assert!(matches!(edit("\"size\": 20", "\"size\": 99999999999"), Err(ApiError::Schema { .. })));
        // Game name edits and unknown keys are fine (the hash skips the game line; unknown keys
        // are part of the raw body the writer of a newer version would hash).
        assert!(edit("\"script_demo\"", "\"renamed\"").is_ok());
        // Reformatted but equal content passes via the canonical hash.
        let compact: String = j.replace("\n", "").replace("  ", "");
        assert_eq!(ApiIndex::from_json(&compact), ApiIndex::from_json(&j));
        // A newer v2 writer adding a key: the raw body hash covers it.
        let mut ext = sample();
        ext.sort();
        let body = write(&ext, None).replacen("  \"namespaces\"", "  \"extra\": {\"a\": [1, 2.5e-3, true, null]},\n  \"namespaces\"", 1);
        let h = fnv1a64(body.as_bytes());
        let file = body.replacen("  \"version\": 2,\n", &format!("  \"version\": 2,\n  \"game\": \"g\",\n  \"hash\": \"fnv1a64:{h:016x}\",\n"), 1);
        assert!(ApiIndex::from_json(&file).is_ok(), "{:?}", ApiIndex::from_json(&file));
    }

    #[test]
    fn json_syntax() {
        let ok = |s: &str| parse(s).unwrap();
        assert_eq!(ok(r#""a\u00e9\ud83d\ude00\n\/""#), Value::Str("a\u{e9}\u{1F600}\n/".into()));
        assert_eq!(ok(" [1, -0.5e+3, true, false, null, {}] "), Value::Arr(vec![
            Value::Num("1".into()),
            Value::Num("-0.5e+3".into()),
            Value::Bool(true),
            Value::Bool(false),
            Value::Null,
            Value::Obj(vec![]),
        ]));
        for bad in [
            "", "[", "{", "{\"a\"}", "{\"a\":1,}", "[1,]", "01", "1.", "-", "1e", "\"\\ud800\"", "\"\\udc00\"",
            "\"\\ud800\\u0041\"", "\"\\x\"", "\"a\nb\"", "\"\\u12\"", "tru", "nul", "[1] 2", "{\"a\":1,\"a\":2}", "\"",
            "\"\\u12g4\"", "+1",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&deep).is_ok());
        let deeper = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(parse(&deeper).is_err());
        assert!(parse(&"[".repeat(100_000)).is_err());
    }

    #[test]
    fn mutated_and_truncated_never_panic() {
        let j = sample().to_json();
        let b = j.as_bytes();
        // Every proper prefix is rejected, except the one that only drops the final newline
        // (the same content, so the canonical hash still matches).
        assert!(ApiIndex::from_json(&j[..j.len() - 1]).is_ok());
        for n in 0..b.len() - 1 {
            if let Ok(s) = std::str::from_utf8(&b[..n]) {
                assert!(ApiIndex::from_json(s).is_err(), "prefix {n} accepted");
            }
        }
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        const PICK: &[u8] = b"{}[]\":,\\0123456789-+.eEtrufalsn \n\tu\x01\xc3\xa9";
        for _ in 0..4000 {
            let mut m = b.to_vec();
            for _ in 0..1 + next() % 4 {
                let i = (next() as usize) % m.len();
                match next() % 3 {
                    0 => m[i] = PICK[(next() as usize) % PICK.len()],
                    1 => {
                        m.remove(i);
                    }
                    _ => m.insert(i, PICK[(next() as usize) % PICK.len()]),
                }
            }
            let s = String::from_utf8_lossy(&m);
            let _ = ApiIndex::from_json(&s);
        }
    }

    #[test]
    fn value_accessors() {
        let v = parse(r#"{"name": "efemod", "n": 42, "neg": -7, "f": 1.5, "on": true, "list": [1, "x"]}"#).unwrap();
        assert_eq!(v.get("name").and_then(Value::as_str), Some("efemod"));
        assert_eq!(v.get("n").and_then(Value::as_u64), Some(42));
        assert_eq!(v.get("neg").and_then(Value::as_i64), Some(-7));
        assert_eq!(v.get("neg").and_then(Value::as_u64), None);
        assert_eq!(v.get("f").and_then(Value::as_u64), None);
        assert_eq!(v.get("f").and_then(Value::as_i64), None);
        assert_eq!(v.get("on").and_then(Value::as_bool), Some(true));
        assert_eq!(v.get("list").and_then(Value::as_array).map(<[Value]>::len), Some(2));
        assert_eq!(v.get("missing"), None);
        assert_eq!(v.get("name").and_then(Value::as_u64), None);
        assert_eq!(v.entries().map(<[(String, Value)]>::len), Some(6));
        assert_eq!(v.entries().unwrap()[0].0, "name");
        assert_eq!(v.as_array(), None);
        assert_eq!(parse("[1]").unwrap().get("x"), None);
    }

    #[test]
    fn parse_rejects_duplicates_and_deep_nesting() {
        match parse(r#"{"a": 1, "a": 2}"#) {
            Err(ApiError::Syntax { message, .. }) => assert!(message.contains("duplicate"), "{message}"),
            other => panic!("expected a syntax error, got {other:?}"),
        }
        assert!(parse(&"[".repeat(MAX_DEPTH + 1)).is_err());
    }
}
