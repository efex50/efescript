//! Value types, function types and embedder limits.

use std::fmt;

/// A value type. Registers are untyped 64-bit slots; types matter only at function boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ValType {
    I32 = 0,
    I64 = 1,
    F32 = 2,
    F64 = 3,
}

impl ValType {
    /// Decodes the binary-format byte.
    pub fn from_byte(b: u8) -> Option<ValType> {
        match b {
            0 => Some(ValType::I32),
            1 => Some(ValType::I64),
            2 => Some(ValType::F32),
            3 => Some(ValType::F64),
            _ => None,
        }
    }
}

/// A function type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

impl FuncType {
    pub fn new(
        params: impl IntoIterator<Item = ValType>,
        results: impl IntoIterator<Item = ValType>,
    ) -> Self {
        FuncType { params: params.into_iter().collect(), results: results.into_iter().collect() }
    }
}

impl ValType {
    /// `i32`, `i64`, `f32` or `f64`.
    pub fn name(self) -> &'static str {
        match self {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
        }
    }
}

impl fmt::Display for ValType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The lowered signature as written in the API index (API_INDEX.md, key `signature`):
/// `(i64, i32) -> i32`, `(f32) -> (i32, i64)` or `(i64, i64)` without results.
impl fmt::Display for FuncType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[ValType]| v.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ");
        write!(f, "({})", list(&self.params))?;
        match self.results.len() {
            0 => Ok(()),
            1 => write!(f, " -> {}", self.results[0]),
            _ => write!(f, " -> ({})", list(&self.results)),
        }
    }
}

/// Size of a memory page in bytes.
pub const PAGE_SIZE: u64 = 65536;

/// Resource limits chosen by the embedder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_memory_pages: u32,
    pub max_call_depth: u32,
    /// Register slots per fiber, all frames together.
    pub max_registers: u32,
    pub max_fibers: u32,
}

/// Defaults: 1024 pages (64 MiB), call depth 4096, 262144 registers per fiber, 256 fibers.
impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_memory_pages: 1024,
            max_call_depth: 4096,
            max_registers: 262_144,
            max_fibers: 256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let t = FuncType::new([ValType::I64, ValType::I64], []);
        assert_eq!(t.params.len(), 2);
        assert_eq!(ValType::from_byte(3), Some(ValType::F64));
        assert_eq!(ValType::from_byte(4), None);
        assert_eq!(Limits::default().max_memory_pages, 1024);
    }

    #[test]
    fn signature_text() {
        use ValType::*;
        assert_eq!(FuncType::new([I64, I32, I64], [I32]).to_string(), "(i64, i32, i64) -> i32");
        assert_eq!(FuncType::new([F32], [I32, I64]).to_string(), "(f32) -> (i32, i64)");
        assert_eq!(FuncType::new([], []).to_string(), "()");
    }
}
