//! Translation errors. Every error names the construct and, where it can, the wasm function
//! (index and `name` section name) and the byte offset in the `.wasm` file.

use std::fmt;

/// Where in the wasm module an error happened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loc {
    /// Wasm function index (imports first).
    pub func: Option<u32>,
    /// The function's name from the `name` section or an export, if there is one.
    pub name: Option<String>,
    /// Byte offset in the `.wasm` file.
    pub offset: Option<usize>,
}

impl Loc {
    pub(crate) fn at(offset: usize) -> Loc {
        Loc { offset: Some(offset), ..Loc::default() }
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(i) = self.func {
            match &self.name {
                Some(n) => parts.push(format!("function {i} `{n}`")),
                None => parts.push(format!("function {i}")),
            }
        }
        if let Some(o) = self.offset {
            parts.push(format!("byte {o}"));
        }
        if parts.is_empty() { Ok(()) } else { write!(f, " (in {})", parts.join(", at ")) }
    }
}

/// Why a `.wasm` module could not be translated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TranslateError {
    /// The bytes are not a valid wasm module.
    #[error("not a valid wasm module (byte {offset}): {message}")]
    Invalid { offset: usize, message: String },
    /// A valid wasm construct outside the supported subset (design doc, section 7).
    #[error("unsupported: {what}{at}")]
    Unsupported { what: String, at: Loc },
    /// A function needs more registers than efevm allows (no splitting).
    #[error("function too large: {what}{at}")]
    TooLarge { what: String, at: Loc },
    /// An import that does not follow the ABI (design doc, section 3).
    #[error("import `{module}::{name}`: {message}")]
    Import { module: String, name: String, message: String },
    /// An import module whose namespace is not in the API index.
    #[error("import module `{module}`: namespace `{namespace}` is not in the API index")]
    UnknownNamespace { namespace: String, module: String },
    /// `efe.api` and the index the translator was given disagree (build skew).
    #[error("the module was built against API index hash {module:016x}, but the index given has hash {index:016x}")]
    ApiHash { module: u64, index: u64 },
    /// A malformed `efe.api` or `efe.provide` section, or a wrong `efe_scratch`.
    #[error("custom section `{section}`: {message}")]
    Section { section: &'static str, message: String },
    /// A provide record that does not match the module.
    #[error("provide `{name}`: {message}")]
    Provide { name: String, message: String },
    /// The translated module does not verify (a translator bug or a module that breaks an
    /// efevm limit, for example a data segment outside the memory).
    #[error("the translated module does not verify: {0}")]
    Verify(String),
}

pub(crate) fn unsupported(what: impl Into<String>, at: Loc) -> TranslateError {
    TranslateError::Unsupported { what: what.into(), at }
}
