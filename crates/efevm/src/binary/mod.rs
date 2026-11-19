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
    /// The header checksum does not match the CRC-32 of the bytes after the header.
    BadChecksum { stored: u32, computed: u32 },
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
            DecodeError::BadChecksum { stored, computed } => {
                write!(f, "checksum mismatch (stored {stored:#010x}, computed {computed:#010x}): the module is corrupt")
            }
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

/// The four magic bytes every `.efvm` module starts with.
pub const MAGIC: [u8; 4] = *b"EFVM";
/// The binary format version this crate reads and writes (SPEC.md 9).
pub const VERSION: u16 = 2;
/// Header length: magic, version and checksum. Sections start at this offset.
pub const HEADER_LEN: usize = 10;

/// CRC-32 (IEEE) of the bytes after the header, the value stored at offset 6.
pub fn checksum(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes.get(HEADER_LEN..).unwrap_or(&[]))
}

/// Recomputes and stores the header checksum of `bytes` in place. Does nothing when `bytes`
/// is shorter than the header. For tools and tests that build or patch `.efvm` bytes by hand;
/// [`encode`] already seals its output.
pub fn seal(bytes: &mut [u8]) {
    let sum = checksum(bytes);
    if let Some(slot) = bytes.get_mut(6..HEADER_LEN) {
        slot.copy_from_slice(&sum.to_le_bytes());
    }
}

/// Encodes a module to `.efvm` bytes.
pub fn encode(m: &ModuleData) -> Vec<u8> {
    encode::encode(m)
}

/// Decodes bytes without verifying the module (the header checksum is checked).
pub fn decode(bytes: &[u8]) -> Result<ModuleData, LoadError> {
    decode::decode(bytes)
}
