//! The `efe.provide` custom section: how a guest describes what it provides (section 13 of
//! the `.efvm` module). The format is defined in crates/efescript/docs/e7.4-wasm-design.md.
//!
//! ```text
//! section = record*
//! record  = u8 kind, uleb32 body_len, body        (readers skip kinds they do not know)
//! kind 0  = function: string name, string export, string doc,
//!                     uleb count + (string name, apitype)*   params,
//!                     uleb count + (string name, apitype)*   results
//! kind 1  = event:    string name, u8 pflag [string payload struct], string doc
//! kind 2  = scratch:  uleb32 size            (the region is the export `efe_scratch`)
//! string  = uleb32 byte length, UTF-8
//! apitype = u8: 0 i32, 1 i64, 2 f32, 3 f64, 4 bool, 5 entity, 6 str, 7 bytes, 8 struct
//!           (8 is followed by string struct name); `ptr` has no encoding
//! ```
//! Everything is built by `const fn`s, so a record is a `static [u8; N]` in the
//! `efe.provide` section. Doc comments are joined with `\n`, one leading space stripped
//! from each line (what `///` leaves behind).

pub const KIND_FUNCTION: u8 = 0;
pub const KIND_EVENT: u8 = 1;
pub const KIND_SCRATCH: u8 = 2;

/// An API type as the record spells it (API_INDEX.md "Parameter types", minus `ptr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiTy {
    I32,
    I64,
    F32,
    F64,
    Bool,
    Entity,
    Str,
    Bytes,
    /// A pointer to a struct of this registered name.
    Struct(&'static str),
}

/// A Rust type that is one scalar API value (`i32 i64 f32 f64 bool Entity`).
pub trait Scalar {
    const API: ApiTy;
}
macro_rules! scalar {
    ($($t:ty => $a:ident),*) => {$(impl Scalar for $t { const API: ApiTy = ApiTy::$a; })*};
}
scalar!(i32 => I32, i64 => I64, f32 => F32, f64 => F64, bool => Bool, crate::Entity => Entity);

/// The results of a provided function: nothing or one scalar.
pub trait Ret {
    const RESULTS: &'static [ApiTy];
}
impl Ret for () {
    const RESULTS: &'static [ApiTy] = &[];
}
impl<T: Scalar> Ret for T {
    const RESULTS: &'static [ApiTy] = &[T::API];
}

/// A `#[repr(C)]` struct with a registered layout. The generated bindings implement it for
/// every struct of the index, so `provide!` can name the struct in a record.
pub trait GuestStruct: Copy {
    /// The name in the API index.
    const NAME: &'static str;
}

/// Writes into `buf`, or only counts when `dry`.
struct W<'a> {
    buf: &'a mut [u8],
    pos: usize,
    dry: bool,
}

impl W<'_> {
    const fn byte(&mut self, b: u8) {
        if !self.dry {
            self.buf[self.pos] = b;
        }
        self.pos += 1;
    }

    const fn uleb(&mut self, mut v: u32) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.byte(b);
                return;
            }
            self.byte(b | 0x80);
        }
    }

    const fn bytes(&mut self, s: &[u8]) {
        let mut i = 0;
        while i < s.len() {
            self.byte(s[i]);
            i += 1;
        }
    }

    const fn string(&mut self, s: &str) {
        self.uleb(s.len() as u32);
        self.bytes(s.as_bytes());
    }

    /// Doc lines joined with `\n`, one leading space stripped from each; at least one
    /// non-blank character is required.
    const fn doc(&mut self, lines: &[&str]) {
        let mut len = 0usize;
        let mut nonblank = false;
        let mut i = 0;
        while i < lines.len() {
            let b = lines[i].as_bytes();
            let skip = if !b.is_empty() && b[0] == b' ' { 1 } else { 0 };
            let mut j = skip;
            while j < b.len() {
                if b[j] != b' ' && b[j] != b'\t' {
                    nonblank = true;
                }
                j += 1;
            }
            len += b.len() - skip + if i > 0 { 1 } else { 0 };
            i += 1;
        }
        assert!(nonblank, "a provided item needs a doc comment (`///`)");
        self.uleb(len as u32);
        let mut i = 0;
        while i < lines.len() {
            if i > 0 {
                self.byte(b'\n');
            }
            let b = lines[i].as_bytes();
            let mut j = if !b.is_empty() && b[0] == b' ' { 1 } else { 0 };
            while j < b.len() {
                self.byte(b[j]);
                j += 1;
            }
            i += 1;
        }
    }

    const fn apitype(&mut self, t: ApiTy) {
        match t {
            ApiTy::I32 => self.byte(0),
            ApiTy::I64 => self.byte(1),
            ApiTy::F32 => self.byte(2),
            ApiTy::F64 => self.byte(3),
            ApiTy::Bool => self.byte(4),
            ApiTy::Entity => self.byte(5),
            ApiTy::Str => self.byte(6),
            ApiTy::Bytes => self.byte(7),
            ApiTy::Struct(n) => {
                self.byte(8);
                self.string(n);
            }
        }
    }

    const fn params(&mut self, ps: &[(&str, ApiTy)]) {
        self.uleb(ps.len() as u32);
        let mut i = 0;
        while i < ps.len() {
            self.string(ps[i].0);
            self.apitype(ps[i].1);
            i += 1;
        }
    }
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn func_body(w: &mut W, name: &str, doc: &[&str], params: &[(&str, ApiTy)], results: &[ApiTy]) {
    w.string(name);
    // The wasm export that holds the function: `efe_provide_<name>` (see `provide!`).
    w.string_export(name);
    w.doc(doc);
    w.params(params);
    w.uleb(results.len() as u32);
    let mut i = 0;
    while i < results.len() {
        let mut k = 0;
        while k < params.len() {
            assert!(!str_eq(params[k].0, "result"), "a parameter is called `result`, which names the result");
            k += 1;
        }
        w.string("result");
        w.apitype(results[i]);
        i += 1;
    }
}

impl W<'_> {
    const fn string_export(&mut self, name: &str) {
        const PREFIX: &str = "efe_provide_";
        self.uleb((PREFIX.len() + name.len()) as u32);
        self.bytes(PREFIX.as_bytes());
        self.bytes(name.as_bytes());
    }
}

const fn dry() -> W<'static> {
    W { buf: &mut [], pos: 0, dry: true }
}

/// Writes the record header: kind and body length.
const fn header(w: &mut W, kind: u8, body: usize) {
    w.byte(kind);
    w.uleb(body as u32);
}

const fn function_body_len(name: &str, doc: &[&str], params: &[(&str, ApiTy)], results: &[ApiTy]) -> usize {
    let mut w = dry();
    func_body(&mut w, name, doc, params, results);
    w.pos
}

/// The byte length of a function record.
pub const fn function_len(name: &str, doc: &[&str], params: &[(&str, ApiTy)], results: &[ApiTy]) -> usize {
    let body = function_body_len(name, doc, params, results);
    let mut w = dry();
    header(&mut w, KIND_FUNCTION, body);
    w.pos + body
}

/// The bytes of a function record; `N` is [`function_len`] of the same arguments.
pub const fn function<const N: usize>(name: &str, doc: &[&str], params: &[(&str, ApiTy)], results: &[ApiTy]) -> [u8; N] {
    let body = function_body_len(name, doc, params, results);
    let mut out = [0u8; N];
    let mut w = W { buf: &mut out, pos: 0, dry: false };
    header(&mut w, KIND_FUNCTION, body);
    func_body(&mut w, name, doc, params, results);
    assert!(w.pos == N, "record length does not match N");
    out
}

const fn event_body(w: &mut W, name: &str, doc: &[&str], payload: Option<&str>) {
    w.string(name);
    match payload {
        Some(p) => {
            w.byte(1);
            w.string(p);
        }
        None => w.byte(0),
    }
    w.doc(doc);
}

const fn event_body_len(name: &str, doc: &[&str], payload: Option<&str>) -> usize {
    let mut w = dry();
    event_body(&mut w, name, doc, payload);
    w.pos
}

/// The byte length of an event record.
pub const fn event_len(name: &str, doc: &[&str], payload: Option<&str>) -> usize {
    let body = event_body_len(name, doc, payload);
    let mut w = dry();
    header(&mut w, KIND_EVENT, body);
    w.pos + body
}

/// The bytes of an event record; `N` is [`event_len`] of the same arguments.
pub const fn event<const N: usize>(name: &str, doc: &[&str], payload: Option<&str>) -> [u8; N] {
    let body = event_body_len(name, doc, payload);
    let mut out = [0u8; N];
    let mut w = W { buf: &mut out, pos: 0, dry: false };
    header(&mut w, KIND_EVENT, body);
    event_body(&mut w, name, doc, payload);
    assert!(w.pos == N, "record length does not match N");
    out
}

const fn scratch_body_len(size: usize) -> usize {
    let mut w = dry();
    w.uleb(size as u32);
    w.pos
}

/// The byte length of a scratch record.
pub const fn scratch_len(size: usize) -> usize {
    let body = scratch_body_len(size);
    let mut w = dry();
    header(&mut w, KIND_SCRATCH, body);
    w.pos + body
}

/// The bytes of a scratch record; `N` is [`scratch_len`] of the same size.
pub const fn scratch<const N: usize>(size: usize) -> [u8; N] {
    assert!(size >= 1 && size <= 1_048_576, "scratch size must be 1..=1048576");
    let body = scratch_body_len(size);
    let mut out = [0u8; N];
    let mut w = W { buf: &mut out, pos: 0, dry: false };
    header(&mut w, KIND_SCRATCH, body);
    w.uleb(size as u32);
    assert!(w.pos == N, "record length does not match N");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_record_bytes() {
        const P: &[(&str, ApiTy)] = &[("a", ApiTy::I32), ("who", ApiTy::Entity), ("s", ApiTy::Str), ("c", ApiTy::Struct("Cfg"))];
        const N: usize = function_len("add", &[" Adds.", " More."], P, &[ApiTy::Bool]);
        const R: [u8; N] = function::<N>("add", &[" Adds.", " More."], P, &[ApiTy::Bool]);
        let mut want = vec![0u8]; // kind
        let mut body = Vec::new();
        let s = |v: &mut Vec<u8>, t: &str| {
            v.push(t.len() as u8);
            v.extend_from_slice(t.as_bytes());
        };
        s(&mut body, "add");
        s(&mut body, "efe_provide_add");
        s(&mut body, "Adds.\nMore.");
        body.push(4);
        s(&mut body, "a");
        body.push(0);
        s(&mut body, "who");
        body.push(5);
        s(&mut body, "s");
        body.push(6);
        s(&mut body, "c");
        body.push(8);
        s(&mut body, "Cfg");
        body.push(1);
        s(&mut body, "result");
        body.push(4);
        want.push(body.len() as u8);
        want.extend(body);
        assert_eq!(R.to_vec(), want);
    }

    #[test]
    fn event_and_scratch_records() {
        const N: usize = event_len("moved", &[" Moved."], Some("Move"));
        let r = event::<N>("moved", &[" Moved."], Some("Move"));
        assert_eq!(r[0], KIND_EVENT);
        assert_eq!(r[1] as usize, N - 2);
        assert_eq!(&r[2..], &[5, b'm', b'o', b'v', b'e', b'd', 1, 4, b'M', b'o', b'v', b'e', 6, b'M', b'o', b'v', b'e', b'd', b'.']);
        const NN: usize = event_len("reset", &[" R."], None);
        assert_eq!(event::<NN>("reset", &[" R."], None)[8], 0);
        const S: usize = scratch_len(4096);
        // 4096 = 0x1000 -> uleb 0x80 0x20
        assert_eq!(scratch::<S>(4096), [KIND_SCRATCH, 2, 0x80, 0x20]);
    }

    #[test]
    fn long_strings_use_multibyte_uleb() {
        const D: &[&str] = &[" x"];
        // A 200-byte name needs a two-byte length.
        const NAME: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const N: usize = function_len(NAME, D, &[], &[]);
        let r = function::<N>(NAME, D, &[], &[]);
        assert_eq!(r.len(), N);
        assert!(r[1] & 0x80 != 0, "body length of a long record is multi-byte");
    }
}
