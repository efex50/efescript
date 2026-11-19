# E10 design: engine loading API, engine-owned caches, pluggable mod loading

Status: the contract for E10 (`mod-update-roadmap.md` at the repository root is the plan).
This file fixes the exact types and signatures the implementation follows. Where it is
stricter or more precise than the plan, this file wins.

## 0. Decisions that refine the plan

1. **Binary assets through `FileSource`.** `FileSource::read` returns `String`. A folder mod
   also has binary assets, so the trait gets `read_bytes` (default: `read` as bytes). `FsSource`
   overrides it to read raw bytes. `MemSource` stores bytes (`BTreeMap<String, Vec<u8>>`) with
   `with(path, &str)` and `with_bytes(path, &[u8])`; `read` checks UTF-8.
2. **The asset list is a parameter of `build_mod`.** A `FileSource` cannot list files, so the
   caller passes the asset paths (relative to the `[build] assets` folder, `/`-separated).
   Native helpers list them (`efa::mod_folder::list_assets`).
3. **`build_mod` is split** into `prepare_mod` (read `mod.toml`, validate the manifest, check
   paths) and `finish_mod` (check module against manifest, read assets), so the engine can put
   its cache around the assemble step in between. `build_mod` composes the three.
4. **`[build] api` at runtime:** each entry is checked with `inside()` (an absolute path or `..`
   is an error) and then ignored; the engine supplies the index. `efescript pack_dir` reads the
   entries (native) and passes the merged index to `build_mod`.
5. **Core without the `efa` feature:** the `LoadInput::Efa` / `LoadInput::Folder` and
   `ModBody::Folder` variants and `LoadError::Compile` do not exist (they need efa types).
   `Engine::load(path)` on a `.efa` file or a mod folder returns `LoadError::CompilerDisabled`,
   and `DirSource` skips folders with a warning.
6. **Errors by input kind:** `Efa` compile errors are `LoadError::Compile(Diagnostics)`. For
   `Package` and `Folder` every failure is `LoadError::Mod(ModError)`; folder build problems
   (mod.toml, limits, compile diagnostics as text) are `ModError::Build(String)`. `load_mods`
   reports `ModError` per label.
7. **Native script_demo ships mod folders.** `xtask build` (native) still packs the mods (into
   `target/mods/<example>/`, which checks them and is what the web dist uses), but fills
   `<profile>/mods/` with the mod *folders* (it first removes every `*.efmod` there), so the
   game compiles them at runtime through the cache and editing
   `<profile>/mods/inventory/src/items.efa` hot-reloads. Web dist keeps the `.efmod` packages.
8. **Cache directory** is `CacheDir::Platform` by default and is resolved lazily, on the first
   disk access, from the engine's game name (`set_game_name` before the first load counts).
   The size cap is enforced at that moment, once per engine.
9. **Game scripts compile against the full registry index** (game + engine + loaded mods'
   provides). Folder mods compile against the **game index** (the registry index without the
   namespaces owned by loaded packages / mods) merged with the index of each declared
   dependency (`mod_index` of its loaded module), like `xtask` packs them.
10. **Watching a `.efa` game script** is not part of E10. `watch` / `watch_package` stay for
    `.efvm` / `.efmod` paths; mod hot reload goes through `ModSource::changed`.

## 1. Crate `crates/efa` (package `efa`, edition 2021, version 0.1.0)

Dependencies: `efevm`, `efemod` (no features, never `write`), `toml = "1"`. Builds for
`wasm32-unknown-unknown`.

```rust
pub mod asm;        // moved from efescript, unchanged except AsmOptions limits and FileSource::read_bytes
pub mod link;       // moved from efescript, unchanged
pub mod mod_folder; // the in-memory half of efescript's pack.rs, plus build_mod
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

`asm::AsmOptions` (still `Clone, Copy, Debug, Default`):

```rust
pub struct AsmOptions<'a> {
    pub api: Option<&'a efevm::api::ApiIndex>,
    /// Longest chain of `use` from the root (root = depth 0). None = unlimited.
    pub max_use_depth: Option<usize>,
    /// Deepest nesting of brackets / blocks in one file. None = unlimited.
    pub max_nesting: Option<usize>,
}
```

Over-limit is an ordinary diagnostic, never a panic. The nesting check must run before any
recursive parsing (e.g. a token-level depth scan in `parse_file`), so deep input cannot blow
the stack.

`asm::source`:

```rust
pub trait FileSource {
    fn read(&self, path: &str) -> Result<String, String>;
    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> { self.read(path).map(String::into_bytes) }
    fn display_name(&self, path: &str) -> String { path.to_string() }
}
pub struct MemSource(pub BTreeMap<String, Vec<u8>>);   // new(), with(path, &str), with_bytes(path, &[u8])
#[cfg(not(target_family = "wasm"))] pub struct FsSource { .. }   // new(root), for_file(file), with_prefix(root, prefix)
```

`FsSource::with_prefix(root, prefix: impl Into<String>)` sets the display-name prefix
explicitly (used for mod folders so diagnostics name the folder).

`mod_folder`:

```rust
pub struct BuildToml { pub entry: String, pub api: Vec<String>, pub assets: String }   // as today
pub struct ModToml   { pub manifest: Manifest, pub build: BuildToml }                   // Clone, Debug, PartialEq
pub fn read_mod_toml(text: &str) -> Result<ModToml, String>;                            // + [meta] (section 3)
pub fn inside(path: &str, what: &str) -> Result<Vec<String>, String>;                   // now pub
pub fn mod_index(module: &efevm::Module, id: &str, version: u32, doc: &str) -> ApiIndex;

#[derive(Clone, Copy, Debug)]
pub struct BuildLimits {
    pub max_total_bytes: u64,   // sum of every file read (mod.toml, sources, assets); default 32 MiB
    pub max_files: usize,       // files read, assets included; default 1024
    pub max_use_depth: usize,   // default 64
    pub max_nesting: usize,     // default 128
}
impl Default for BuildLimits;

pub struct PreparedMod { pub manifest: Manifest, pub build: BuildToml, /** entry path inside the folder, `/`-separated */ pub entry: String }
pub struct BuiltMod    { pub manifest: Manifest, pub module: efevm::Module, pub assets: BTreeMap<String, Vec<u8>> }

/// Reads `mod.toml` from the root of `src`, validates the manifest, checks `[build]` paths.
pub fn prepare_mod(src: &dyn FileSource, limits: &BuildLimits) -> Result<PreparedMod, String>;
/// `check_module`, then reads each asset `<build.assets>/<p>` with `read_bytes`. Refuses asset
/// names that fail `efemod::container::validate_entry_name("assets/<p>")` and the reserved
/// names `manifest.json`, `module.efvm`, `engine/*`.
pub fn finish_mod(p: PreparedMod, module: efevm::Module, src: &dyn FileSource, assets: &[String], limits: &BuildLimits) -> Result<BuiltMod, String>;
/// prepare_mod + assemble_program(entry, LimitedSource(src), AsmOptions{api, limits}) + finish_mod.
pub fn build_mod(src: &dyn FileSource, api: Option<&ApiIndex>, assets: &[String], limits: &BuildLimits) -> Result<BuiltMod, String>;

/// Wraps a source and fails a read once `max_files` or `max_total_bytes` would be exceeded.
pub struct LimitedSource<'a> { .. }  // new(inner: &'a dyn FileSource, limits: &BuildLimits); implements FileSource
pub fn asm_options<'a>(api: Option<&'a ApiIndex>, limits: &BuildLimits) -> AsmOptions<'a>;

#[cfg(not(target_family = "wasm"))]
/// Reads `<dir>/mod.toml` and lists the regular files under its `[build] assets` folder
/// (sorted, `/`-separated, relative to that folder; symlinks skipped; a missing default
/// `assets` folder is an empty list, a missing named one an error).
pub fn list_assets(dir: &std::path::Path) -> Result<Vec<String>, String>;
```

The entry is assembled with the folder as the source root (`assemble_program("src/main.efa", ..)`),
so `use` resolves relative to the entry and must stay inside the entry's directory.

efescript keeps: CLI, LSP, debugger, `host_std`, `bundle`, audio, `pack_dir` (now
`read [build] api files` + `build_mod` + `Package::write`), `read_api_input`, `.efmod` writing.
It re-exports `pub use efa::{asm, link};` and `pub use efa::mod_folder::{read_mod_toml, mod_index, BuildToml, ModToml};`
from `pack`. Only efescript (and xtask, and test dev-dependencies) enable `efemod[write]`.

## 2. `Manifest.meta` (efemod)

```rust
pub struct Manifest { /* existing fields */ pub meta: Option<efevm::api::json::Value> }  // Eq is dropped (Value is PartialEq only)
pub const MAX_META_BYTES: usize = 64 * 1024;   // canonical JSON text of the value
pub const MAX_META_DEPTH: usize = 16;          // the object itself is depth 1
```

- `meta` must be a JSON object. `validate` and `from_json` enforce object, size and depth.
- `to_json` writes `"meta": <canonical JSON>` last, only when `Some`. Canonical = no spaces,
  keys in stored order, strings escaped as the existing writer does.
- `mod.toml`'s `[meta]` table (free-form) converts: string → Str, integer → Num, float → Num
  (NaN/inf refused), bool → Bool, datetime → Str (its TOML text), array → Arr, table → Obj
  (toml key order). `read_mod_toml` accepts the `meta` table.

## 3. Core: `ScriptCache` (`crates/core/src/engine/scripting/cache.rs`)

```rust
pub const CACHE_FORMAT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheDir { Platform, Off, Path(std::path::PathBuf) }   // default Platform

#[derive(Clone, Debug)]
pub struct CacheConfig { pub disk: CacheDir, pub memory: bool, pub max_disk_bytes: u64 }
// Default: Platform, true, 64 MiB. EngineConfig gains `pub script_cache: CacheConfig`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub mem_hits: u64, pub mem_misses: u64,
    /// Module::decode / Package::read calls actually made (a memory hit makes none).
    pub decodes: u64,
    pub disk_hits: u64, pub disk_misses: u64, pub disk_writes: u64, pub disk_evicted: u64,
    /// Compiles actually run (a disk hit runs none).
    pub compiles: u64,
}

pub struct ScriptCache { .. }
impl ScriptCache {
    pub fn new(config: CacheConfig) -> ScriptCache;
    pub fn config(&self) -> &CacheConfig;
    pub fn stats(&self) -> CacheStats;
    /// The resolved disk directory (`<base>/efa/v{CACHE_FORMAT}` lives under it), if any yet.
    pub fn disk_dir(&self) -> Option<&std::path::Path>;
    pub fn clear_memory(&mut self);
    pub fn clear_disk(&mut self) -> std::io::Result<()>;
    /// Memory cache over Module::decode (decode + verify). Key blake3(bytes); a hit is
    /// confirmed by comparing the full bytes.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<efevm::Module, efevm::LoadError>;
    /// Memory cache over efemod::Package::read(bytes, &ReadLimits::default()).
    pub fn read_package(&mut self, bytes: &[u8]) -> Result<efemod::Package, efemod::PackageError>;
    /// Disk-cached assemble of `root` in `files` against `api` (feature `efa`). Returns the
    /// decoded module and its bytes. `game` names the default directory.
    #[cfg(feature = "efa")]
    pub fn compile(&mut self, game: &str, root: &str, files: &dyn efa::asm::FileSource,
                   api: &efevm::api::ApiIndex, opts: &efa::asm::AsmOptions)
                   -> Result<(efevm::Module, Vec<u8>), efa::asm::Diagnostics>;
}
```

Rules:

- Memory cache: total stored bytes capped at 64 MiB; past the cap it is cleared. `memory:
  false` disables it (every call decodes).
- Disk key: `blake3( "efa-cache" | CACHE_FORMAT | efa::VERSION | files.display_name(root) | 0 |
  root contents | api.hash() )` as 64 hex chars. Files: `<base>/efa/v{N}/<first 2 hex>/<hash>.efvm`
  and `<hash>.deps`.
- `.deps`: text, first line `efa-deps 1`, then one line per file read during the compile:
  `<64 hex blake3 of contents> <path>` (the path as given to `FileSource::read`). A file that
  could not be read is not recorded (the compile failed then, and failures are not cached).
- Recording: a `FileSource` wrapper records every successful read during the compile.
- Lookup: key file pair present → every `.deps` path is re-read through `files` and its hash
  compared → `.efvm` read → `decode` (verify) → the module's section-12 `api.hash` must equal
  `api.hash()`. Any failure or mismatch is a miss (the stale pair is deleted) and the compile
  runs. A hit still goes through link and policy afterwards (the caller does that).
- Writes: `<name>.tmp-<pid>` then `rename`; `.efvm` first, then `.deps`. Write errors are
  logged (`log::warn!`) and ignored.
- Size cap: on first resolve of the directory, if the total size of `efa/v{N}` exceeds
  `max_disk_bytes`, delete pairs oldest-mtime first until under it. Older `efa/v*` format
  directories are deleted.
- Platform base: Linux/other unix `$XDG_CACHE_HOME` else `$HOME/.cache`; macOS
  `$HOME/Library/Caches`; Windows `%LOCALAPPDATA%`; then `efeengine/<game>` with the game name
  sanitised to `[A-Za-z0-9_-]` (others → `_`). Not found → no disk cache.
- wasm: the disk cache is always off (no fs code compiled).
- Dependency: `blake3 = { version = "1", default-features = false, features = ["std"] }` (pure Rust on wasm).

## 4. Core: loading (`crates/core/src/engine/scripting/load.rs`)

```rust
pub enum LoadInput<'a> {
    #[cfg(feature = "efa")]
    Efa    { name: &'a str, files: &'a dyn efa::asm::FileSource, root: &'a str },
    Efvm   { name: &'a str, bytes: &'a [u8] },
    Package{ label: &'a str, bytes: &'a [u8] },
    #[cfg(feature = "efa")]
    Folder { label: &'a str, files: &'a dyn efa::asm::FileSource, assets: &'a [String] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded { Module(ModuleId), Package(ModuleId) }
impl Loaded { pub fn id(self) -> ModuleId }

#[derive(Debug)]
pub enum LoadError {
    Io(String),
    #[cfg(feature = "efa")] Compile(efa::asm::Diagnostics),
    Script(ScriptError),
    Mod(ModError),
    CompilerDisabled,
    UnknownKind(String),     // the path
}
// Display, std::error::Error, From<ScriptError>, From<ModError>.

pub trait ModSource {
    fn list(&mut self) -> Vec<ModItem>;
    /// Labels whose content changed since the last `list`/`changed`. Default: none.
    fn changed(&mut self) -> Vec<String> { vec![] }
}
pub struct ModItem { pub label: String, pub body: ModBody }
pub enum ModBody {
    Package(Vec<u8>),
    #[cfg(feature = "efa")]
    Folder { files: Box<dyn efa::asm::FileSource>, assets: Vec<String> },
}

#[cfg(not(target_family = "wasm"))]
pub struct DirSource { .. }
impl DirSource { pub fn new(dir: impl Into<std::path::PathBuf>) -> DirSource }
impl ModSource for DirSource { .. }

pub trait AssetHandler {
    fn matches(&self, path: &str) -> bool;
    fn check(&mut self, m: &Manifest, path: &str, bytes: &[u8]) -> Result<(), String>;
    fn commit(&mut self, id: ModuleId, m: &Manifest, path: &str, bytes: &[u8], res: &mut Resources);
    fn unload(&mut self, id: ModuleId, res: &mut Resources);
}
```

`ModError` gains `Build(String)` and `Asset { path: String, reason: String }`.
`LoadReport` is unchanged (`loaded: Vec<(String, ModuleId)>`, `failed: Vec<(String, ModError)>`).

`Engine`:

```rust
#[cfg(not(target_family = "wasm"))]
pub fn load(&mut self, path: impl AsRef<Path>) -> Result<Loaded, LoadError>;
pub fn load_input(&mut self, input: LoadInput<'_>) -> Result<Loaded, LoadError>;
pub fn add_mod_source(&mut self, s: impl ModSource + 'static);
pub fn set_mod_policy(&mut self, p: impl ModPolicy + 'static);      // unchanged
pub fn add_asset_handler(&mut self, h: impl AssetHandler + 'static);
pub fn load_mods(&mut self) -> LoadReport;
pub fn cache(&mut self) -> &mut ScriptCache;
```

Kept: `load_module(name, bytes)`, `load_mod(bytes, ModOptions)` (both decode through the
memory cache), `unload_module`, `unload_module_and_dependents`, `package(id)`, `watch`,
`watch_package`. **Removed:** `load_module_file`, `load_mods_dir`, `load_packages`, the public
`load_package`.

`Engine::load(path)`:
- a directory containing `mod.toml` → `Folder` (label = folder name, files =
  `FsSource::with_prefix(dir, dir.display())`, assets = `list_assets(dir)`);
- `*.efa` → `Efa` (name = file stem, files = `FsSource::for_file`, root = file name);
- `*.efvm` → read → `Efvm` (name = file stem); `*.efmod` → read → `Package` (label = file name);
- anything else → `UnknownKind`; read errors → `Io`. Without the `efa` feature: `.efa` and mod
  folders → `CompilerDisabled`.

Trust and pipeline:
- `Efa`, `Efvm`: game code, no policy, as `load_module` today (`Efa` compiles through
  `ScriptCache::compile` against the full registry index).
- `Package`, `Folder`: always game check → id clash → dependency check (must be loaded
  packages) → policy and grant → **asset check** (for each asset, the first handler whose
  `matches` is true runs `check`; any `Err` → `ModError::Asset`, nothing loaded) → instantiate
  → **commit** (each matched asset to its handler, with `&mut Resources`) → `PackageInfo` holds
  only the assets no handler matched. `init` runs once after the load (or after all loads in
  `load_mods`, in load order).
- Folder: `prepare_mod` (with `BuildLimits::default()`) → `ScriptCache::compile(entry,
  LimitedSource(files), game index + deps' indexes, asm_options(limits))` → `finish_mod` →
  package path above. Asset reads count against the limits.
- Unload of a package calls `unload(id, res)` on every handler that committed for it.

`load_mods()`:
1. `list()` every source in registration order, remembering (source index, label).
2. Read manifests only: `read_package` for packages, `prepare_mod` for folders. Failures go
   to `report.failed`.
3. One `efemod::resolve` over all items and the loaded packages.
4. Load in resolution order; dependents of a failed item fail with
   `ResolveProblem::DependencyFailed`.
With no source registered it returns an empty report. `DefaultPolicy` still grants nothing.

Hot reload: the script system (every target), at most every 0.25 s of engine time, calls
`changed()` on each source. For each changed label that is loaded, it calls `list()` once for
that source, takes the item with that label and reloads it in place (package: the
`reload_package` rules; folder: recompile through the cache, then the same rules). After a
successful reload, each loaded dependent that is a folder mod (transitively, in load order)
is recompiled and reloaded when its module bytes changed. Asset handlers get `unload` then
`commit` for the reloaded package. Failures are logged; the old version keeps running.

## 5. Tests (`crates/core/tests/load.rs`), demo, xtask

See the plan, section 6. Tests use `CacheDir::Path(<temp dir>)` or `CacheDir::Off`, never the
platform directory. `efeengine_core` re-exports `efa` (`pub use efa;`, feature `efa`) so games
and tests reach `MemSource`/`FsSource` without depending on efa directly.
