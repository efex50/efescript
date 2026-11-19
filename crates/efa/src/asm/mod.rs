//! The efescript v2 assembler: `.efa` text to an `efevm` module.
//!
//! The language is specified in `crates/efescript/docs/efa-syntax.md` (built on
//! `docs/e2-design.md`; multi-file programs: section 16 and `docs/e8-design.md` section 3).
//! Pipeline: every file of the use graph goes through [`lexer::lex`] and `parser::parse`;
//! when no file has a syntax error, one [`Unit`] per file is linked by [`crate::link::link`].
//! Diagnostics are sorted by (file order, line, column) and capped at 100.

pub mod ast;
pub mod diag;
pub mod lexer;
mod lower;
mod parser;
pub mod source;

use std::collections::HashMap;

pub use diag::{Diagnostic, Diagnostics};
pub use lower::Unit;
#[cfg(not(target_family = "wasm"))]
pub use source::FsSource;
pub use source::{FileSource, MemSource};

fn sort_diags(d: &mut [Diagnostic]) {
    d.sort_by_key(|x| (x.line, x.column));
}

/// At most this many diagnostics are returned; the last one says that more were found.
const MAX_DIAGS: usize = 100;

fn cap_diags(d: &mut Vec<Diagnostic>) {
    if d.len() > MAX_DIAGS {
        let file = d.first().map(|x| x.file.clone()).unwrap_or_default();
        d.truncate(MAX_DIAGS - 1);
        d.push(Diagnostic::without_location(
            &file,
            format!("too many errors; stopping after {MAX_DIAGS}"),
        ));
    }
}

/// Parse only (for tools). Returns the lexer and parser diagnostics, sorted by position, on error.
pub fn parse(src: &str, file: &str) -> Result<ast::Module, Diagnostics> {
    let (module, mut diags) = parse_file(src, file, None);
    if diags.is_empty() {
        Ok(module)
    } else {
        sort_diags(&mut diags);
        cap_diags(&mut diags);
        Err(Diagnostics(diags))
    }
}

/// The first token that nests deeper than `max` brackets / braces, as a diagnostic. This is a
/// flat scan over the tokens, so it runs before any recursive parsing.
fn check_nesting(tokens: &[lexer::Token], src: &str, file: &str, max: usize) -> Option<Diagnostic> {
    use lexer::TokenKind as K;
    let mut depth = 0usize;
    for t in tokens {
        match t.kind {
            K::LBrace | K::LParen | K::LBracket => {
                depth += 1;
                if depth > max {
                    return Some(Diagnostic::new(
                        file,
                        src,
                        t.span,
                        format!("nesting is deeper than the limit of {max}"),
                    ));
                }
            }
            K::RBrace | K::RParen | K::RBracket => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn parse_file(src: &str, file: &str, max_nesting: Option<usize>) -> (ast::Module, Vec<Diagnostic>) {
    let (tokens, mut diags) = lexer::lex(src, file);
    if let Some(d) = max_nesting.and_then(|m| check_nesting(&tokens, src, file, m)) {
        diags.push(d);
        return (ast::Module::default(), diags);
    }
    let (module, parse_diags) = parser::parse(&tokens, src, file);
    diags.extend(parse_diags);
    (module, diags)
}

/// Options of [`assemble_with`] and [`assemble_program`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AsmOptions<'a> {
    /// The API index to assemble against (`--api FILE.efapi`). With it, every import must
    /// exist in the index with a matching signature, `import ns.name` may omit the signature,
    /// struct constants (`S.size`, `S.align`, `S.<field>`, `S.id`) are defined, and the module
    /// gets an `api` section (index hash and namespace versions). Without it the assembler
    /// accepts any import and writes no `api` section.
    pub api: Option<&'a efevm::api::ApiIndex>,
    /// Longest chain of `use` from the root (root = depth 0). `None` = unlimited.
    pub max_use_depth: Option<usize>,
    /// Deepest nesting of brackets / blocks in one file. `None` = unlimited.
    pub max_nesting: Option<usize>,
}

/// Assemble `.efa` source to a verified module. `file` is used in diagnostics and the lines section.
pub fn assemble(src: &str, file: &str) -> Result<efevm::Module, Diagnostics> {
    assemble_with(src, file, &AsmOptions::default())
}

/// [`assemble`] with options (an API index). See [`AsmOptions`]. This is a program of one
/// in-memory file: a `use` in it finds no file.
pub fn assemble_with(src: &str, file: &str, opts: &AsmOptions) -> Result<efevm::Module, Diagnostics> {
    let roots = [Root { path: file.to_string(), name: file.to_string(), text: Some(src.to_string()) }];
    program(&roots, &MemSource::new(), opts)
}

/// Assembles the program whose root file is `root` (a path in `src`), following `use`
/// (efa-syntax 16). The root's name in diagnostics and the lines section is
/// `src.display_name(root)`; the other files are named by their normalised path.
pub fn assemble_program(root: &str, src: &dyn FileSource, opts: &AsmOptions) -> Result<efevm::Module, Diagnostics> {
    assemble_programs(&[root], src, opts)
}

/// Like [`assemble_program`] with several roots linked into one module. The first is the
/// root of the program (its directory is the root directory, only it may hold `memory` and
/// `scratch`); the others are treated as if the first used them: they must lie inside the
/// root directory and get a namespace from their path.
pub fn assemble_programs(roots: &[&str], src: &dyn FileSource, opts: &AsmOptions) -> Result<efevm::Module, Diagnostics> {
    let roots: Vec<Root> =
        roots.iter().map(|r| Root { path: r.to_string(), name: src.display_name(r), text: None }).collect();
    program(&roots, src, opts)
}

/// A root file: its path in the source, its display name, and its text when it is not read
/// from the source.
struct Root {
    path: String,
    name: String,
    text: Option<String>,
}

/// One parsed file of the program.
struct File {
    name: String,
    namespace: String,
    text: String,
    module: ast::Module,
    /// The key (normalised path) of every `use` item, in source order.
    use_keys: Vec<Option<String>>,
}

/// Lexical normalisation of a `/`-separated relative path: `.` dropped, `..` pops. `None`
/// when it is absolute, has an empty segment, a `\`, or leaves the source root.
fn normalize(path: &str) -> Option<Vec<String>> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" => return None,
            "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s.to_string()),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The namespace of a file from its segments relative to the root directory (16.4).
fn namespace_of(rel: &[String]) -> Result<String, String> {
    let mut segs: Vec<&str> = rel.iter().map(String::as_str).collect();
    if let Some(last) = segs.last_mut() {
        *last = last.strip_suffix(".efa").unwrap_or(last);
    }
    match segs.iter().find(|s| !efevm::api::types::is_ident(s)) {
        Some(bad) => Err(format!(
            "`{bad}` is not an identifier: every segment of a used file's path (without `.efa`) must be one, because it becomes the file's namespace"
        )),
        None => Ok(segs.join(".")),
    }
}

/// Resolves the text of a `use` in the file with segments `from` (its directory is the
/// base) to the target's segments, checking the rules of 16.1.
fn resolve_use(path: &str, from: Option<&[String]>, root_dir: Option<&[String]>) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Err("the path is empty".into());
    }
    if path.starts_with('/') {
        return Err(format!("`{path}` is absolute; `use` paths are relative to the file that contains them"));
    }
    if path.contains('\\') {
        return Err(format!("`{path}` contains `\\`; use `/` as the separator"));
    }
    if !path.ends_with(".efa") {
        return Err(format!("`{path}` does not end in `.efa`"));
    }
    if path.split('/').any(str::is_empty) {
        return Err(format!("`{path}` has an empty path segment"));
    }
    let (Some(from), Some(root_dir)) = (from, root_dir) else {
        return Err("the root file's path cannot be used as the base of relative paths".into());
    };
    let mut segs: Vec<String> = from.get(..from.len().saturating_sub(1)).unwrap_or(&[]).to_vec();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                if segs.pop().is_none() {
                    return Err(format!("`{path}` leaves the root directory"));
                }
            }
            s => segs.push(s.to_string()),
        }
    }
    if !segs.starts_with(root_dir) || segs.len() == root_dir.len() {
        return Err(format!("`{path}` leaves the root directory"));
    }
    Ok(segs)
}

fn program(roots: &[Root], src: &dyn FileSource, opts: &AsmOptions) -> Result<efevm::Module, Diagnostics> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut files: Vec<File> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let Some(first) = roots.first() else {
        return Err(Diagnostics(vec![Diagnostic::without_location("", "no root file")]));
    };
    let root_segs = normalize(&first.path);
    let root_dir: Option<Vec<String>> = root_segs.as_ref().map(|s| s.get(..s.len() - 1).unwrap_or(&[]).to_vec());

    /// A file to visit: key, segments, display name, namespace, text (roots of `assemble_with`),
    /// and the `use` that reached it (file index, span) for read errors.
    struct Visit {
        key: String,
        segs: Option<Vec<String>>,
        name: String,
        namespace: String,
        text: Option<String>,
        from: Option<(usize, ast::Span)>,
        /// Length of the `use` chain from the root.
        depth: usize,
    }
    let mut stack: Vec<Visit> = Vec::new();
    for (k, r) in roots.iter().enumerate().rev() {
        let segs = normalize(&r.path);
        let namespace = if k == 0 {
            String::new()
        } else {
            let rel = match (&segs, &root_dir) {
                (Some(s), Some(d)) if s.starts_with(d) && s.len() > d.len() => Some(s[d.len()..].to_vec()),
                _ => None,
            };
            match rel.map(|r| namespace_of(&r)) {
                Some(Ok(ns)) => ns,
                Some(Err(m)) => {
                    diags.push(Diagnostic::without_location(&r.name, m));
                    continue;
                }
                None => {
                    diags.push(Diagnostic::without_location(
                        &r.name,
                        "this root file is not inside the directory of the first root file",
                    ));
                    continue;
                }
            }
        };
        let key = segs.as_ref().map(|s| s.join("/")).unwrap_or_else(|| r.path.clone());
        stack.push(Visit { key, segs, name: r.name.clone(), namespace, text: r.text.clone(), from: None, depth: 0 });
    }
    // Depth-first in `use` order; the first time a file is reached decides its place.
    let mut failed: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(v) = stack.pop() {
        if by_key.contains_key(&v.key) || failed.contains(&v.key) {
            continue;
        }
        let text = match v.text {
            Some(t) => t,
            None => {
                let path = v.segs.as_ref().map(|s| s.join("/")).unwrap_or_else(|| v.key.clone());
                match src.read(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        let d = match v.from.and_then(|(f, span)| files.get(f).map(|x| (x, span))) {
                            Some((f, span)) => Diagnostic::new(&f.name, &f.text, span, format!("cannot read `{}`: {e}", v.name)),
                            None => Diagnostic::without_location(&v.name, format!("cannot read `{}`: {e}", v.name)),
                        };
                        diags.push(d);
                        failed.insert(v.key);
                        continue;
                    }
                }
            }
        };
        let index = files.len();
        by_key.insert(v.key.clone(), index);
        let (module, file_diags) = parse_file(&text, &v.name, opts.max_nesting);
        diags.extend(file_diags);
        let mut use_keys = Vec::new();
        let mut children = Vec::new();
        for item in &module.items {
            let ast::Item::Use(u) = item else { continue };
            let Ok(path) = std::str::from_utf8(&u.path) else {
                diags.push(Diagnostic::new(&v.name, &text, u.path_span, "the path is not valid UTF-8"));
                use_keys.push(None);
                continue;
            };
            let target = resolve_use(path, v.segs.as_deref(), root_dir.as_deref()).and_then(|segs| {
                let rel = segs.get(root_dir.as_ref().map_or(0, Vec::len)..).unwrap_or(&[]).to_vec();
                namespace_of(&rel).map(|ns| (segs, ns))
            });
            match target {
                Ok(_) if opts.max_use_depth.is_some_and(|m| v.depth + 1 > m) => {
                    diags.push(Diagnostic::new(
                        &v.name,
                        &text,
                        u.path_span,
                        format!("`use` chain is deeper than the limit of {}", opts.max_use_depth.unwrap_or(0)),
                    ));
                    use_keys.push(None);
                }
                Ok((segs, namespace)) => {
                    let key = segs.join("/");
                    use_keys.push(Some(key.clone()));
                    let name = if Some(&segs) == root_segs.as_ref() { first.name.clone() } else { src.display_name(&key) };
                    children.push(Visit { key, segs: Some(segs), name, namespace, text: None, from: Some((index, u.path_span)), depth: v.depth + 1 });
                }
                Err(m) => {
                    diags.push(Diagnostic::new(&v.name, &text, u.path_span, m));
                    use_keys.push(None);
                }
            }
        }
        files.push(File { name: v.name, namespace: v.namespace, text, module, use_keys });
        stack.extend(children.into_iter().rev());
    }

    let order: HashMap<String, usize> = files.iter().enumerate().map(|(k, f)| (f.name.clone(), k)).rev().collect();
    let finish = |mut diags: Vec<Diagnostic>| {
        diags.sort_by_key(|d| (order.get(&d.file).copied().unwrap_or(0), d.line, d.column));
        cap_diags(&mut diags);
        Diagnostics(diags)
    };
    if !diags.is_empty() {
        return Err(finish(diags));
    }

    let mut units: Vec<Unit> = files
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let uses = f.use_keys.iter().map(|key| key.as_ref().and_then(|key| by_key.get(key).copied())).collect();
            Unit::new(&f.name, &f.namespace, k, &f.text, &f.module, uses, opts.api)
        })
        .collect();
    let mut refs: Vec<&mut dyn crate::link::Unit> = units.iter_mut().map(|u| u as &mut dyn crate::link::Unit).collect();
    crate::link::link(&mut refs, &crate::link::LinkOptions { api: opts.api }).map_err(finish)
}
