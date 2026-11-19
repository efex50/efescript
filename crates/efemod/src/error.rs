//! Package errors (MOD_PACKAGE.md section 7).

use std::fmt;

/// Why a package was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// A size limit was exceeded (`what` names the package or the entry).
    TooLarge { what: String, size: u64, limit: u64 },
    /// The archive is outside the 7z profile (section 1) or malformed.
    Container(String),
    /// An entry name breaks the name rules.
    BadEntryName { name: String, reason: String },
    /// A required entry is missing.
    Missing(&'static str),
    /// An entry's CRC-32 does not match.
    Crc { name: String },
    /// `manifest.json` is invalid (message carries the key path).
    Manifest(String),
    /// `module.efvm` failed to decode or verify.
    Module(efevm::LoadError),
    /// The module imports namespaces the manifest does not declare.
    UndeclaredImport { namespaces: Vec<String> },
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::TooLarge { what, size, limit } => {
                write!(f, "{what} is too large: {size} bytes (limit {limit})")
            }
            PackageError::Container(m) => write!(f, "bad package container: {m}"),
            PackageError::BadEntryName { name, reason } => {
                write!(f, "bad entry name {name:?}: {reason}")
            }
            PackageError::Missing(n) => write!(f, "package has no `{n}`"),
            PackageError::Crc { name } => write!(f, "CRC-32 mismatch in `{name}`"),
            PackageError::Manifest(m) => write!(f, "bad manifest: {m}"),
            PackageError::Module(e) => write!(f, "bad module: {e}"),
            PackageError::UndeclaredImport { namespaces } => {
                write!(f, "module imports undeclared namespaces: {}", namespaces.join(", "))
            }
        }
    }
}

impl std::error::Error for PackageError {}
