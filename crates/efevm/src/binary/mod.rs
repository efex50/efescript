//! Binary `.efvm` codec (SPEC.md section 9). Public entry points: `encode` and unverified `decode`.

pub(crate) mod leb;
pub(crate) mod encode;
pub(crate) mod decode;

use crate::error::LoadError;
use crate::module::ModuleData;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    UnexpectedEnd,
    BadMagic,
    BadVersion(u16),
    LebTooLong,
    SectionOutOfOrder(u8),
    UnknownSection(u8),
    SectionSizeMismatch(u8),
    BadUtf8,
    BadValType(u8),
    BadFlag(u8),
    BadExportKind(u8),
    UnknownOpcode(u64),
    CountTooLarge,
    FunctionCodeMismatch,
    RegisterTooLarge,
    ValueTooLarge,
    /// An `apitype` byte of section 13 that is not 0..=8 (`ptr` has no encoding).
    BadApiType(u8),
    /// A struct field `prim` byte of section 13 that is not 0..=11.
    BadPrim(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnexpectedEnd => f.write_str("unexpected end of input"),
            DecodeError::BadMagic => f.write_str("bad magic bytes"),
            DecodeError::BadVersion(v) => write!(f, "unsupported version {v}"),
            DecodeError::LebTooLong => f.write_str("LEB128 value too long"),
            DecodeError::SectionOutOfOrder(id) => write!(f, "section {id} out of order"),
            DecodeError::UnknownSection(id) => write!(f, "unknown section {id}"),
            DecodeError::SectionSizeMismatch(id) => write!(f, "section {id} size mismatch"),
            DecodeError::BadUtf8 => f.write_str("invalid UTF-8 string"),
            DecodeError::BadValType(b) => write!(f, "bad value type {b}"),
            DecodeError::BadFlag(b) => write!(f, "bad flag byte {b}"),
            DecodeError::BadExportKind(b) => write!(f, "bad export kind {b}"),
            DecodeError::UnknownOpcode(n) => write!(f, "unknown opcode {n}"),
            DecodeError::CountTooLarge => f.write_str("count exceeds remaining input"),
            DecodeError::FunctionCodeMismatch => f.write_str("functions and code counts differ"),
            DecodeError::RegisterTooLarge => f.write_str("register number too large"),
            DecodeError::ValueTooLarge => f.write_str("value too large"),
            DecodeError::BadApiType(b) => write!(f, "bad API type {b}"),
            DecodeError::BadPrim(b) => write!(f, "bad struct field type {b}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Encodes a module to `.efvm` bytes.
pub fn encode(m: &ModuleData) -> Vec<u8> {
    encode::encode(m)
}

/// Decodes bytes without verifying.
pub fn decode(bytes: &[u8]) -> Result<ModuleData, LoadError> {
    decode::decode(bytes)
}
