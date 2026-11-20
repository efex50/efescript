//! Shared by the guest tests: a decoder for the `efe.provide` custom section written
//! independently of `efe-guest` and `efe_guest.h` (format: docs/e7.4-wasm-design.md).
#![allow(dead_code)]

use efevm::api::ApiIndex;
use efescript::bindmodel::Wasm;
use std::path::PathBuf;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

pub fn fixture_index() -> ApiIndex {
    let text = std::fs::read_to_string(fixture("guest_test.efapi")).expect("fixture index");
    ApiIndex::from_json(&text).expect("valid fixture index")
}

/// An API type of a record: code 0..=8 and, for 8, the struct name.
pub type Ty = (u8, Option<String>);

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Rec {
    Func { name: String, export: String, doc: String, params: Vec<(String, Ty)>, results: Vec<(String, Ty)> },
    Event { name: String, payload: Option<String>, doc: String },
    Scratch(u32),
}

struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        let (b, rest) = self.0.split_first().ok_or("truncated")?;
        self.0 = rest;
        Ok(*b)
    }
    /// A uleb128; zero-padded forms are accepted.
    fn uleb(&mut self) -> Result<u32, String> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let b = self.u8()?;
            v |= ((b & 0x7f) as u64) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                return u32::try_from(v).map_err(|_| "uleb too big".to_string());
            }
            if shift > 35 {
                return Err("uleb too long".into());
            }
        }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.uleb()? as usize;
        String::from_utf8(self.bytes(n)?.to_vec()).map_err(|e| e.to_string())
    }
    fn ty(&mut self) -> Result<Ty, String> {
        let c = self.u8()?;
        match c {
            0..=7 => Ok((c, None)),
            8 => Ok((8, Some(self.string()?))),
            _ => Err(format!("bad api type {c}")),
        }
    }
    fn params(&mut self) -> Result<Vec<(String, Ty)>, String> {
        let n = self.uleb()?;
        (0..n).map(|_| Ok((self.string()?, self.ty()?))).collect()
    }
}

/// Decodes the whole `efe.provide` section; unknown record kinds are skipped.
pub fn decode_provide(bytes: &[u8]) -> Result<Vec<Rec>, String> {
    let mut r = Rd(bytes);
    let mut out = Vec::new();
    while !r.0.is_empty() {
        let kind = r.u8()?;
        let len = r.uleb()? as usize;
        let mut b = Rd(r.bytes(len)?);
        match kind {
            0 => {
                let name = b.string()?;
                let export = b.string()?;
                let doc = b.string()?;
                let params = b.params()?;
                let results = b.params()?;
                out.push(Rec::Func { name, export, doc, params, results });
            }
            1 => {
                let name = b.string()?;
                let payload = match b.u8()? {
                    0 => None,
                    1 => Some(b.string()?),
                    f => return Err(format!("bad payload flag {f}")),
                };
                let doc = b.string()?;
                out.push(Rec::Event { name, payload, doc });
            }
            2 => out.push(Rec::Scratch(b.uleb()?)),
            _ => {}
        }
        if !b.0.is_empty() && kind <= 2 {
            return Err(format!("record of kind {kind} has {} trailing bytes", b.0.len()));
        }
    }
    Ok(out)
}

/// The wasm values of a record type (design doc, section 3).
pub fn lower(t: &Ty) -> Vec<Wasm> {
    match t.0 {
        0 | 4 | 8 => vec![Wasm::I32],
        1 | 5 => vec![Wasm::I64],
        2 => vec![Wasm::F32],
        3 => vec![Wasm::F64],
        _ => vec![Wasm::I32, Wasm::I32],
    }
}

/// The records every example mod (Rust and C) must provide, by name.
pub fn expected_records() -> Vec<Rec> {
    let p = |n: &str, c: u8| (n.to_string(), (c, None));
    let res = |c: u8| vec![("result".to_string(), (c, None))];
    vec![
        Rec::Func { name: "add".into(), export: "efe_provide_add".into(), doc: "Adds two numbers, saturating.".into(), params: vec![p("a", 0), p("b", 0)], results: res(0) },
        Rec::Func {
            name: "weigh".into(),
            export: "efe_provide_weigh".into(),
            doc: "Sums the bytes of `data`, the length of `name`, and the score and handle it is given.".into(),
            params: vec![p("name", 6), p("data", 7), ("cfg".into(), (8, Some("Score".into()))), p("who", 5), p("on", 4)],
            results: res(3),
        },
        Rec::Func { name: "checked".into(), export: "efe_provide_checked".into(), doc: "Traps when `n` is negative.".into(), params: vec![p("n", 0)], results: res(0) },
        Rec::Func { name: "run".into(), export: "efe_provide_run".into(), doc: "Calls imports of the bindings.".into(), params: vec![p("n", 0)], results: res(0) },
        Rec::Event { name: "coin_picked".into(), payload: Some("Score".into()), doc: "A coin was picked up.".into() },
        Rec::Event { name: "level_restarted".into(), payload: None, doc: "The level restarted.".into() },
        Rec::Scratch(4096),
    ]
}

/// Checks `got` holds every expected record exactly once (extra records are fine).
pub fn assert_records(got: &[Rec]) {
    for want in expected_records() {
        let n = got.iter().filter(|g| **g == want).count();
        assert_eq!(n, 1, "record {want:?} found {n} times in {got:#?}");
    }
}
