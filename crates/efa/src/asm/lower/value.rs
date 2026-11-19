//! Immediate values and their typing (efa-syntax.md section 6).

use crate::asm::ast::{Operand, Ty};

/// An untyped immediate: an integer-like value (integers, characters, consts, data symbols) or
/// the source text of a float literal.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Val {
    Int(i128),
    Float(String),
}

/// A typed constant, ready to be emitted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Konst {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Konst {
    /// The zero of a type (used to keep module indices stable after an error).
    pub(super) fn zero(ty: Ty) -> Konst {
        match ty {
            Ty::I32 => Konst::I32(0),
            Ty::I64 => Konst::I64(0),
            Ty::F32 => Konst::F32(0.0),
            Ty::F64 => Konst::F64(0.0),
        }
    }
}

/// Types `v` for a slot of type `ty`. The error is the message text.
pub(super) fn type_val(ty: Ty, v: &Val) -> Result<Konst, String> {
    match (ty, v) {
        (Ty::I32, Val::Int(n)) => {
            if (-(1i128 << 31)..=(1i128 << 32) - 1).contains(n) {
                Ok(Konst::I32(*n as u32 as i32))
            } else {
                Err(format!("integer {n} does not fit in i32"))
            }
        }
        (Ty::I64, Val::Int(n)) => {
            if (-(1i128 << 63)..=(1i128 << 64) - 1).contains(n) {
                Ok(Konst::I64(*n as u64 as i64))
            } else {
                Err(format!("integer {n} does not fit in i64"))
            }
        }
        (Ty::I32 | Ty::I64, Val::Float(_)) => Err("float literal in an integer position".into()),
        (Ty::F32, Val::Int(n)) => Ok(Konst::F32(*n as f32)),
        (Ty::F64, Val::Int(n)) => Ok(Konst::F64(*n as f64)),
        (Ty::F32, Val::Float(t)) => parse_f32(t).map(Konst::F32),
        (Ty::F64, Val::Float(t)) => parse_f64(t).map(Konst::F64),
    }
}

enum Special<'a> {
    Inf(bool),
    Nan(bool),
    Bits(Option<u64>),
    Plain(&'a str),
}

fn special(text: &str) -> Special<'_> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, text),
    };
    if body == "inf" {
        Special::Inf(neg)
    } else if body == "nan" {
        Special::Nan(neg)
    } else if let Some(h) = body.strip_prefix("nan:0x") {
        if neg {
            Special::Bits(None)
        } else {
            Special::Bits(u64::from_str_radix(h, 16).ok())
        }
    } else {
        Special::Plain(text)
    }
}

fn parse_f32(text: &str) -> Result<f32, String> {
    match special(text) {
        Special::Inf(neg) => Ok(if neg { f32::NEG_INFINITY } else { f32::INFINITY }),
        Special::Nan(neg) => Ok(f32::from_bits(0x7fc0_0000 | (u32::from(neg) << 31))),
        Special::Bits(Some(b)) => match u32::try_from(b) {
            Ok(b) => Ok(f32::from_bits(b)),
            Err(_) => Err(format!("`{text}` does not fit in f32 (at most 8 hex digits)")),
        },
        Special::Bits(None) => Err(format!("invalid float literal `{text}`")),
        Special::Plain(t) => t.parse::<f32>().map_err(|_| format!("invalid float literal `{text}`")),
    }
}

pub(super) fn parse_f64(text: &str) -> Result<f64, String> {
    match special(text) {
        Special::Inf(neg) => Ok(if neg { f64::NEG_INFINITY } else { f64::INFINITY }),
        Special::Nan(neg) => Ok(f64::from_bits(0x7ff8_0000_0000_0000 | (u64::from(neg) << 63))),
        Special::Bits(Some(b)) => Ok(f64::from_bits(b)),
        Special::Bits(None) => Err(format!("invalid float literal `{text}`")),
        Special::Plain(t) => t.parse::<f64>().map_err(|_| format!("invalid float literal `{text}`")),
    }
}

/// How an operand reads in an error message ("expected a register, found 5").
pub(super) fn describe(op: &Operand) -> String {
    match op {
        Operand::Name(i) => format!("`{}`", i.name),
        Operand::Int(v, _) => v.to_string(),
        Operand::Float(t, _) => t.clone(),
        Operand::Char(c, _) => format!("{c:?}"),
        Operand::Str(..) => "a string".into(),
        Operand::Call(_) => "a call".into(),
        Operand::Labels(..) => "a label list".into(),
        Operand::Tuple(..) => "a tuple".into(),
        Operand::Signature(_) => "a signature".into(),
        Operand::Expr(_) => "a constant expression".into(),
    }
}

/// `r<digits>` (at least one digit) gives the register number; a number too large for `usize`
/// gives `usize::MAX` (rejected where the register is used).
pub(super) fn raw_reg(name: &str) -> Option<usize> {
    let digits = name.strip_prefix('r')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(digits.parse::<usize>().unwrap_or(usize::MAX))
}

/// The float text of a computed value: `{:?}` round-trips an `f64`; `inf` and `nan` are the
/// literal words.
pub(super) fn float_text(x: f64) -> String {
    if x.is_nan() {
        "nan".into()
    } else if x.is_infinite() {
        if x < 0.0 { "-inf".into() } else { "inf".into() }
    } else {
        format!("{x:?}")
    }
}
