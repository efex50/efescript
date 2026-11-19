//! The in-memory half of building a mod folder (`mod.toml`, sources, assets): parsing
//! `mod.toml`, the build limits, and `build_mod`, which assembles the entry through a
//! [`FileSource`] and checks the result. Nothing here touches the file system except the
//! native-only helpers [`list_assets`] and `collect_files`.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Component;
use std::path::Path;

use efemod::manifest::Permissions;
use efemod::{Manifest, ReadLimits, Version, VersionReq};
use efevm::api::json::Value;
use efevm::api::ApiIndex;

use crate::asm::{assemble_program, AsmOptions, FileSource};

/// The `[build]` table of `mod.toml` (not copied into the package).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildToml {
    /// The entry file, relative to the mod folder.
    pub entry: String,
    /// Default `--api` inputs, relative to `mod.toml` (`.efapi` or `.efmod`).
    pub api: Vec<String>,
    /// The asset folder, relative to the mod folder.
    pub assets: String,
}

impl Default for BuildToml {
    fn default() -> Self {
        BuildToml { entry: "src/main.efa".into(), api: Vec::new(), assets: "assets".into() }
    }
}

/// A parsed `mod.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct ModToml {
    /// Everything that becomes `manifest.json` (not yet validated).
    pub manifest: Manifest,
    pub build: BuildToml,
}

type Table = toml::Table;

/// Fails on a key of `t` that is not in `known`, naming it.
fn no_unknown(t: &Table, known: &[&str], table: &str) -> Result<(), String> {
    for k in t.keys() {
        if !known.contains(&k.as_str()) {
            return Err(format!("mod.toml: unknown key `{k}` in [{table}] (known: {})", known.join(", ")));
        }
    }
    Ok(())
}

fn get_str(t: &Table, key: &str, table: &str) -> Result<Option<String>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("mod.toml: `{key}` in [{table}] must be a string")),
    }
}

fn get_int(t: &Table, key: &str, table: &str) -> Result<Option<i64>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::Integer(n)) => Ok(Some(*n)),
        Some(_) => Err(format!("mod.toml: `{key}` in [{table}] must be an integer")),
    }
}

fn get_strs(t: &Table, key: &str, table: &str) -> Result<Option<Vec<String>>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| match v {
                toml::Value::String(s) => Ok(s.clone()),
                _ => Err(format!("mod.toml: `{key}` in [{table}] must be an array of strings")),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(format!("mod.toml: `{key}` in [{table}] must be an array of strings")),
    }
}

fn get_table<'a>(root: &'a Table, key: &str) -> Result<Option<&'a Table>, String> {
    match root.get(key) {
        None => Ok(None),
        Some(toml::Value::Table(t)) => Ok(Some(t)),
        Some(_) => Err(format!("mod.toml: `{key}` must be a table")),
    }
}

/// A TOML value of `[meta]` as JSON. `path` names it in errors (`meta.a.b`). Floats are
/// written with `{:?}`, which is always a valid JSON number (`1.0`, `-2.5e10`); NaN and the
/// infinities have no JSON form and are refused.
fn toml_to_json(v: &toml::Value, path: &str) -> Result<Value, String> {
    Ok(match v {
        toml::Value::String(s) => Value::Str(s.clone()),
        toml::Value::Integer(n) => Value::Num(n.to_string()),
        toml::Value::Float(f) if f.is_finite() => Value::Num(format!("{f:?}")),
        toml::Value::Float(_) => return Err(format!("mod.toml: `{path}` in [meta] is not a finite number")),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::Str(d.to_string()),
        toml::Value::Array(a) => Value::Arr(
            a.iter().enumerate().map(|(i, x)| toml_to_json(x, &format!("{path}[{i}]"))).collect::<Result<_, _>>()?,
        ),
        toml::Value::Table(t) => Value::Obj(
            t.iter()
                .map(|(k, x)| Ok((k.clone(), toml_to_json(x, &format!("{path}.{k}"))?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

fn to_u32(n: i64, key: &str, table: &str) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| format!("mod.toml: `{key}` in [{table}] is out of range"))
}

/// Parses `mod.toml`. Unknown keys and tables are errors naming the key. The manifest part
/// is not validated here (`pack_dir` does that); only types and versions are checked.
pub fn read_mod_toml(text: &str) -> Result<ModToml, String> {
    let root: Table = text.parse().map_err(|e| format!("mod.toml: {e}"))?;
    for k in root.keys() {
        if !["mod", "dependencies", "permissions", "build", "meta"].contains(&k.as_str()) {
            return Err(format!(
                "mod.toml: unknown key or table `{k}` (known tables: mod, dependencies, permissions, build, meta)"
            ));
        }
    }
    let m = get_table(&root, "mod")?.ok_or("mod.toml: the [mod] table is missing")?;
    no_unknown(m, &["id", "version", "name", "description", "authors", "license", "game", "api_version"], "mod")?;
    let id = get_str(m, "id", "mod")?.ok_or("mod.toml: [mod] needs `id`")?;
    let version = get_str(m, "version", "mod")?.ok_or("mod.toml: [mod] needs `version`")?;
    let version: Version = version.parse().map_err(|e| format!("mod.toml: [mod] version: {e}"))?;
    let mut manifest = Manifest::new(id, version);
    manifest.name = get_str(m, "name", "mod")?;
    manifest.description = get_str(m, "description", "mod")?;
    manifest.authors = get_strs(m, "authors", "mod")?.unwrap_or_default();
    manifest.license = get_str(m, "license", "mod")?;
    manifest.game = get_str(m, "game", "mod")?;
    if let Some(n) = get_int(m, "api_version", "mod")? {
        manifest.api_version = to_u32(n, "api_version", "mod")?;
    }

    if let Some(d) = get_table(&root, "dependencies")? {
        // toml tables iterate in key order, which is the sorted-by-id order.
        for (id, v) in d {
            let toml::Value::String(req) = v else {
                return Err(format!("mod.toml: dependency `{id}` must be a range string"));
            };
            let req: VersionReq = req.parse().map_err(|e| format!("mod.toml: dependency `{id}`: {e}"))?;
            manifest.dependencies.push(efemod::Dependency { id: id.clone(), req });
        }
    }

    if let Some(p) = get_table(&root, "permissions")? {
        no_unknown(p, &["namespaces", "budget", "memory_pages"], "permissions")?;
        let mut perms = Permissions::default();
        perms.namespaces = get_strs(p, "namespaces", "permissions")?.unwrap_or_default();
        if let Some(n) = get_int(p, "budget", "permissions")? {
            perms.budget =
                Some(u64::try_from(n).map_err(|_| "mod.toml: `budget` in [permissions] is out of range".to_string())?);
        }
        if let Some(n) = get_int(p, "memory_pages", "permissions")? {
            perms.memory_pages = Some(to_u32(n, "memory_pages", "permissions")?);
        }
        manifest.permissions = perms;
    }

    let mut build = BuildToml::default();
    if let Some(b) = get_table(&root, "build")? {
        no_unknown(b, &["entry", "api", "assets"], "build")?;
        if let Some(e) = get_str(b, "entry", "build")? {
            build.entry = e;
        }
        if let Some(a) = get_strs(b, "api", "build")? {
            build.api = a;
        }
        if let Some(a) = get_str(b, "assets", "build")? {
            build.assets = a;
        }
    }
    if let Some(meta) = get_table(&root, "meta")? {
        manifest.meta = Some(toml_to_json(&toml::Value::Table(meta.clone()), "meta")?);
    }
    Ok(ModToml { manifest, build })
}

/// A `[build]` path: relative, no `..`, no root or prefix. Returns its segments.
pub fn inside(path: &str, what: &str) -> Result<Vec<String>, String> {
    let mut segs = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(s) => {
                segs.push(s.to_str().ok_or_else(|| format!("[build] {what}: `{path}` is not valid UTF-8"))?.to_string())
            }
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "[build] {what}: `{path}` must stay inside the mod folder (no absolute path, no `..`)"
                ))
            }
        }
    }
    if segs.is_empty() {
        return Err(format!("[build] {what}: `{path}` is empty"));
    }
    Ok(segs)
}

/// Limits on building a mod folder from an untrusted source.
#[derive(Clone, Copy, Debug)]
pub struct BuildLimits {
    /// Sum of every file read (`mod.toml`, sources, assets).
    pub max_total_bytes: u64,
    /// Files read, assets included.
    pub max_files: usize,
    /// Longest chain of `use` from the entry.
    pub max_use_depth: usize,
    /// Deepest nesting of brackets / blocks in one source file.
    pub max_nesting: usize,
}

impl Default for BuildLimits {
    fn default() -> Self {
        BuildLimits { max_total_bytes: 32 * 1024 * 1024, max_files: 1024, max_use_depth: 64, max_nesting: 128 }
    }
}

/// A mod folder whose `mod.toml` has been read and checked.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedMod {
    pub manifest: Manifest,
    pub build: BuildToml,
    /// The entry path inside the folder, `/`-separated.
    pub entry: String,
}

/// A built mod folder: everything a package holds.
#[derive(Debug)]
pub struct BuiltMod {
    pub manifest: Manifest,
    pub module: efevm::Module,
    pub assets: BTreeMap<String, Vec<u8>>,
}

/// Wraps a source and fails a read once `max_files` or `max_total_bytes` would be exceeded.
pub struct LimitedSource<'a> {
    inner: &'a dyn FileSource,
    max_files: usize,
    max_bytes: u64,
    files: Cell<usize>,
    bytes: Cell<u64>,
    /// Paths already counted: a file read again (cache verification, then the compile) is
    /// charged once, so a folder within the limits is not refused for being read twice.
    seen: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl<'a> LimitedSource<'a> {
    pub fn new(inner: &'a dyn FileSource, limits: &BuildLimits) -> LimitedSource<'a> {
        LimitedSource {
            inner,
            max_files: limits.max_files,
            max_bytes: limits.max_total_bytes,
            files: Cell::new(0),
            bytes: Cell::new(0),
            seen: Default::default(),
        }
    }

    fn count(&self, path: &str, len: usize) -> Result<(), String> {
        if self.seen.borrow().contains(path) {
            return Ok(());
        }
        let files = self.files.get() + 1;
        let bytes = self.bytes.get().saturating_add(len as u64);
        if files > self.max_files {
            return Err(format!("too many files (limit {}) at `{path}`", self.max_files));
        }
        if bytes > self.max_bytes {
            return Err(format!("too many bytes (limit {}) at `{path}`", self.max_bytes));
        }
        self.files.set(files);
        self.bytes.set(bytes);
        self.seen.borrow_mut().insert(path.to_string());
        Ok(())
    }
}

impl FileSource for LimitedSource<'_> {
    fn read(&self, path: &str) -> Result<String, String> {
        let t = self.inner.read(path)?;
        self.count(path, t.len())?;
        Ok(t)
    }

    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let b = self.inner.read_bytes(path)?;
        self.count(path, b.len())?;
        Ok(b)
    }

    fn display_name(&self, path: &str) -> String {
        self.inner.display_name(path)
    }

    fn list(&self) -> Option<Vec<String>> {
        self.inner.list()
    }
}

/// The assembler options for building under `limits`.
pub fn asm_options<'a>(api: Option<&'a ApiIndex>, limits: &BuildLimits) -> AsmOptions<'a> {
    AsmOptions { api, max_use_depth: Some(limits.max_use_depth), max_nesting: Some(limits.max_nesting) }
}

/// Reads `mod.toml` from the root of `src`, validates the manifest, checks the `[build]` paths.
/// `[build] api` entries are checked with [`inside`] and otherwise ignored.
pub fn prepare_mod(src: &dyn FileSource, limits: &BuildLimits) -> Result<PreparedMod, String> {
    let text = src.read("mod.toml").map_err(|e| format!("mod.toml: {e}"))?;
    if text.len() as u64 > limits.max_total_bytes {
        return Err(format!("mod.toml: larger than {} bytes", limits.max_total_bytes));
    }
    let mt = read_mod_toml(&text)?;
    mt.manifest.validate().map_err(|e| format!("mod.toml: {e}"))?;
    let entry = inside(&mt.build.entry, "entry")?.join("/");
    inside(&mt.build.assets, "assets")?;
    for a in &mt.build.api {
        inside(a, "api")?;
    }
    Ok(PreparedMod { manifest: mt.manifest, build: mt.build, entry })
}

/// Checks `module` against the manifest, then reads each asset `<build.assets>/<p>`.
pub fn finish_mod(
    p: PreparedMod,
    module: efevm::Module,
    src: &dyn FileSource,
    assets: &[String],
    limits: &BuildLimits,
) -> Result<BuiltMod, String> {
    efemod::check_module(&p.manifest, &module).map_err(|e| e.to_string())?;
    if assets.len() > limits.max_files {
        return Err(format!("too many files (limit {})", limits.max_files));
    }
    let dir = inside(&p.build.assets, "assets")?.join("/");
    let read_limits = ReadLimits::default();
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    for rel in assets {
        if rel == "manifest.json" || rel == "module.efvm" || rel == "engine" || rel.starts_with("engine/") {
            return Err(format!("asset `{rel}`: the name is reserved"));
        }
        efemod::container::validate_entry_name(&format!("assets/{rel}"), &read_limits)
            .map_err(|e| format!("asset `{rel}`: {e}"))?;
        let data = src.read_bytes(&format!("{dir}/{rel}")).map_err(|e| format!("asset `{rel}`: {e}"))?;
        total = total.saturating_add(data.len() as u64);
        if total > limits.max_total_bytes {
            return Err(format!("assets: too many bytes (limit {})", limits.max_total_bytes));
        }
        out.insert(rel.clone(), data);
    }
    Ok(BuiltMod { manifest: p.manifest, module, assets: out })
}

/// `prepare_mod`, assemble the entry (the folder is the source root), then `finish_mod`.
/// `api` is the index the entry is assembled against; `[build] api` entries are ignored.
pub fn build_mod(
    src: &dyn FileSource,
    api: Option<&ApiIndex>,
    assets: &[String],
    limits: &BuildLimits,
) -> Result<BuiltMod, String> {
    let ls = LimitedSource::new(src, limits);
    let p = prepare_mod(&ls, limits)?;
    let module = assemble_program(&p.entry, &ls, &asm_options(api, limits)).map_err(|d| d.to_string())?;
    finish_mod(p, module, &ls, assets, limits)
}

/// Every regular file below `dir`, as `(path relative with '/', full path)`, sorted.
/// Symbolic links are not followed (they are skipped).
#[cfg(not(target_family = "wasm"))]
fn collect_files(dir: &Path, rel: &str, out: &mut Vec<(String, std::path::PathBuf)>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name =
            e.file_name().into_string().map_err(|n| format!("{}: file name {n:?} is not valid UTF-8", dir.display()))?;
        let ty = e.file_type().map_err(|e2| format!("{}: {e2}", e.path().display()))?;
        let rel_name = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        if ty.is_symlink() {
            continue;
        } else if ty.is_dir() {
            collect_files(&e.path(), &rel_name, out)?;
        } else if ty.is_file() {
            out.push((rel_name, e.path()));
        }
    }
    Ok(())
}

/// Reads `<dir>/mod.toml` and lists the regular files under its `[build] assets` folder
/// (sorted, `/`-separated, relative to that folder; symlinks skipped). A missing default
/// `assets` folder is an empty list, a missing named one an error.
#[cfg(not(target_family = "wasm"))]
pub fn list_assets(dir: &Path) -> Result<Vec<String>, String> {
    let toml_path = dir.join("mod.toml");
    let text = std::fs::read_to_string(&toml_path).map_err(|e| format!("{}: {e}", toml_path.display()))?;
    let mt = read_mod_toml(&text)?;
    let seg = inside(&mt.build.assets, "assets")?;
    let mut asset_dir = dir.to_path_buf();
    asset_dir.extend(&seg);
    match std::fs::symlink_metadata(&asset_dir) {
        Ok(md) if md.is_dir() => {
            let mut files = Vec::new();
            collect_files(&asset_dir, "", &mut files)?;
            Ok(files.into_iter().map(|(rel, _)| rel).collect())
        }
        Ok(_) => Err(format!("[build] assets: `{}` is not a directory", mt.build.assets)),
        Err(_) if mt.build.assets == "assets" => Ok(Vec::new()),
        Err(e) => Err(format!("[build] assets: {}: {e}", asset_dir.display())),
    }
}

/// The API index of a bare module (no manifest): its section 13 under namespace `id`.
pub fn mod_index(module: &efevm::Module, id: &str, version: u32, doc: &str) -> ApiIndex {
    match &module.data().interface {
        Some(i) => i.index(id, version, doc),
        None => efevm::Interface::default().index(id, version, doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::MemSource;

    const TOML: &str = "[mod]\nid = \"demo\"\nversion = \"1.0.0\"\n";
    const MAIN: &str = "export fn main() {\n    ret\n}\n";

    fn folder() -> MemSource {
        MemSource::new().with("mod.toml", TOML).with("src/main.efa", MAIN)
    }

    #[test]
    fn builds_a_mem_folder() {
        let src = folder().with_bytes("assets/a/b.bin", &[0, 255, 7]);
        let b = build_mod(&src, None, &["a/b.bin".to_string()], &BuildLimits::default()).unwrap();
        assert_eq!(b.manifest.id, "demo");
        assert_eq!(b.assets["a/b.bin"], vec![0, 255, 7]);
    }

    #[test]
    fn too_many_files() {
        let src = folder().with_bytes("assets/x", b"1").with_bytes("assets/y", b"2");
        let l = BuildLimits { max_files: 3, ..BuildLimits::default() };
        let e = build_mod(&src, None, &["x".into(), "y".into()], &l).unwrap_err();
        assert!(e.contains("too many files"), "{e}");
    }

    #[test]
    fn too_many_bytes() {
        let src = folder().with_bytes("assets/x", &[0; 100]);
        let l = BuildLimits { max_total_bytes: 50, ..BuildLimits::default() };
        let e = build_mod(&src, None, &["x".into()], &l).unwrap_err();
        assert!(e.contains("too many bytes"), "{e}");
    }

    #[test]
    fn use_chain_too_deep() {
        let mut src = folder().with("src/main.efa", &format!("use \"f0.efa\"\n{MAIN}"));
        for k in 0..10 {
            src = src.with(&format!("src/f{k}.efa"), &format!("use \"f{}.efa\"\n", k + 1));
        }
        src = src.with("src/f10.efa", "");
        let l = BuildLimits { max_use_depth: 5, ..BuildLimits::default() };
        let e = build_mod(&src, None, &[], &l).unwrap_err();
        assert!(e.contains("deeper than the limit"), "{e}");
        build_mod(&src, None, &[], &BuildLimits::default()).unwrap();
    }

    #[test]
    fn nesting_too_deep_is_an_error() {
        let src = folder().with("src/main.efa", &"{".repeat(100_000));
        let e = build_mod(&src, None, &[], &BuildLimits::default()).unwrap_err();
        assert!(e.contains("nesting is deeper"), "{e}");
        let src = folder().with("src/main.efa", &format!("{}{}", "(".repeat(100_000), ")".repeat(100_000)));
        assert!(build_mod(&src, None, &[], &BuildLimits::default()).is_err());
    }

    #[test]
    fn reserved_asset_names() {
        for name in ["manifest.json", "module.efvm", "engine/x"] {
            let src = folder().with_bytes(&format!("assets/{name}"), b"x");
            let e = build_mod(&src, None, &[name.to_string()], &BuildLimits::default()).unwrap_err();
            assert!(e.contains("reserved"), "{name}: {e}");
        }
    }

    #[test]
    fn meta_table_converts_to_json() {
        let text = format!(
            "{TOML}[meta]\nname = \"x\\ty\"\ncount = 3\nratio = 1.0\nbig = -2.5e10\nflag = true\n\
             when = 1979-05-27T07:32:00Z\nlist = [1, [\"a\"], {{ k = false }}]\n\
             [meta.nested]\ndeep = {{ x = 0.5 }}\n"
        );
        let mt = read_mod_toml(&text).unwrap();
        let meta = mt.manifest.meta.clone().expect("meta");
        assert_eq!(meta.get("name"), Some(&Value::Str("x\ty".into())));
        assert_eq!(meta.get("count"), Some(&Value::Num("3".into())));
        assert_eq!(meta.get("ratio"), Some(&Value::Num("1.0".into())));
        assert_eq!(meta.get("big"), Some(&Value::Num("-25000000000.0".into())));
        assert_eq!(meta.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(meta.get("when"), Some(&Value::Str("1979-05-27T07:32:00Z".into())));
        assert_eq!(
            meta.get("list"),
            Some(&Value::Arr(vec![
                Value::Num("1".into()),
                Value::Arr(vec![Value::Str("a".into())]),
                Value::Obj(vec![("k".into(), Value::Bool(false))]),
            ]))
        );
        let nested = meta.get("nested").expect("nested");
        assert_eq!(nested.get("deep").and_then(|d| d.get("x")), Some(&Value::Num("0.5".into())));
        // The converted value is valid JSON text that parses back to itself.
        let m = &mt.manifest;
        assert_eq!(efemod::Manifest::from_json(&m.to_json()).unwrap().meta, m.meta);
        assert!(read_mod_toml(TOML).unwrap().manifest.meta.is_none());
    }

    #[test]
    fn meta_nan_and_infinity_are_refused() {
        for v in ["nan", "inf", "-inf"] {
            let e = read_mod_toml(&format!("{TOML}[meta]\nx = {v}\n")).unwrap_err();
            assert!(e.contains("`meta.x`") && e.contains("finite"), "{v}: {e}");
        }
        let e = read_mod_toml(&format!("{TOML}[meta]\na = {{ b = [nan] }}\n")).unwrap_err();
        assert!(e.contains("`meta.a.b[0]`"), "{e}");
    }

    #[test]
    fn meta_must_be_a_table() {
        let e = read_mod_toml(&format!("meta = 3\n{TOML}")).unwrap_err();
        assert!(e.contains("`meta` must be a table"), "{e}");
    }

    #[test]
    fn build_api_must_stay_inside() {
        let src = folder().with("mod.toml", &format!("{TOML}[build]\napi = [\"../x.efapi\"]\n"));
        let e = build_mod(&src, None, &[], &BuildLimits::default()).unwrap_err();
        assert!(e.contains("inside the mod folder"), "{e}");
        let src = folder().with("mod.toml", &format!("{TOML}[build]\napi = [\"ok.efapi\"]\n"));
        build_mod(&src, None, &[], &BuildLimits::default()).unwrap();
    }

    #[test]
    fn a_file_read_twice_is_counted_once() {
        let src = MemSource::new().with("a.efa", "xx");
        let l = BuildLimits { max_files: 1, max_total_bytes: 2, ..BuildLimits::default() };
        let ls = LimitedSource::new(&src, &l);
        ls.read("a.efa").unwrap();
        ls.read("a.efa").unwrap();
        ls.read_bytes("a.efa").unwrap();
    }

    /// Mutated and adversarial sources must be errors, never panics (untrusted input).
    #[test]
    fn mutated_sources_never_panic() {
        let base = "use \"lib.efa\" as l\nconst K = 3\nstruct S { x: i32, y: f32[2] }\nexport fn main(a: i64) -> i64 {\n    reg x\n    i64.add x, a, l.LIMIT\n  top:\n    br_if x, top\n    call_indirect (i64) -> i64, x, (a)\n    ret x\n}\ndata d = \"hi\", 1, 2\n";
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let junk = ["(", ")", "[", "]", "{", "}", "\"", "'", "\n", ",", "->", "use ", "\\", "\u{0}", "\u{1F600}", "-", "9999999999999999999999", "0x", "&", ".", ":", "pub "];
        let limits = BuildLimits { max_nesting: 16, max_use_depth: 4, ..BuildLimits::default() };
        for _ in 0..1500 {
            let mut b: Vec<char> = base.chars().collect();
            for _ in 0..(1 + next() % 6) {
                let i = (next() as usize) % (b.len() + 1);
                match next() % 3 {
                    0 if i < b.len() => {
                        b.remove(i);
                    }
                    1 => {
                        for c in junk[(next() as usize) % junk.len()].chars().rev() {
                            b.insert(i, c);
                        }
                    }
                    _ if i < b.len() => b[i] = junk[(next() as usize) % junk.len()].chars().next().unwrap(),
                    _ => {}
                }
            }
            let text: String = b.into_iter().collect();
            let src = MemSource::new().with("mod.toml", TOML).with("src/main.efa", &text).with("src/lib.efa", "pub const LIMIT = 1\n");
            let _ = build_mod(&src, None, &[], &limits);
        }
        for n in [10usize, 1000, 100_000] {
            let deep = format!("export fn main() {{\n    nop {}1{}\n    ret\n}}\n", "(".repeat(n), ")".repeat(n));
            let src = folder().with("src/main.efa", &deep);
            assert!(build_mod(&src, None, &[], &BuildLimits::default()).is_err() || n <= 128);
            let types = format!("export fn main() {{\n    nop {}\n}}\n", "[".repeat(n));
            let src = folder().with("src/main.efa", &types);
            let _ = build_mod(&src, None, &[], &BuildLimits::default());
        }
    }
}
