//! The guest ABI of `crates/efescript/docs/e7.4-wasm-design.md`: wasm lowering of API types,
//! import resolution and the `efe.provide` records.

use crate::error::TranslateError;
use efevm::{ApiType, FuncType, ValType};

/// The wasm values an API type occupies on wasm32 (design doc, section 3.2).
pub(crate) fn wasm_lower(t: &ApiType) -> &'static [ValType] {
    match t {
        ApiType::I32 | ApiType::Bool | ApiType::Ptr | ApiType::Struct(_) => &[ValType::I32],
        ApiType::I64 | ApiType::Entity => &[ValType::I64],
        ApiType::F32 => &[ValType::F32],
        ApiType::F64 => &[ValType::F64],
        ApiType::Str | ApiType::Bytes => &[ValType::I32, ValType::I32],
    }
}

/// The wasm signature of a list of parameters and results.
pub(crate) fn wasm_sig(params: &[&ApiType], results: &[&ApiType]) -> FuncType {
    FuncType::new(
        params.iter().flat_map(|t| wasm_lower(t).iter().copied()),
        results.iter().flat_map(|t| wasm_lower(t).iter().copied()),
    )
}

/// Spelling of a list of API types for a message: `(str, i32)`.
pub(crate) fn api_text(ts: &[&ApiType]) -> String {
    format!("({})", ts.iter().map(|t| t.spelling()).collect::<Vec<_>>().join(", "))
}

/// A record of the `efe.provide` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rec {
    Func { name: String, export: String, doc: String, params: Vec<(String, ApiType)>, results: Vec<(String, ApiType)> },
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
    /// A LEB128 of at most 5 bytes; zero-padded forms are accepted.
    fn uleb(&mut self) -> Result<u32, String> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                return u32::try_from(v).map_err(|_| "uleb128 does not fit 32 bits".to_string());
            }
            if shift >= 35 {
                return Err("uleb128 longer than 5 bytes".into());
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
    fn ty(&mut self) -> Result<ApiType, String> {
        Ok(match self.u8()? {
            0 => ApiType::I32,
            1 => ApiType::I64,
            2 => ApiType::F32,
            3 => ApiType::F64,
            4 => ApiType::Bool,
            5 => ApiType::Entity,
            6 => ApiType::Str,
            7 => ApiType::Bytes,
            8 => ApiType::Struct(self.string()?),
            c => return Err(format!("unknown API type code {c}")),
        })
    }
    fn params(&mut self) -> Result<Vec<(String, ApiType)>, String> {
        let n = self.uleb()?;
        let mut v = Vec::new();
        for _ in 0..n {
            v.push((self.string()?, self.ty()?));
        }
        Ok(v)
    }
}

/// Decodes the (concatenated) `efe.provide` sections. Unknown record kinds (for example the
/// not yet defined kind 3, mod-defined structs) are skipped, as the ABI says.
pub(crate) fn decode_provide(bytes: &[u8]) -> Result<Vec<Rec>, TranslateError> {
    let bad = |message: String| TranslateError::Section { section: "efe.provide", message };
    let mut r = Rd(bytes);
    let mut out = Vec::new();
    while !r.0.is_empty() {
        let kind = r.u8().map_err(bad)?;
        let len = r.uleb().map_err(bad)? as usize;
        let body = r.bytes(len).map_err(bad)?;
        let mut b = Rd(body);
        let rec = (|| -> Result<Option<Rec>, String> {
            Ok(match kind {
                0 => {
                    let name = b.string()?;
                    let export = b.string()?;
                    let doc = b.string()?;
                    let params = b.params()?;
                    let results = b.params()?;
                    Some(Rec::Func { name, export, doc, params, results })
                }
                1 => {
                    let name = b.string()?;
                    let payload = match b.u8()? {
                        0 => None,
                        1 => Some(b.string()?),
                        f => return Err(format!("bad payload flag {f}")),
                    };
                    let doc = b.string()?;
                    Some(Rec::Event { name, payload, doc })
                }
                2 => Some(Rec::Scratch(b.uleb()?)),
                _ => None,
            })
        })()
        .map_err(|m| bad(format!("record of kind {kind}: {m}")))?;
        if rec.is_some() && !b.0.is_empty() {
            return Err(bad(format!("record of kind {kind} has {} trailing bytes", b.0.len())));
        }
        out.extend(rec);
    }
    Ok(out)
}
