//! Error types for loading, linking, building and spawning.

use crate::binary::DecodeError;
use crate::op::Op;
use crate::verify::VerifyError;
use crate::trap::Trap;
use std::fmt;

/// Failure to turn bytes or a builder into a verified module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Decode { offset: u64, error: DecodeError },
    /// `func` is in the module-function index space (imports first).
    Verify { func: Option<u32>, inst: Option<u32>, error: VerifyError },
    Link(LinkError),
    Build(BuildError),
}

/// Why a module cannot be linked against an `Imports` (`Instance::new`,
/// `Imports::check_module`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    MissingImport { namespace: String, name: String },
    /// `module` is the signature the module imports, `host` the registered one, both in the
    /// index spelling (`(i64, i64) -> i32`).
    ImportTypeMismatch { namespace: String, name: String, module: String, host: String },
    MemoryTooLarge { min_pages: u32, cap: u32 },
    /// The module's `api` section names a namespace the host does not define.
    UnknownNamespace { namespace: String },
    /// The module was built against version `module` of `namespace`, the host has `host`.
    NamespaceVersion { namespace: String, module: u32, host: u32 },
    /// Not an error by itself: `Imports::check_module` appends it after real errors when the
    /// module's recorded index hash differs from the host's (`host` is `None` for imports not
    /// built by a `HostRegistry`). `Instance::new` never returns it.
    ApiHash { module: u64, host: Option<u64> },
    /// The module's section 14 refers to event `namespace.name`, which the host does not
    /// register.
    UnknownEvent { namespace: String, name: String },
    /// The module's section 14 records another payload for event `namespace.name` than the
    /// host registers. `module` and `host` are `&Name (N bytes)` or `no payload`.
    EventPayload { namespace: String, name: String, module: String, host: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    ImportAfterFunction,
    UnboundLabel,
    LabelBoundTwice,
    UndefinedFunction(u32),
    WrongShape(Op),
    /// More than 65535 registers requested.
    TooManyRegisters,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    NoSuchExport,
    NotAFunction,
    ImportEntry,
    ArgCount { expected: u32, got: u32 },
    TooManyFibers,
    StackOverflow,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Decode { offset, error } => write!(f, "decode error at byte {offset}: {error}"),
            LoadError::Verify { func, inst, error } => {
                write!(f, "verification failed")?;
                if let Some(func) = func {
                    write!(f, " in function {func}")?;
                }
                if let Some(inst) = inst {
                    write!(f, " at instruction {inst}")?;
                }
                write!(f, ": {error}")
            }
            LoadError::Link(e) => write!(f, "link error: {e}"),
            LoadError::Build(e) => write!(f, "build error: {e}"),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::MissingImport { namespace, name } => write!(f, "missing import {namespace}.{name}"),
            LinkError::ImportTypeMismatch { namespace, name, module, host } => {
                write!(f, "signature mismatch for {namespace}.{name}: module imports {module}, host has {host}")
            }
            LinkError::MemoryTooLarge { min_pages, cap } => {
                write!(f, "module needs {min_pages} memory pages, limit is {cap}")
            }
            LinkError::UnknownNamespace { namespace } => {
                write!(f, "module was built against namespace {namespace}, which the host does not define")
            }
            LinkError::NamespaceVersion { namespace, module, host } => {
                write!(f, "namespace {namespace} is version {host}, module was built against version {module}")
            }
            LinkError::ApiHash { module, host } => {
                write!(f, "module was built against API index fnv1a64:{module:016x}")?;
                match host {
                    Some(h) => write!(f, ", host index is fnv1a64:{h:016x}"),
                    None => f.write_str(", host imports have no index"),
                }
            }
            LinkError::UnknownEvent { namespace, name } => {
                write!(f, "module uses event {namespace}.{name}, which the host does not register")
            }
            LinkError::EventPayload { namespace, name, module, host } => {
                write!(f, "payload mismatch for event {namespace}.{name}: module expects {module}, host has {host}")
            }
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::ImportAfterFunction => f.write_str("import declared after a function"),
            BuildError::UnboundLabel => f.write_str("label used but never bound"),
            BuildError::LabelBoundTwice => f.write_str("label bound twice"),
            BuildError::UndefinedFunction(i) => write!(f, "function {i} declared but not defined"),
            BuildError::WrongShape(op) => write!(f, "wrong builder method for {}", op.name()),
            BuildError::TooManyRegisters => f.write_str("too many registers (limit is 65535)"),
        }
    }
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpawnError::NoSuchExport => f.write_str("no such export"),
            SpawnError::NotAFunction => f.write_str("export is not a function"),
            SpawnError::ImportEntry => f.write_str("cannot spawn an imported (host) function"),
            SpawnError::ArgCount { expected, got } => {
                write!(f, "expected {expected} arguments, got {got}")
            }
            SpawnError::TooManyFibers => f.write_str("too many fibers"),
            SpawnError::StackOverflow => f.write_str("register stack exhausted"),
        }
    }
}

impl std::error::Error for LoadError {}
impl std::error::Error for LinkError {}
impl std::error::Error for BuildError {}
impl std::error::Error for SpawnError {}

/// Why `Instance::call` did not return results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// The fiber could not be created.
    Spawn(SpawnError),
    /// The function trapped. `location` and `backtrace` are what `Instance::trap_location` and
    /// `Instance::backtrace` reported before the fiber was removed.
    Trap { trap: Trap, location: Option<(u32, u32)>, backtrace: Vec<(u32, u32)> },
    /// The budget ran out (including budget debt from bulk memory operations).
    OutOfBudget,
    /// The function yielded.
    Yield,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Spawn(e) => write!(f, "spawn failed: {e}"),
            CallError::Trap { trap, .. } => write!(f, "trap: {trap}"),
            CallError::OutOfBudget => f.write_str("out of budget"),
            CallError::Yield => f.write_str("function yielded"),
        }
    }
}

impl std::error::Error for CallError {}
