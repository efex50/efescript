//! Writers for the `efe.provide` custom section and helpers to append custom sections.
#![allow(dead_code)]

pub fn uleb(mut v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

pub fn string(s: &str) -> Vec<u8> {
    let mut o = uleb(s.len() as u32);
    o.extend(s.as_bytes());
    o
}

pub fn record(kind: u8, body: Vec<u8>) -> Vec<u8> {
    let mut o = vec![kind];
    o.extend(uleb(body.len() as u32));
    o.extend(body);
    o
}

/// `(name, code, struct name for code 8)`.
pub type P<'a> = (&'a str, u8, Option<&'a str>);

fn params(ps: &[P]) -> Vec<u8> {
    let mut o = uleb(ps.len() as u32);
    for (n, c, s) in ps {
        o.extend(string(n));
        o.push(*c);
        if let Some(s) = s {
            o.extend(string(s));
        }
    }
    o
}

pub fn func(name: &str, export: &str, doc: &str, ps: &[P], rs: &[P]) -> Vec<u8> {
    let mut b = string(name);
    b.extend(string(export));
    b.extend(string(doc));
    b.extend(params(ps));
    b.extend(params(rs));
    record(0, b)
}

pub fn event(name: &str, payload: Option<&str>, doc: &str) -> Vec<u8> {
    let mut b = string(name);
    match payload {
        Some(p) => {
            b.push(1);
            b.extend(string(p));
        }
        None => b.push(0),
    }
    b.extend(string(doc));
    record(1, b)
}

pub fn scratch(size: u32) -> Vec<u8> {
    record(2, uleb(size))
}

/// Appends a custom section to a wasm module.
pub fn with_custom(mut wasm: Vec<u8>, name: &str, data: &[u8]) -> Vec<u8> {
    let mut body = string(name);
    body.extend(data);
    wasm.push(0);
    wasm.extend(uleb(body.len() as u32));
    wasm.extend(body);
    wasm
}

pub const I32: u8 = 0;
pub const I64: u8 = 1;
pub const F32: u8 = 2;
pub const F64: u8 = 3;
pub const BOOL: u8 = 4;
pub const ENTITY: u8 = 5;
pub const STR: u8 = 6;
pub const BYTES: u8 = 7;
pub const STRUCT: u8 = 8;
