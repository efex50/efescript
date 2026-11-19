//! SemVer versions and version requirements, as specified in
//! `crates/efevm/MOD_PACKAGE.md` section 5.
//!
//! Matching follows Cargo's semantics (the `semver` crate's `eval.rs`), restricted to
//! the forms the format allows: no build metadata and no wildcards.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// Maximum length of a version string in bytes (section 5.1).
const MAX_VERSION_LEN: usize = 64;
/// Maximum number of comparators in a requirement (section 5.2).
const MAX_COMPARATORS: usize = 4;

/// Error returned when a version or a requirement does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionError(pub String);

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for VersionError {}

fn err<T>(message: impl Into<String>) -> Result<T, VersionError> {
    Err(VersionError(message.into()))
}

/// One dot-separated identifier of a pre-release.
///
/// A numeric identifier is stored as `u64`; a numeric identifier larger than `u64::MAX`
/// is rejected by the parser (a deliberate restriction of this format).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreId {
    /// A purely numeric identifier. Compares below every alphanumeric identifier.
    Num(u64),
    /// An identifier with at least one letter or hyphen. Compares by ASCII bytes.
    Alpha(String),
}

impl fmt::Display for PreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreId::Num(n) => write!(f, "{n}"),
            PreId::Alpha(s) => f.write_str(s),
        }
    }
}

/// A SemVer version: `MAJOR.MINOR.PATCH` with an optional pre-release.
///
/// Build metadata (`+...`) is not supported. Ordering is SemVer precedence.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pre: Vec<PreId>,
}

impl Version {
    /// A release version `major.minor.patch` without a pre-release.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version { major, minor, patch, pre: Vec::new() }
    }

    /// The pre-release identifiers; empty for a release version.
    pub fn pre(&self) -> &[PreId] {
        &self.pre
    }
}

/// SemVer precedence of two pre-release lists: a release (empty list) is greater than any
/// pre-release, otherwise the identifiers compare left to right.
fn cmp_pre(a: &[PreId], b: &[PreId]) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.cmp(b),
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| cmp_pre(&self.pre, &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            write!(f, "{id}")?;
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, VersionError> {
        if s.len() > MAX_VERSION_LEN {
            return err(format!("version is longer than {MAX_VERSION_LEN} bytes"));
        }
        if s.contains('+') {
            return err("build metadata (`+...`) is not supported in versions");
        }
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let major = version_number(parts.next(), "major")?;
        let minor = version_number(parts.next(), "minor")?;
        let patch = version_number(parts.next(), "patch")?;
        if parts.next().is_some() {
            return err("version has more than three numbers (MAJOR.MINOR.PATCH)");
        }
        let pre = match pre {
            Some(pre) => parse_pre(pre)?,
            None => Vec::new(),
        };
        Ok(Version { major, minor, patch, pre })
    }
}

/// Parses one required numeric component of a version.
fn version_number(part: Option<&str>, what: &str) -> Result<u32, VersionError> {
    match part {
        None => err(format!("missing {what} number (expected MAJOR.MINOR.PATCH)")),
        Some(part) => number(part, what),
    }
}

/// Parses a decimal `u32` without leading zeros.
fn number(part: &str, what: &str) -> Result<u32, VersionError> {
    if part.is_empty() {
        return err(format!("empty {what} number"));
    }
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return err(format!("{what} number must contain only digits"));
    }
    if part.len() > 1 && part.starts_with('0') {
        return err(format!("{what} number has a leading zero"));
    }
    part.parse::<u32>()
        .map_err(|_| VersionError(format!("{what} number is larger than {}", u32::MAX)))
}

/// Parses a dot-separated pre-release list (without the leading `-`).
fn parse_pre(pre: &str) -> Result<Vec<PreId>, VersionError> {
    let mut ids = Vec::new();
    for id in pre.split('.') {
        if id.is_empty() {
            return err("empty pre-release identifier");
        }
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return err("pre-release identifiers may contain only 0-9, A-Z, a-z and `-`");
        }
        if id.bytes().all(|b| b.is_ascii_digit()) {
            if id.len() > 1 && id.starts_with('0') {
                return err("numeric pre-release identifier has a leading zero");
            }
            match id.parse::<u64>() {
                Ok(n) => ids.push(PreId::Num(n)),
                Err(_) => {
                    return err("numeric pre-release identifier is larger than 18446744073709551615");
                }
            }
        } else {
            ids.push(PreId::Alpha(id.to_owned()));
        }
    }
    Ok(ids)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    Exact,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    Tilde,
    Caret,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Exact => "=",
            Op::Greater => ">",
            Op::GreaterEq => ">=",
            Op::Less => "<",
            Op::LessEq => "<=",
            Op::Tilde => "~",
            Op::Caret => "^",
        }
    }
}

/// One comparator of a requirement. `minor` and `patch` are `None` when omitted; `pre`
/// is only non-empty when all three numbers are given.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Comparator {
    op: Op,
    major: u32,
    minor: Option<u32>,
    patch: Option<u32>,
    pre: Vec<PreId>,
}

/// A version requirement (`req` in section 5.2): one or more comparators, all of which
/// must match. The requirement `*` has no comparators and matches every release version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VersionReq {
    comparators: Vec<Comparator>,
}

impl VersionReq {
    /// Whether `v` satisfies every comparator, under the pre-release rule of section 5.2:
    /// a pre-release version matches only if some comparator names the same
    /// `MAJOR.MINOR.PATCH` with a pre-release.
    pub fn matches(&self, v: &Version) -> bool {
        if !self.comparators.iter().all(|c| c.matches(v)) {
            return false;
        }
        if v.pre.is_empty() {
            return true;
        }
        self.comparators.iter().any(|c| c.pre_is_compatible(v))
    }
}

impl FromStr for VersionReq {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, VersionError> {
        if s.contains('+') {
            return err("build metadata (`+...`) is not supported in version requirements");
        }
        let text = s.trim_matches(' ');
        if text.is_empty() {
            return err("empty version requirement");
        }
        if text == "*" {
            return Ok(VersionReq { comparators: Vec::new() });
        }
        let mut comparators = Vec::new();
        for piece in text.split(',') {
            if comparators.len() == MAX_COMPARATORS {
                return err(format!("a requirement has at most {MAX_COMPARATORS} comparators"));
            }
            comparators.push(parse_comparator(piece)?);
        }
        Ok(VersionReq { comparators })
    }
}

fn parse_comparator(piece: &str) -> Result<Comparator, VersionError> {
    let text = piece.trim_matches(' ');
    let (op, rest) = if let Some(r) = text.strip_prefix(">=") {
        (Op::GreaterEq, r)
    } else if let Some(r) = text.strip_prefix("<=") {
        (Op::LessEq, r)
    } else if let Some(r) = text.strip_prefix('>') {
        (Op::Greater, r)
    } else if let Some(r) = text.strip_prefix('<') {
        (Op::Less, r)
    } else if let Some(r) = text.strip_prefix('=') {
        (Op::Exact, r)
    } else if let Some(r) = text.strip_prefix('^') {
        (Op::Caret, r)
    } else if let Some(r) = text.strip_prefix('~') {
        (Op::Tilde, r)
    } else {
        (Op::Caret, text)
    };
    let rest = rest.trim_start_matches(' ');
    if rest.is_empty() {
        return if text.is_empty() {
            err("empty comparator (stray comma?)")
        } else {
            err("an operator must be followed by a version")
        };
    }

    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let mut parts = core.split('.');
    let major = range_number(parts.next(), "major", true)?.unwrap_or(0);
    let minor = range_number(parts.next(), "minor", false)?;
    let patch = range_number(parts.next(), "patch", false)?;
    if parts.next().is_some() {
        return err("version has more than three numbers (MAJOR.MINOR.PATCH)");
    }
    let pre = match pre {
        None => Vec::new(),
        Some(pre) => {
            if patch.is_none() {
                return err("a pre-release needs all of MAJOR.MINOR.PATCH");
            }
            parse_pre(pre)?
        }
    };
    Ok(Comparator { op, major, minor, patch, pre })
}

/// Parses an optional numeric component of a requirement. Wildcards are rejected here.
fn range_number(part: Option<&str>, what: &str, required: bool) -> Result<Option<u32>, VersionError> {
    match part {
        None if required => err(format!("missing {what} number")),
        None => Ok(None),
        Some("*" | "x" | "X") => err(
            "wildcards (`*`, `x`) are not supported in version requirements; write `^1` or `=1`",
        ),
        Some(part) => number(part, what).map(Some),
    }
}

impl Comparator {
    fn matches(&self, v: &Version) -> bool {
        match self.op {
            Op::Exact => self.matches_exact(v),
            Op::Greater => self.matches_greater(v),
            Op::GreaterEq => self.matches_exact(v) || self.matches_greater(v),
            Op::Less => self.matches_less(v),
            Op::LessEq => self.matches_exact(v) || self.matches_less(v),
            Op::Tilde => self.matches_tilde(v),
            Op::Caret => self.matches_caret(v),
        }
    }

    fn pre_is_compatible(&self, v: &Version) -> bool {
        self.major == v.major
            && self.minor == Some(v.minor)
            && self.patch == Some(v.patch)
            && !self.pre.is_empty()
    }

    fn matches_exact(&self, v: &Version) -> bool {
        if v.major != self.major {
            return false;
        }
        if let Some(minor) = self.minor {
            if v.minor != minor {
                return false;
            }
        }
        if let Some(patch) = self.patch {
            if v.patch != patch {
                return false;
            }
        }
        v.pre == self.pre
    }

    fn matches_greater(&self, v: &Version) -> bool {
        if v.major != self.major {
            return v.major > self.major;
        }
        match self.minor {
            None => return false,
            Some(minor) => {
                if v.minor != minor {
                    return v.minor > minor;
                }
            }
        }
        match self.patch {
            None => return false,
            Some(patch) => {
                if v.patch != patch {
                    return v.patch > patch;
                }
            }
        }
        cmp_pre(&v.pre, &self.pre) == Ordering::Greater
    }

    fn matches_less(&self, v: &Version) -> bool {
        if v.major != self.major {
            return v.major < self.major;
        }
        match self.minor {
            None => return false,
            Some(minor) => {
                if v.minor != minor {
                    return v.minor < minor;
                }
            }
        }
        match self.patch {
            None => return false,
            Some(patch) => {
                if v.patch != patch {
                    return v.patch < patch;
                }
            }
        }
        cmp_pre(&v.pre, &self.pre) == Ordering::Less
    }

    fn matches_tilde(&self, v: &Version) -> bool {
        if v.major != self.major {
            return false;
        }
        if let Some(minor) = self.minor {
            if v.minor != minor {
                return false;
            }
        }
        if let Some(patch) = self.patch {
            if v.patch != patch {
                return v.patch > patch;
            }
        }
        cmp_pre(&v.pre, &self.pre) != Ordering::Less
    }

    fn matches_caret(&self, v: &Version) -> bool {
        if v.major != self.major {
            return false;
        }
        let Some(minor) = self.minor else {
            return true;
        };
        let Some(patch) = self.patch else {
            return if self.major > 0 { v.minor >= minor } else { v.minor == minor };
        };
        if self.major > 0 {
            if v.minor != minor {
                return v.minor > minor;
            } else if v.patch != patch {
                return v.patch > patch;
            }
        } else if minor > 0 {
            if v.minor != minor {
                return false;
            } else if v.patch != patch {
                return v.patch > patch;
            }
        } else if v.minor != minor || v.patch != patch {
            return false;
        }
        cmp_pre(&v.pre, &self.pre) != Ordering::Less
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.comparators.is_empty() {
            return f.write_str("*");
        }
        for (i, c) in self.comparators.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(c.op.symbol())?;
            write!(f, "{}", c.major)?;
            if let Some(minor) = c.minor {
                write!(f, ".{minor}")?;
            }
            if let Some(patch) = c.patch {
                write!(f, ".{patch}")?;
            }
            for (j, id) in c.pre.iter().enumerate() {
                f.write_str(if j == 0 { "-" } else { "." })?;
                write!(f, "{id}")?;
            }
        }
        Ok(())
    }
}
