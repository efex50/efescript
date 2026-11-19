//! Multi-file support of the language server: the file source that prefers open buffers,
//! the program root of a file (`mod.toml`), and path / URI helpers.

use crate::asm::{FileSource, FsSource};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// Lexical normalisation: `.` dropped, `..` pops (no file system access).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A file source for the files below `base`: an open buffer wins over the disk. Display
/// names are the full paths, so a diagnostic's file maps back to a URI.
pub struct BufSource<'a> {
    pub base: PathBuf,
    pub bufs: &'a HashMap<PathBuf, &'a str>,
}

impl FileSource for BufSource<'_> {
    fn read(&self, path: &str) -> Result<String, String> {
        let full = normalize(&self.base.join(path));
        if let Some(t) = self.bufs.get(&full) {
            return Ok((*t).to_string());
        }
        FsSource::new(self.base.clone()).read(path)
    }

    fn display_name(&self, path: &str) -> String {
        normalize(&self.base.join(path)).to_string_lossy().into_owned()
    }
}

/// `[build] entry` of the text of a `mod.toml` (default `src/main.efa`). Only this key is
/// read: a line-based reader is enough and keeps the crate free of a TOML dependency.
pub fn manifest_entry(text: &str) -> String {
    let mut in_build = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(sec) = line.strip_prefix('[') {
            in_build = sec.trim_end_matches(']').trim() == "build";
            continue;
        }
        if !in_build {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == "entry" {
                let v = v.trim();
                for q in ['"', '\''] {
                    if let Some(inner) = v.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
                        return inner.to_string();
                    }
                }
            }
        }
    }
    "src/main.efa".to_string()
}

/// The root file of the program `file` belongs to: the entry of the nearest `mod.toml`
/// walking up from the file when the file lies in the entry's directory, else the file itself.
pub fn program_root(file: &Path) -> PathBuf {
    let file = normalize(file);
    let mut dir = file.parent();
    while let Some(d) = dir {
        let manifest = d.join("mod.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).unwrap_or_default();
            let entry = normalize(&d.join(manifest_entry(&text)));
            if entry.parent().is_some_and(|ed| file.starts_with(ed)) {
                return entry;
            }
            return file;
        }
        dir = d.parent();
    }
    file
}

fn hex(b: u8) -> u8 {
    b"0123456789ABCDEF"[b as usize & 15]
}

/// `/a/b c.efa` to `file:///a/b%20c.efa`.
pub fn path_to_uri(p: &Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::from("file://");
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex(b >> 4) as char);
            out.push(hex(b) as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_reading() {
        assert_eq!(manifest_entry("[mod]\nid = \"a\"\n"), "src/main.efa");
        assert_eq!(manifest_entry("[mod]\nentry = \"no.efa\"\n[build]\nentry = \"game/x.efa\" # c\n"), "game/x.efa");
        assert_eq!(manifest_entry("[build]\nentry='y.efa'\n"), "y.efa");
    }

    #[test]
    fn normalising() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        assert_eq!(path_to_uri(Path::new("/a b/c.efa")), "file:///a%20b/c.efa");
    }
}
