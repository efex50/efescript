//! Standalone executables: a runner executable with a compiled module appended.
//!
//! Layout: `[runner bytes][module bytes][module length: u64 LE][magic "EFVMBNDL"]`.
//! The last 16 bytes are the trailer.

use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The last 8 bytes of a bundle.
pub const MAGIC: [u8; 8] = *b"EFVMBNDL";
const TRAILER: usize = 16;

#[derive(Debug)]
pub enum BundleError {
    Io(String),
    /// The magic is present but the stored length does not fit in the file.
    BadLength { len: u64, max: u64 },
    /// The output path is the runner itself.
    SameAsRunner(String),
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::Io(m) => f.write_str(m),
            BundleError::BadLength { len, max } => write!(
                f,
                "error: corrupt bundle trailer: module length {len} exceeds the {max} bytes available"
            ),
            BundleError::SameAsRunner(p) => {
                write!(f, "error: the output {p} is the runner itself; choose another -o")
            }
        }
    }
}

impl std::error::Error for BundleError {}

fn io_err(what: &str, e: std::io::Error) -> BundleError {
    BundleError::Io(format!("error: {what}: {e}"))
}

/// Interprets a 16-byte trailer for a file of `file_len` bytes.
fn check_trailer(trailer: &[u8; TRAILER], file_len: u64) -> Result<Option<u64>, BundleError> {
    if trailer[8..] != MAGIC {
        return Ok(None);
    }
    let mut l = [0u8; 8];
    l.copy_from_slice(&trailer[..8]);
    let len = u64::from_le_bytes(l);
    let max = file_len - TRAILER as u64;
    if len > max {
        return Err(BundleError::BadLength { len, max });
    }
    Ok(Some(len))
}

/// Finds the embedded module: `Ok(Some((start, len)))`, `Ok(None)` without a trailer magic,
/// `Err` when the magic is present with an impossible length.
pub fn find_payload(exe_bytes: &[u8]) -> Result<Option<(usize, usize)>, BundleError> {
    let n = exe_bytes.len();
    if n < TRAILER {
        return Ok(None);
    }
    let mut t = [0u8; TRAILER];
    t.copy_from_slice(&exe_bytes[n - TRAILER..]);
    Ok(check_trailer(&t, n as u64)?.map(|len| (n - TRAILER - len as usize, len as usize)))
}

/// The runner part without an existing payload and trailer (identity when there is none or
/// the trailer is corrupt).
pub fn strip_payload(exe_bytes: &[u8]) -> &[u8] {
    match find_payload(exe_bytes) {
        Ok(Some((start, _))) => &exe_bytes[..start],
        _ => exe_bytes,
    }
}

/// Builds a bundle; an existing payload of `runner` is dropped first.
pub fn make_bundle(runner: &[u8], module: &[u8]) -> Vec<u8> {
    let runner = strip_payload(runner);
    let mut out = Vec::with_capacity(runner.len() + module.len() + TRAILER);
    out.extend_from_slice(runner);
    out.extend_from_slice(module);
    out.extend_from_slice(&(module.len() as u64).to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out
}

/// Writes `runner + module + trailer` to `out` (mode 0o755 on Unix). Refuses to overwrite
/// the runner itself.
pub fn write_bundle(runner_path: &Path, module: &[u8], out: &Path) -> Result<(), BundleError> {
    let runner = std::fs::read(runner_path)
        .map_err(|e| io_err(&format!("{}", runner_path.display()), e))?;
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(runner_path), std::fs::canonicalize(out)) {
        if a == b {
            return Err(BundleError::SameAsRunner(out.display().to_string()));
        }
    }
    std::fs::write(out, make_bundle(&runner, module))
        .map_err(|e| io_err(&format!("{}", out.display()), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| io_err(&format!("{}", out.display()), e))?;
    }
    Ok(())
}

/// Reads the module embedded in the running executable, if any. Only the trailer is read
/// first, so a normal start does not read the whole file.
pub fn embedded_module_of_current_exe() -> Result<Option<Vec<u8>>, BundleError> {
    let exe = std::env::current_exe().map_err(|e| io_err("cannot locate the executable", e))?;
    let what = exe.display().to_string();
    let mut f = std::fs::File::open(&exe).map_err(|e| io_err(&what, e))?;
    let file_len = f.metadata().map_err(|e| io_err(&what, e))?.len();
    if file_len < TRAILER as u64 {
        return Ok(None);
    }
    let mut t = [0u8; TRAILER];
    f.seek(SeekFrom::Start(file_len - TRAILER as u64)).map_err(|e| io_err(&what, e))?;
    f.read_exact(&mut t).map_err(|e| io_err(&what, e))?;
    let Some(len) = check_trailer(&t, file_len)? else { return Ok(None) };
    let start = file_len - TRAILER as u64 - len;
    f.seek(SeekFrom::Start(start)).map_err(|e| io_err(&what, e))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).map_err(|e| io_err(&what, e))?;
    Ok(Some(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trailer(len: u64) -> Vec<u8> {
        let mut v = len.to_le_bytes().to_vec();
        v.extend_from_slice(&MAGIC);
        v
    }

    #[test]
    fn round_trip() {
        let b = make_bundle(b"RUNNER", b"module");
        assert_eq!(b.len(), 6 + 6 + 16);
        assert_eq!(find_payload(&b).unwrap(), Some((6, 6)));
        assert_eq!(strip_payload(&b), b"RUNNER");
        assert_eq!(&b[6..12], b"module");
    }

    #[test]
    fn empty_module() {
        let b = make_bundle(b"RUN", b"");
        assert_eq!(b.len(), 3 + 16);
        assert_eq!(find_payload(&b).unwrap(), Some((3, 0)));
        assert_eq!(strip_payload(&b), b"RUN");
    }

    #[test]
    fn empty_runner() {
        let b = make_bundle(b"", b"m");
        assert_eq!(find_payload(&b).unwrap(), Some((0, 1)));
        assert_eq!(strip_payload(&b), b"");
    }

    #[test]
    fn short_files() {
        for n in 0..16 {
            let v = vec![b'x'; n];
            assert_eq!(find_payload(&v).unwrap(), None);
            assert_eq!(strip_payload(&v), &v[..]);
        }
        // exactly a trailer with length 0
        let t = trailer(0);
        assert_eq!(find_payload(&t).unwrap(), Some((0, 0)));
        assert_eq!(strip_payload(&t), b"");
    }

    #[test]
    fn no_magic() {
        let v = vec![0u8; 100];
        assert_eq!(find_payload(&v).unwrap(), None);
        assert_eq!(strip_payload(&v), &v[..]);
    }

    #[test]
    fn magic_with_bad_length() {
        let mut v = b"some runner".to_vec();
        v.extend(trailer(1000));
        assert!(matches!(find_payload(&v), Err(BundleError::BadLength { .. })));
        assert_eq!(strip_payload(&v), &v[..]);
        let mut w = b"r".to_vec();
        w.extend(trailer(u64::MAX));
        assert!(find_payload(&w).is_err());
        // length one more than available
        let mut x = b"abcd".to_vec();
        x.extend(trailer(5));
        assert!(find_payload(&x).is_err());
        let mut y = b"abcd".to_vec();
        y.extend(trailer(4));
        assert_eq!(find_payload(&y).unwrap(), Some((0, 4)));
    }

    #[test]
    fn rebundle_replaces_payload() {
        let first = make_bundle(b"RUNNER", b"old module");
        let second = make_bundle(&first, b"new");
        assert_eq!(second.len(), 6 + 3 + 16);
        assert_eq!(find_payload(&second).unwrap(), Some((6, 3)));
        assert_eq!(&second[6..9], b"new");
    }

    #[test]
    fn corrupt_runner_is_kept_whole() {
        let mut bad = b"RUN".to_vec();
        bad.extend(trailer(99));
        let b = make_bundle(&bad, b"m");
        assert_eq!(&b[..bad.len()], &bad[..]);
        assert_eq!(b.len(), bad.len() + 1 + 16);
    }
}
