//! Where the files of a multi-file program come from (efa-syntax 16.1). The assembler never
//! touches the file system itself: it reads through a [`FileSource`].

use std::collections::BTreeMap;

/// Reads the files of a program by their path relative to the source's root, with `/` as the
/// separator and no `.` or `..` segments (the assembler normalises `use` paths first).
pub trait FileSource {
    /// The text of the file at `path`, or a message saying why it cannot be read.
    fn read(&self, path: &str) -> Result<String, String>;
    /// The raw bytes of the file at `path` (assets). The default reads the text.
    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read(path).map(String::into_bytes)
    }
    /// The name of the file at `path` in diagnostics and in the lines section. The default
    /// is the path itself.
    fn display_name(&self, path: &str) -> String {
        path.to_string()
    }
    /// Every file of the source (paths as `read` takes them, `/`-separated, any order), or
    /// `None` when the source cannot list itself. The engine hashes a mod folder from this
    /// list; a source that returns `None` makes every load of it count as changed.
    fn list(&self) -> Option<Vec<String>> {
        None
    }
}

/// Files held in memory, keyed by path (tests, language-server buffers, web tools).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemSource(pub BTreeMap<String, Vec<u8>>);

impl MemSource {
    pub fn new() -> MemSource {
        MemSource(BTreeMap::new())
    }

    /// Adds (or replaces) a file; returns `self` for chaining.
    pub fn with(mut self, path: &str, text: &str) -> MemSource {
        self.0.insert(path.to_string(), text.as_bytes().to_vec());
        self
    }

    /// Adds (or replaces) a binary file; returns `self` for chaining.
    pub fn with_bytes(mut self, path: &str, bytes: &[u8]) -> MemSource {
        self.0.insert(path.to_string(), bytes.to_vec());
        self
    }
}

impl FileSource for MemSource {
    fn read(&self, path: &str) -> Result<String, String> {
        let b = self.0.get(path).ok_or_else(|| "no such file".to_string())?;
        String::from_utf8(b.clone()).map_err(|_| "not valid UTF-8".to_string())
    }

    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        self.0.get(path).cloned().ok_or_else(|| "no such file".to_string())
    }

    fn list(&self) -> Option<Vec<String>> {
        Some(self.0.keys().cloned().collect())
    }
}

/// The largest source file [`FsSource`] reads.
#[cfg(not(target_family = "wasm"))]
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Files below a directory of the native file system. Paths are joined to the root,
/// canonicalised, and refused when they lead outside the root (also through a symbolic
/// link); only regular files up to [`MAX_FILE_BYTES`] are read.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug)]
pub struct FsSource {
    root: std::path::PathBuf,
    /// Prefix of the display names (the root directory as the user wrote it); empty for none.
    prefix: String,
}

#[cfg(not(target_family = "wasm"))]
impl FsSource {
    /// Files below `root`; display names are the paths relative to it.
    pub fn new(root: impl Into<std::path::PathBuf>) -> FsSource {
        FsSource { root: root.into(), prefix: String::new() }
    }

    /// Files below `root`; display names are `prefix` joined to the path.
    pub fn with_prefix(root: impl Into<std::path::PathBuf>, prefix: impl Into<String>) -> FsSource {
        FsSource { root: root.into(), prefix: prefix.into() }
    }

    /// The source rooted at the directory of `file` (a path as the user typed it), and the
    /// file's name relative to it. Display names keep the directory as typed, so the root
    /// file is shown (and recorded in the lines section) exactly as `file`.
    pub fn for_file(file: &str) -> (FsSource, String) {
        match file.rsplit_once('/') {
            Some((dir, name)) => {
                let dir = if dir.is_empty() { "/" } else { dir };
                (FsSource { root: dir.into(), prefix: dir.to_string() }, name.to_string())
            }
            None => (FsSource { root: ".".into(), prefix: String::new() }, file.to_string()),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
impl FsSource {
    fn read_raw(&self, path: &str) -> Result<Vec<u8>, String> {
        use std::path::{Component, Path};
        let rel = Path::new(path);
        if path.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err("the path must be relative and must not contain `.` or `..`".to_string());
        }
        let root = self.root.canonicalize().map_err(|e| format!("cannot open the root directory: {e}"))?;
        let full = root.join(rel).canonicalize().map_err(|e| e.to_string())?;
        if !full.starts_with(&root) {
            return Err("it leads outside the root directory (through a symbolic link)".to_string());
        }
        let md = std::fs::metadata(&full).map_err(|e| e.to_string())?;
        if !md.is_file() {
            return Err("not a regular file".to_string());
        }
        if md.len() > MAX_FILE_BYTES {
            return Err(format!("the file is larger than {MAX_FILE_BYTES} bytes"));
        }
        std::fs::read(&full).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_family = "wasm"))]
impl FileSource for FsSource {
    fn read(&self, path: &str) -> Result<String, String> {
        String::from_utf8(self.read_raw(path)?).map_err(|_| "not valid UTF-8".to_string())
    }

    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read_raw(path)
    }

    fn list(&self) -> Option<Vec<String>> {
        fn walk(dir: &std::path::Path, rel: &str, out: &mut Vec<String>) -> Option<()> {
            for e in std::fs::read_dir(dir).ok()? {
                let e = e.ok()?;
                let name = e.file_name().into_string().ok()?;
                let rel_name = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                let ty = e.file_type().ok()?;
                if ty.is_symlink() {
                    continue;
                } else if ty.is_dir() {
                    walk(&e.path(), &rel_name, out)?;
                } else if ty.is_file() {
                    out.push(rel_name);
                }
            }
            Some(())
        }
        let mut out = Vec::new();
        walk(&self.root, "", &mut out)?;
        Some(out)
    }

    fn display_name(&self, path: &str) -> String {
        if self.prefix.is_empty() {
            path.to_string()
        } else if self.prefix.ends_with('/') {
            format!("{}{path}", self.prefix)
        } else {
            format!("{}/{path}", self.prefix)
        }
    }
}
