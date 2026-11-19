# efescript E8 design: modules and mod packages

Status: implemented (E8.1–E8.13), 2026-11-16. Draft 1 was written 2026-11-15; the owner's
decisions (section 13) and what changed while building it (section 14) override the earlier
sections where they differ.
Scope: roadmap section E8 (`crates/efescript/ROADMAP.md`). The request it answers: "add module
support and a jar-like bundling system for better moving the mods". Normative documents:

| What | Where |
|---|---|
| package container, entries, manifest, versions, limits | `crates/efevm/MOD_PACKAGE.md` |
| `.efvm` section 13 `interface` | `crates/efevm/SPEC.md` 9.2 |
| `.efa` `use`, `pub`, `provide`, `struct`, `scratch` | `docs/efa-syntax.md` section 16 |

This document holds the decisions with their reasons (2), the design of each part (3–10)
and the implementation plan (11). Where it and a normative document differ, the normative
document wins.

**Superseded in part by E10** (`docs/e10-design.md`, `crates/efescript/ROADMAP.md`). The
runtime loading API described in sections 9.2 and 10 changed:
- Removed: `load_module_file`, `load_mods_dir`, `load_packages` and the public `load_package`.
  Use `Engine::load(path)` (a `.efa`, `.efvm`, `.efmod` file or a mod folder),
  `Engine::load_input(LoadInput)`, or `add_mod_source` (a `ModSource`, e.g. `DirSource`) and
  `load_mods()`. Mod sources are never installed automatically.
- Folder mods (`mod.toml` with sources, section 4 of `MOD_PACKAGE.md`) are also compiled at
  runtime by the engine, through `ScriptCache` (memory and disk). `efescript pack` still builds
  `.efmod` files. Mods can carry `meta` and asset handlers (`AssetHandler`).
- Native `cargo xtask build` copies mod folders next to the game; web builds keep `.efmod` files.

The rest of this document stays as the record of E8.

## 1. Goal and the shape of the result

A mod is written as many source files, built into **one module**, shipped as **one package
file** (`.efmod`, a restricted 7z archive), and loaded from bytes by any game built with the engine.
The package carries the manifest (id, version, dependencies, permission requests), the module
(with its declared interface) and assets. The mod's API index is derived from the module, so
it travels with it without being a second copy.

```
mod folder                    build (efescript pack)                 game (engine)
mod.toml ──────────────┐
src/main.efa ─ use ─┐  ├──► manifest.json ┐                    load_package(bytes)
src/items.efa ──────┴──┼──► link ─► module.efvm (sections 12, 13) ├─► .efmod ─►  policy grants
assets/** ─────────────┘                                        ┘            resolve deps
                                                                              link, register provides
```

Three rules from the API index carry over unchanged: registering is the only way to make a
function callable, registering puts it in the index, and tools read only index files.

## 2. Decisions

### D1. Container: a 7z archive, read with `sevenz-rust2` (changed by the owner, 2026-11-15)

A `.efmod` is a 7z archive restricted to a small profile (MOD_PACKAGE.md section 1: LZMA2 or
copy coders only, one coder and one entry per folder (non-solid), no encryption, no encrypted
or other filtered headers, dictionary and header sizes capped). Mod authors and players open it
with tools they already have (7-Zip, any archive manager), which is the point of a jar-like
file. The owner chose 7z over the restricted ZIP of draft 1 (a ZIP reader on `miniz_oxide` was
implemented first and then replaced). The reader is the pure-Rust crate `sevenz-rust2` with
default features off (no AES, bzip2 or PPMd), so `efemod` still builds for wasm32; the
writer is efemod's own (on the `lzma-rust2` encoder, because `sevenz-rust2`'s writer always
compresses the header) and sits behind `efemod`'s `write` feature, which only
the packer (`efescript pack`, `cargo xtask mods`) turns on, never the engine.

**Trade-off.** The parser of untrusted input is now a third-party crate rather than a few
hundred lines we own. `efemod` therefore checks from the outside what it can before decoding
(coder ids, dictionary size, header size, entry count, one entry per folder, declared sizes)
and reads every entry through a bounded reader; a never-panics test drives the crate's parser
with mutated and random packages. Findings about the crate are recorded in "Changes recorded
during implementation". The writer is canonical (fixed order, no timestamps), so packs are
reproducible.

### D2. Manifest: `mod.toml` to write, `manifest.json` inside the package

Authors write `mod.toml` (comments, readable, familiar from Cargo) in the mod folder; only
build tools read it, with the `toml` crate (already in the workspace through `xtask`).
`efescript pack` validates it and writes `manifest.json` into the package. The engine reads
only the JSON, with the hand-written, depth-limited JSON reader efevm already has for
`.efapi` files, so the engine side needs no TOML or serde dependency and parses one format with
one strict reader. This is the Cargo/npm split: an authoring file and a normalised file in the
artifact. Fields, types and validation are in MOD_PACKAGE.md sections 3–5; in short: `id` (a
lowercase identifier that is also module name, namespace and owner), `version` (SemVer without
build metadata), `name`, `description`, `authors`, `license`, optional `game`, `api_version`
(the namespace version, default 1), `dependencies` (id → Cargo-style range: `^ ~ = > >= < <=`,
comma for "and", bare means `^`), and `permissions` (`namespaces`, `budget`, `memory_pages`) as
requests. There is no "built against" key: section 12 of the module already records that, and
two copies could disagree.

### D3. Multi-file `.efa`: `use "path.efa" [as alias]`, `pub`, access as `alias.item`

A file pulls in another with `use "util/math.efa" as m` (alias defaults to the file stem) and
names its public items `m.square`, `m.LIMIT`, `m.table.len`, `m.Item.size`. Items are private
unless `pub` (`export` implies public). The syntax reuses what `.efa` already has: dotted names
are single tokens and already mean "qualified" (`ns.name`, `msg.len`, `S.size`), so no `::`
token is added, and string paths avoid inventing a module path language before E6 designs its
own `use other::thing`. Ambiguity is removed at the alias declaration (an alias may not equal a
local name, a visible struct, or the first segment of an imported namespace), so every dotted
name has one meaning. Imports stay per file and are merged. Each file has a namespace from its
path (`util/math.efa` → `util.math`) used for debug names (`util.math.square` in the names
section) and its own file entry in the lines section, so backtraces and the debugger show
where code came from. Paths are relative, cannot leave the root directory, and cycles are
allowed because linking is phased (D4). Full rules: efa-syntax section 16.

### D4. Linker: phased linking of in-memory units into one `ModuleBuilder` (declare, resolve, define; section 14 item 1)

The linker does not take `.efvm` files. A verified module has lost what linking needs: a
data address is just a `const.i64` immediate, indistinguishable from the number 1024, and
globals cannot be imported, so merging modules would need a relocation format in efevm. The
linker instead takes **units** (one per source file, any front end) in two phases: `declare`
returns each unit's symbols with everything that decides layout (function signatures, data
item sizes and alignment, globals, consts, structs, imports, provides, exports, visibility);
the linker assigns every function index, global index and data address for the whole program
and hands back a `SymbolMap`; then `define` lets each unit emit its bodies, data bytes, names
and lines into the one shared `efevm::ModuleBuilder`, with all addresses already known. No
relocations exist, nothing is patched afterwards, and the result is verified once by
`finish()`. The input is a Rust trait (`link::Unit`), not `.efa`: the assembler implements it
per file, E6's compiler implements it per language file, and both can be mixed in one build.
A program of one `.efa` unit must produce exactly today's bytes (the round-trip tests keep
passing), which fixes the order of builder calls. An on-disk object format (for caching or
prebuilt libraries) is not needed for E8 and is listed under later work. So the roadmap's
`efescript link a.efvm b.efvm` command is replaced by `efescript build root.efa`, which follows
`use`, and by `build a.efa b.efa` for several roots.

### D5. Section 13 `interface`: provides, struct layouts, scratch region

A new `.efvm` section 13 (layout and verifier rules: SPEC.md 9.2) lists the module's
**provides** (name, function index, doc, parameters and results with names and `ApiType`s),
the **struct layouts** those use (own ones, and copies of external ones with their owner so the
mod's index is self-contained and the engine can check the layout matches), and the
**scratch** region for copied arguments. Dependencies and their version ranges are not in the
module: they are package-level facts and live in the manifest; namespace versions are already
in section 12. Section 13 is a regular known section (not a custom section ≥ 128, which a
decoder may skip), because a module's API must never be silently dropped. The format version
stays 1; there is no compatibility with engines from before E8 to keep (the project has no
released users). The
engine registers one trampoline per provide when the mod loads, through `HostRegistry`, so a
provided function is in the index because it was registered, as every callable function is;
`efevm::Interface::index(id, version, doc)` derives the mod's `.efapi` from the same data at
build time.

### D6. `engine.mods.export` is removed; unload refuses, with an explicit cascade; reload checks dependents

Declared provides replace `engine.mods.export` completely: they carry types beyond scalars,
docs and parameter names, are known before any code runs (no window between load and the end
of `init` when the index is incomplete), and can be checked at build time. Keeping both would
give two registration paths with different hot-reload rules for the same thing. The function is
removed and `engine.mods` becomes version 2 with asset access instead (9.4); the tests and the
`script_demo` mod move to `provide`. **Unload is refused** while another loaded module imports
from the mod's namespaces (`ScriptError::Needed { module, by }`), because silently breaking
the callers (today's behaviour: their calls trap) is surprising; the game can ask for the cascade
explicitly with `unload_module_and_dependents`, which unloads dependents first in reverse load
order and returns what it removed. **Hot reload** registers the new provides, unregisters the
ones the new version dropped (closing today's stale-index gap), and is **refused** (old
version keeps running, error logged) when a loaded dependent imports a function the new
version drops or changes, when a struct a dependent uses changes layout, or when
`api_version` changes while dependents are loaded. Trampolines call by provide name, so
dependents reach the new code without relinking.

### D7. Copy-in/copy-out through the provider's scratch region

Memories are separate, so a pointer cannot cross. The trampoline copies each `str`, `bytes`
and `&Struct` argument from the caller's memory into the **provider's scratch region** (an
8-aligned area the assembler reserves, default 4 KiB, at most 1 MiB, declared in section 13),
passes pointers into that region to the provider, and after a successful return copies
`bytes` and struct arguments back into the caller's original ranges. `str` is copy-in only and
must be valid UTF-8. Results stay scalar, as the index already requires. A fixed region is
simpler and more predictable than calling an allocator inside the provider (no re-entrant call,
no guest bug can corrupt the copy, the limit is visible in the index tooling) and it is safe
because a running module cannot be entered again, so one region per instance is never used by
two calls at once. Arguments that do not fit, out-of-range caller pointers and invalid UTF-8
trap in the caller with the provide's and parameter's name; the copy costs the provider's
budget like `memory.copy`. Details: section 7.

### D8. Engine API: `load_package(bytes)`, a policy hook, set-wise dependency resolution

`Engine::load_package(&[u8])` and `load_packages(Vec<(label, bytes)>)` work everywhere,
`load_mods_dir(dir)` scans `mods/*.efmod` on native only. A `ModPolicy` set by the game sees
each request (id, version, requested namespaces, budget, memory, dependencies and the
namespaces the module really imports) and returns a `Grant` or a refusal; the package never
grants itself anything. **The default policy (`DefaultPolicy`) grants nothing**: no
namespace at all, budget and memory at the engine defaults. A game must install a `ModPolicy`
to give mods any namespace, which matches E5's rule that an empty allow-list allows nothing.
A grant applies to the **whole package** as one unit (its one module, linked from all its
source files, and its assets); there is no per-file or per-entry permission. A mod id may not
equal a namespace that is already registered (the game's own, `engine`, `std`, `efe`, or one
of another mod) nor be the first segment of one; the loader refuses such a package before it
asks the policy, with an error naming the clash. Dependency resolution (`efemod::resolve`, pure and testable without an engine) runs
over the whole set before anything is instantiated: duplicate ids, missing dependencies,
versions outside the range and cycles are reported by name, dependents of a failed package
are skipped with "dependency X failed", the rest load in dependency order (ties by id), and
every outcome is in a `LoadReport`. Details: section 9.

## 3. Multi-file `.efa` (E8.1)

Normative: efa-syntax section 16. Implementation shape:

- **File source.** `asm::FileSource` trait: `fn read(&self, path: &RelPath) -> Result<String,
  String>`. `FsSource { root }` (native, canonicalises, refuses escapes and symlinks out) and
  `MemSource(BTreeMap<String, String>)` (tests, LSP buffers, web tools). The assembler never
  touches the file system directly.
- **Entry point.** `asm::assemble_program(root: &str, src: &dyn FileSource, opts:
  &AsmOptions) -> Result<Module, Diagnostics>`; `assemble_with(src, file, opts)` stays and is a
  program of one in-memory file. `AsmOptions` stays `{ api }`: the CLI merges several `--api`
  into one index (section 14 items 4 and 12).
- **Pipeline.** Parse every file of the use graph (lexer and parser unchanged apart from the
  new items, `&` token and reserved words), build one `asm::Unit` per file, run the linker
  (4). Diagnostics carry the file name already; they are collected over all files, sorted by
  (file order, line, column) and capped at 100 in total.
- **Language server.** Documents are parsed with a `FileSource` that prefers open buffers over
  disk. Go to definition and hover follow `alias.item` to the other file; completion after
  `alias.` lists that file's public items; diagnostics of a file include errors found while
  linking the program it belongs to (the root is the nearest `mod.toml`'s `[build] entry`, or
  the file itself).
- **Debugger.** `break FILE:LINE` already exists; it now resolves the file through the lines
  section's file list; `list` and `where` show the file. Backtraces print the qualified names.

## 4. Linker (E8.2)

Module `efescript::link` (`src/link/{mod.rs, layout.rs, tests.rs}`; the efescript crate,
beside `asm` and the future E6 compiler; efevm stays a VM). Implemented interface:

```rust
pub trait Unit {
    fn file(&self) -> &str;                 // name in diagnostics and the lines section
    fn namespace(&self) -> &str;            // qualifies function names (`util.math`); "" for the root
    fn declare(&mut self, diags: &mut Vec<Diagnostic>) -> Decls;                      // phase 1
    fn resolve(&mut self, cx: &ResolveCx<'_>, diags: &mut Vec<Diagnostic>) -> Resolved; // phase 2a
    fn define(&mut self, cx: &mut DefineCx<'_>, diags: &mut Vec<Diagnostic>);           // phase 2c
    fn diagnostic(&self, span: Span, message: String) -> Diagnostic;  // default: no source line
}

pub struct Decls {
    pub imports: Vec<ImportDecl>,   // namespace, name, ty: FuncType, span
    pub funcs: Vec<FuncDecl>,       // name, ty, public, export: Option<String>, span
    pub globals: Vec<GlobalDecl>,   // name, ty: ValType, mutable, public, export, span
    pub data: Vec<DataDecl>,        // name, size, align (8 for .efa), public, span
    pub consts: Vec<ConstDecl>,     // name, value: ConstValue (Int(i128) | Float(text)), public, span
    pub structs: Vec<StructDecl>,   // desc: StructDesc, public, export, span
    pub provides: Vec<ProvideDecl>, // name, params, results: Vec<ProvideParam>, doc, target: SymRef, span
    pub memory: Option<MemoryDecl>, // min: Option<u32>, max, span
    pub table: Option<Span>,        // entries come in Resolved
    pub scratch: Option<ScratchDecl>, // size, span
}
pub enum ProvideType { Api(ApiType), Struct(SymRef), IndexStruct(StructDesc) }
pub struct SymRef { pub unit: usize, pub name: String, pub span: Span }
pub struct Resolved { pub globals: Vec<u64>, pub data: Vec<Vec<u8>>, pub table: Option<Vec<Option<u32>>> }

pub struct SymbolMap { pub units: Vec<UnitSyms>, pub imports: Vec<(String, String, FuncType)>,
                       pub memory: (u32, Option<u32>), pub scratch: Option<(u64, u32)>, pub data_end: u64 }
pub struct UnitSyms { pub file, pub namespace, pub decls: Decls,
                      pub imports: Vec<u32>, pub funcs: Vec<u32>, pub globals: Vec<u32>, pub data: Vec<u64> }
impl SymbolMap {
    pub fn unit(&self, u: usize) -> Option<&UnitSyms>;
    pub fn symbols(&self, u: usize) -> Vec<SymbolEntry>;   // space, name, symbol, public, span; incl. x.len, S.size ...
    pub fn lookup(&self, from: usize, target: usize, space: Space, name: &str) -> Result<Symbol, LookupError>;
}
pub enum Space { Func, Global, Value, Struct }
pub enum Symbol { Func { index, ty }, Global { index, ty, mutable }, Value(ConstValue), Struct(StructDesc) }
pub enum LookupError { Unknown, Private { file: String, span: Span } }   // .diagnostic(..) renders it
pub struct ResolveCx<'a> { pub syms: &'a SymbolMap, pub unit: usize }
pub struct DefineCx<'a> { pub builder: &'a mut ModuleBuilder, pub syms: &'a SymbolMap, pub unit: usize, pub file_id: u32, .. }
impl DefineCx<'_> {
    pub fn define(&mut self, func: u32, body: FunctionBuilder, spans: Vec<Span>, name_span: Span);
    pub fn reg_name(&mut self, func: u32, reg: u16, name: &str);
}
pub struct LinkOptions<'a> { pub api: Option<&'a ApiIndex> }   // Copy + Default
pub fn link(units: &mut [&mut dyn Unit], opts: &LinkOptions) -> Result<efevm::Module, Vec<Diagnostic>>;
// layout: pub fn layout(..) -> SymbolMap; pub fn interface(..) -> Option<Interface>;
//         DATA_START, PAGE, DEFAULT_SCRATCH, MAX_SCRATCH, align_up, first_note
```

Every phase writes diagnostics to a sink and goes on, so one run reports as much as the
single-file assembler did; `finish()` runs only when there is none. Unit 0 is the root.

Phase 1 (`layout::layout`), in unit order (efa-syntax 16.1):
1. Merge imports by `ns.name` (a different signature in another unit → error at the later
   one naming the first file); imports get indices `0..` in first-appearance order.
2. Functions get indices after the imports in unit order, then source order. Globals likewise.
3. Data addresses: one cursor from 1024 over all units, each item aligned to its `align`; then
   the scratch region (8-aligned, at 1024 without data; size from the root's `scratch`, else
   4096 if any provide has a `str`/`bytes`/struct parameter, else none; `scratch 0` with such
   a provide is an error).
4. Memory: the root's `memory` item (it must hold data and scratch), else `ceil(end / 65536)`
   pages, at least 1; `end` is 0 when there is neither data nor scratch, as in section 8.
5. Program-wide checks: export names (functions, then globals), provide names and exported
   struct names unique; at most one `table`; `memory`/`scratch` only in the root. The
   duplicate note is "first defined at line N" in the same unit (today's text) and "first
   defined in FILE:N" across units.
6. `layout::interface` builds section 13 (below) before phase 2.

Phase 2: (a) every unit `resolve`s global initial values, data bytes and table entries with
the `SymbolMap`; (b) the linker writes imports, `declare_func` for every function, globals,
table, memory and data (no segment for empty bytes); (c) per unit `file(name)` (always, so
the lines section has one file per unit) and `define`, whose bodies use `loc(file_id, …)`;
(d) the API reference (with `LinkOptions::api`: index hash and the version of every imported
namespace), exports (functions by index, then globals), names (imports `ns.name`, functions
qualified by the unit's namespace, then the collected register names), `interface(..)` last.
Type interning order is therefore imports, function declarations, then the bodies'
`add_type`, exactly as before: a program of one unit gives today's bytes. A verification
error is mapped through the spans given to `DefineCx::define` to the unit's statement.

Section 13: provides in unit then source order with their function index; structs: every
own `export struct` (unit then source order), then copies of index structs with their owner
and component id in the order provides first use them; the scratch region of step 3. Each
provide's lowered signature is checked against its function (error showing both), results
must be scalar, `ptr` is refused, a program struct must be `export` and visible, parameter
names are unique. Section 13 is written only when there is a provide, an exported struct or a
scratch region.

**E6.** The language compiler implements `Unit` for each language file: its type checker runs
before `declare` (sizes and signatures are known from types), its code generator runs in
`define`. A language file can `use` an `.efa` file and the other way round once E6 defines
how a language `use` names an `.efa` file; the linker does not care which front end a unit
came from.

## 5. Section 13 in efevm (E8.4)

Normative: SPEC.md 9.2. New in `efevm`:
- `module::{Interface, Provide}`, `ModuleData.interface: Option<Interface>`; encode, decode
  (`DecodeError::{BadApiType, BadPrim}`), verify (`VerifyError::Interface { what, reason }`);
  `ModuleBuilder::interface(Interface)`.
- `Interface::host_fns(namespace) -> Vec<HostFn>` (the descriptions the engine registers) and
  `Interface::index(id, version, doc) -> ApiIndex` (the mod's own index: one namespace `id`
  owned by `id`, the provides as functions, own structs owned by `id`, copied structs with
  their recorded owner). Both pure data, no I/O.
- `ApiIndex::merge(&[&ApiIndex]) -> Result<ApiIndex, ApiError>` for repeated `--api` (8):
  identical namespaces, functions and structs are deduplicated; the same name with different
  content is an error naming both games; the merged `game` is the first one's.
- The JSON reader's `Value` and `parse` become public (`efevm::api::json`), so `efemod` parses
  `manifest.json` with the same strict reader instead of adding a JSON dependency.

## 6. Declared exports in the engine (E8.4, E8.11)

- **Registration at load.** When a module with section 13 is loaded as a mod
  (`load_mod`, `load_package`), right after it links and before `init` runs: define namespace
  `<id>` (version `api_version`, owner `<id>`, doc from the manifest), register own structs
  (`register_struct`, owner `<id>`), check copied structs against the registry (same size,
  alignment, fields; otherwise `ScriptError::StructMismatch { name, expected, found }`), then
  register one trampoline per provide with `Interface::host_fns`. Any failure rolls back
  everything registered for this load (as `register_component` does today) and the module is
  not kept. A game module (`load_module`) with section 13 is refused: only mods provide.
- **Trampoline.** Captures `(ModuleId, provide name)`, looks the provide up in the slot's
  current module on every call (so hot reload needs no re-link of callers), then performs the
  copy ABI (7) and runs the function with `spawn_func(index)` from the provider's budget, as
  `run_export` does today.
- **`engine.mods` v2.** `export` is removed; `asset_len(path: str) -> (len: i64)` and
  `asset_read(path: str, buf: bytes) -> (len: i64)` read the calling mod's own package assets
  (9.4); -1 for no such asset or no package. `ModOptions` loses nothing but no longer needs
  `engine.mods` to provide.
- **Dependents.** Module X depends on mod Y when X imports from a namespace owned by Y. It is
  computed from imports, so it also covers bare modules loaded with `load_mod`.
- **Unload.** `Engine::unload_module(id)` returns `ScriptError::Needed { module, by:
  Vec<String> }` while dependents are loaded. `Engine::unload_module_and_dependents(id) ->
  Result<Vec<ModuleId>, ScriptError>` unloads dependents first (reverse load order). Unload
  drops the instance, the package assets and every registry entry of the owner (functions,
  namespace, own structs).
- **Hot reload** (native: `watch_module` for bare mods, `watch_package` for packages). The new
  bytes are read, decoded, verified and linked first; then, if dependents are loaded, the
  reload is refused unless every function a dependent imports still exists with the same
  lowered signature, `api_version` is unchanged, and every struct a dependent uses has the same
  layout. If accepted: provides that disappeared are unregistered, changed docs and new
  provides are registered (`replace`/`register`), own structs nobody else uses are replaced,
  the instance is swapped, `init` runs again and the states of its `Script`s reset to 0 as
  today. A package reload also re-reads the manifest: its `id` must be unchanged, dependencies
  must still resolve against what is loaded, and the policy is asked again.

## 7. The copy ABI across mods (E8.5)

For a call from module C (caller) to provide P of mod M, with M's scratch region `[S, S+N)`:

1. **Budget.** Units = 1 + (bytes copied in + bytes to copy out) / 256, taken from M's
   remaining budget before anything runs; not enough → the caller traps "out of budget in
   `M.P`".
2. **Copy in.** Parameters are processed left to right with a cursor starting at `S`:
   - scalars (`i32 i64 f32 f64 bool entity`) pass through unchanged; `i32`/`f32`/`bool`
     have their upper 32 bits cleared (SPEC "Changes from draft 1", item 7);
   - `str (ptr, len)`: read `len` bytes from C's memory with the checked `Caller` helpers;
     out of bounds or a negative pointer/length → trap in C; not UTF-8 → trap in C; written
     at `align8(cursor)` in M's memory; M receives `(that address, len)`;
   - `bytes (ptr, len)`: the same without the UTF-8 check, and remembered for copy-out;
   - `&S (ptr)`: `S.size` bytes (from the registered layout), placed at `align8(cursor)`,
     remembered for copy-out; M receives the address.
   If the cursor would pass `S + N`, the caller traps before M runs: ``argument `item` of
   `inventory.give` needs 9000 bytes of scratch, the mod has 4096``. A zero-length `str` or
   `bytes` gets the current cursor as its address.
3. **Run.** M's function runs from its budget (spawn by function index, one resume).
4. **Copy out** (only when it returned normally): every `bytes` and `&S` range is copied from
   M's scratch back to the original range in C's memory (still in bounds: memory never
   shrinks). On a trap, out of budget or yield nothing is copied back and C traps with M's
   message, as today.
5. **Results** are scalars and are passed back as they are.

Limits: scratch at most 1 MiB (verifier), so one call copies at most 2 MiB; an entity handle
is passed as is (handles are world-wide, not per memory); `ptr` cannot appear in a provide.
Nested calls work: while M runs it is busy, and a call from M to a third mod uses that mod's
scratch; a call back into M or C is refused as re-entrant, so a scratch region is never used
by two calls at once. M must not keep pointers into its scratch region after returning.

## 8. Several index files (E8.3)

`--api` may be repeated in `build`, `run`, `lsp`, `pack` and `debug`. Each value is an
`.efapi` file or an `.efmod` package (its index is derived from its manifest and section 13).
The indices are merged with `ApiIndex::merge`. Section 12 records the merged index's hash and
the version of every imported namespace, taken from whichever input defines it. The language
server accepts several indices the same way and, without `--api`, uses every `*.efapi` in the
workspace root plus the `[build] api` list of the nearest `mod.toml`.

## 9. Packages in the engine (E8.6–E8.12)

### 9.1 The `efemod` crate

`crates/efemod`, dependencies `efevm` (path) and `sevenz-rust2` (default features off; its
encoder (`lzma-rust2/encoder`) only through efemod's `write` feature); no I/O, `#![forbid(unsafe_code)]`, builds for `wasm32-unknown-unknown`.

```rust
pub struct Version { /* major, minor, patch, pre */ }        // FromStr, Ord, Display
pub struct VersionReq { /* comparators */ }                  // FromStr, matches(&Version)
pub struct Dependency { pub id: String, pub req: VersionReq }
pub struct Permissions { pub namespaces: Vec<String>, pub budget: Option<u64>, pub memory_pages: Option<u32> }
pub struct Manifest { pub id, pub version: Version, pub name, pub description, pub authors, pub license,
                      pub game: Option<String>, pub api_version: u32, pub dependencies: Vec<Dependency>,
                      pub permissions: Permissions }
impl Manifest { pub fn from_json(&str) -> Result<Self, PackageError>; pub fn to_json(&self) -> String;
                pub fn validate(&self) -> Result<(), PackageError>; }
pub struct ReadLimits { /* MOD_PACKAGE.md section 6 */ }
pub struct Package { pub manifest: Manifest, pub module: efevm::Module, pub module_bytes: Vec<u8>,
                     pub assets: BTreeMap<String, Vec<u8>>, pub ignored: Vec<String> }
impl Package {
    pub fn read(bytes: &[u8], limits: &ReadLimits) -> Result<Package, PackageError>;
    pub fn write(manifest: &Manifest, module_bytes: &[u8], assets: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, PackageError>;
    pub fn index(&self) -> efevm::ApiIndex;                   // Interface::index with the manifest
}
pub mod container; // the 7z profile, used by Package
pub fn resolve(candidates: &[Candidate<'_>], loaded: &[(&str, &Version)]) -> Resolution;
```

`Resolution { order: Vec<usize>, problems: Vec<(usize, ResolveProblem)> }` with
`ResolveProblem::{DuplicateId { id }, Missing { dep, req }, Version { dep, req, found },
Cycle { path: Vec<String> }, DependencyFailed { dep }}`. A dependency is satisfied by a
candidate of the set or by an already loaded package (a bare `load_mod` module has no
SemVer and never satisfies a dependency). The order is a topological sort with ties broken by
id; a cycle is reported once with its path (`a -> b -> a`) for each package on it.

### 9.2 Engine API

```rust
impl Engine {
    pub fn set_mod_policy(&mut self, policy: impl ModPolicy + 'static);
    pub fn load_package(&mut self, bytes: &[u8]) -> Result<ModuleId, ModError>;     // deps already loaded
    pub fn load_packages(&mut self, packages: Vec<(String, Vec<u8>)>) -> LoadReport; // label, bytes
    #[cfg(not(target_family = "wasm"))]
    pub fn load_mods_dir(&mut self, dir: impl AsRef<Path>) -> LoadReport;           // *.efmod, sorted, not recursive
    #[cfg(not(target_family = "wasm"))]
    pub fn watch_package(&mut self, id: ModuleId, path: impl AsRef<Path>) -> Result<(), ScriptError>;
    pub fn package(&self, id: ModuleId) -> Option<&PackageInfo>;                    // manifest + grant
    pub fn package_asset(&self, id: ModuleId, path: &str) -> Option<&[u8]>;
    pub fn unload_module_and_dependents(&mut self, id: ModuleId) -> Result<Vec<ModuleId>, ScriptError>;
}

pub trait ModPolicy { fn grant(&mut self, request: &ModRequest<'_>) -> Result<Grant, String>; }
impl<F: FnMut(&ModRequest<'_>) -> Result<Grant, String>> ModPolicy for F {}
pub struct ModRequest<'a> { pub manifest: &'a Manifest, pub imports: &'a [String] /* namespaces the module imports */ }
pub struct Grant { pub namespaces: Vec<String>, pub budget: u64, pub memory_pages: u32 }
pub struct DefaultPolicy;   // grants no namespace; budget and memory at the engine defaults

pub struct LoadReport { pub loaded: Vec<(String, ModuleId)>, pub failed: Vec<(String, ModError)> }
pub enum ModError {
    Package(efemod::PackageError),       // container, manifest, module decode/verify, undeclared import
    Resolve(efemod::ResolveProblem),
    GameMismatch { id: String, wanted: String, game: String },
    IdClash { id: String, namespace: String, owner: String }, // the id equals (or prefixes) a registered namespace
    Denied { id: String, reason: String },                   // the policy refused
    NotGranted { id: String, namespaces: Vec<String> },      // imports outside the grant
    Script(ScriptError),                                     // name taken, link errors, struct mismatch, ...
}
```

`Grant::namespaces` is intersected with the request (a policy cannot widen it by mistake);
the module's imports must then lie within the grant or the dependencies' namespaces. A
declared dependency's namespace needs no grant: the policy sees the dependencies in the
request and refuses the package if it does not want them, and a dependency's provides run
with the dependency's own grant and budget. The grant is stored once per package
(`PackageInfo::grant`) and is the allow-list of its one module, so every function of every
source file of the mod is under the same grant. Budget
and memory become the module's per-frame budget and `Limits::max_memory_pages`
(`ModOptions` gains `memory_pages`).

### 9.3 Load sequence of one package

1. `Package::read` (MOD_PACKAGE.md section 7).
2. `game` check; id clash check (the id must not equal or prefix a registered namespace).
3. Dependencies resolved against loaded packages (`load_package`) or the set (`load_packages`).
4. Policy: `ModRequest` → `Grant`; refusal → `Denied`; imports outside → `NotGranted`.
5. `Scripting::load_mod` with `ModOptions { name: id, version: api_version, budget,
   memory_pages, allowed: grant ∪ dependency ids }`: name check, link (by-name errors as today).
6. Section 13 registration (6), with rollback on failure.
7. `init` runs (as `Engine::load_mod` does today), so the next package in the order can link
   against this one's provides.
8. The `PackageInfo` (manifest, grant, assets) is stored in the module slot.

### 9.4 Assets

Assets stay in memory as the package's `BTreeMap`. The game reads them with
`Engine::package_asset(id, "items.json")`; a mod reads its own with `engine.mods.asset_len` /
`asset_read` (paths without the `assets/` prefix). No asset is written to disk by the engine.

### 9.5 Index integration

A loaded package's provides and own structs are in `Engine::api_index()` with owner `<id>`;
unload removes them. The E4.6 index test (`crates/core/tests/api_index.rs`) gains a case that
loads a package with provides and structs and checks both directions again (every callable
function is in the index with a doc, every index entry is callable) and that unload leaves the
index as it was.

## 10. Tooling

| Command | Does |
|---|---|
| `efescript build ROOT.efa [MORE.efa…] [--api F]… [-o out.efvm]` | follows `use` from each root; several roots are linked into one module |
| `efescript pack DIR [--api F]… [-o out.efmod]` | reads `DIR/mod.toml`, builds `[build] entry` with `--api` plus `[build] api`, checks the manifest against the module, adds `assets/`, writes the canonical package (default name `<id>-<version>.efmod`) |
| `efescript inspect FILE.efmod\|FILE.efvm` | prints manifest, dependencies, permissions, provides with signatures and docs, structs, assets with sizes, ignored entries (warnings), and section 12 |
| `efescript api-export FILE.efmod\|FILE.efvm --id ID [-o F]` | writes the mod's `.efapi` (`--id` only for a bare module) |
| `--api FILE.efmod` | everywhere `--api` is accepted (8) |
| `cargo xtask mods <example>` | packs `examples/<example>/mods/*/` against the example's `.efapi` (`target/efapi/<example>.efapi`) into `target/mods/<example>/`; `build`/`dist` run it and copy the packages to `<profile>/mods/` or `dist/<example>/mods/` |

The `bundle` command keeps its meaning (a standalone executable); packages are only `pack`.

## 11. Implementation plan

Sizes: **small** (one subagent, under ~300 changed lines, one area), **mid** (one subagent,
several files, needs care), **architectural** (touches a shared interface; do it with review,
and land it before its dependents). Tasks are ordered so shared foundations land first; tasks
in the same wave can run in parallel.

### Wave 1: foundations (independent)

| # | Task | Size | Files | Depends |
|---|---|---|---|---|
| T1 | **Section 13 in efevm**: `Interface`, `Provide`, encode/decode, verifier rules 1–8, builder method, `Interface::host_fns` / `index`, decode never-panics extended, remove "planned" from SPEC 9.2 | architectural | `crates/efevm/src/{module.rs, binary/{mod,encode,decode}.rs, verify.rs, builder.rs, lib.rs}`, new `crates/efevm/src/api/interface.rs`, `api/mod.rs`, `tests/api.rs` (or new `tests/interface.rs`), `SPEC.md` | – |
| T2 | **Public JSON reader** in efevm (`api::json::{Value, parse}` made `pub`, documented, depth limit kept) | small | `crates/efevm/src/api/json.rs`, `api/mod.rs` | – |
| T3 | **`ApiIndex::merge`** and repeated `--api` for `build`/`run`/`debug`/`lsp` (`.efapi` inputs only); `AsmOptions.apis` | small | `crates/efevm/src/api/index.rs`, `crates/efescript/src/{main.rs, asm/mod.rs, asm/lower/mod.rs, lsp/*}`, `tests/api_index.rs` | – |
| T4 | **efemod: versions and ranges** (`Version`, `VersionReq`, MOD_PACKAGE.md 5, table-driven tests); creates the crate and adds it to the workspace | small | new `crates/efemod/{Cargo.toml, src/lib.rs, src/version.rs}`, root `Cargo.toml` | – |

### Wave 2: linker and container

| # | Task | Size | Files | Depends |
|---|---|---|---|---|
| T5 | **Linker core**: `link::{Unit, Decls, SymbolMap, DefineCx, link}`, phase 1 layout and checks, phase 2 driver, section 13 assembly; unit tests with hand-written test units (not `.efa`) | architectural | new `crates/efescript/src/link/{mod.rs, layout.rs, tests.rs}`, `src/lib.rs` | T1 |
| T6 | **efemod: container profile** (built as ZIP, then replaced by 7z; see D1) reader and canonical writer, CRC-32, limits, entry-name rules, rejection tests, never-panics test | mid | `crates/efemod/src/{zip.rs, crc.rs, limits.rs}`, `crates/efemod/tests/zip.rs`, `crates/efemod/Cargo.toml` (miniz_oxide) | T4 |

### Wave 3: front end, manifest, engine exports

| # | Task | Size | Files | Depends |
|---|---|---|---|---|
| T7 | **Multi-file `.efa`**: lexer (`&`, reserved words), parser (`use`, `pub`, `provide`, `struct`, `scratch`), lowering split into an `asm::Unit` (declare/define), `FileSource` with `FsSource`/`MemSource`, `assemble_program`, per-file diagnostics; single-file byte-identical (existing round-trip tests must pass unchanged); new tests for every rule of efa-syntax 16; remove "planned" there | architectural | `crates/efescript/src/asm/{lexer.rs, parser.rs, ast.rs, mod.rs, diag.rs, lower/*}`, new `src/asm/source.rs`, `tests/v2_multifile.rs`, `docs/efa-syntax.md` | T5 |
| T8 | **efemod: manifest and package**: `Manifest` JSON read/write and validation, `Package::{read, write, index}`, manifest-vs-module rules, never-panics on mutated manifests | mid | `crates/efemod/src/{manifest.rs, package.rs, lib.rs}`, `crates/efemod/tests/package.rs` | T1, T2, T6 |
| T9 | **Engine: declared exports** replacing `engine.mods.export`: registration at load with rollback, trampoline by name (scalars only in this task), struct copies check, `engine.mods` v2 without `export`, `Needed` on unload + `unload_module_and_dependents`, hot-reload rules of section 6; migrate tests and `script_demo`'s mod to `provide` built with `ModuleBuilder` (or `.efa` once T7 is in) | mid | `crates/core/src/engine/scripting/{mods.rs, host.rs, api_engine.rs, mod.rs}`, `crates/core/src/engine.rs`, `crates/core/tests/{scripting.rs, api_index.rs}`, `examples/script_demo/{scripts/mod_adder.efa, scripts/mod_user.efa, tests/e2e.rs}` | T1 |

### Wave 4: data across mods, resolution, tools

| # | Task | Size | Files | Depends |
|---|---|---|---|---|
| T10 | **Copy ABI** (section 7) in the trampoline: copy in/out through the scratch region, UTF-8 check, budget charge, named traps; tests for each rule, nested calls, re-entrance | mid | `crates/core/src/engine/scripting/{host.rs, mods.rs}`, `crates/core/tests/scripting.rs` | T9 |
| T11 | **efemod: `resolve`** (pure dependency resolution, problems by name, deterministic order) | small | `crates/efemod/src/resolve.rs`, `crates/efemod/tests/resolve.rs` | T4 |
| T12 | **`efescript pack`, `inspect`, `api-export` for mods, `--api x.efmod`**; `mod.toml` with the `toml` crate (unknown keys rejected) | mid | `crates/efescript/Cargo.toml` (efemod, toml), `src/main.rs`, new `src/pack.rs`, `tests/pack.rs` | T3, T7, T8 |

### Wave 5: engine packages and integration

| # | Task | Size | Files | Depends |
|---|---|---|---|---|
| T13 | **Engine packages**: `ModPolicy`, `DefaultPolicy`, `load_package`, `load_packages`, `load_mods_dir` (native), `watch_package`, `PackageInfo`, assets and `engine.mods.asset_*`, `ModError`, `LoadReport` | mid | `crates/core/Cargo.toml` (efemod), new `crates/core/src/engine/scripting/packages.rs`, `scripting/{mod.rs, mods.rs, host.rs, api_engine.rs}`, `crates/core/src/engine.rs`, `crates/core/tests/packages.rs` | T8, T9, T11 |
| T14 | **Language server and debugger across files** (section 3) | mid | `crates/efescript/src/lsp/*`, `src/debugger/*`, `tests/{v2_lsp.rs, v2_debugger.rs}` | T7 |
| T15 | **xtask and example**: `cargo xtask mods`, `build`/`dist` copy packages; `examples/script_demo/mods/{adder,user}/` as folders with `mod.toml`, e2e test loading both packages through `load_packages` with a policy | small | `xtask/src/*`, `examples/script_demo/{mods/**, src/*, tests/e2e.rs}` | T12, T13 |
| T16 | **Index test for packages** (9.5) and docs: tick E8 items, remove "planned" markers in MOD_PACKAGE.md, record deviations in a "Changes recorded during implementation" section here | small | `crates/core/tests/api_index.rs`, `crates/efescript/ROADMAP.md`, `crates/efevm/MOD_PACKAGE.md`, this file | T13 |

Critical path: T1 → T5 → T7 → T12 → T15. T9/T10 and the efemod chain (T4 → T6 → T8, T11)
proceed beside it.

## E8.13 Typed script events

Script events were the one script-visible thing outside the API index: `engine.events.send`
took a string name and raw bytes, so nothing could check them. Decision (format and ABI in
efevm; normative: SPEC.md 9.2 rule 9 and 9.3, API_INDEX.md "Events"):

- **Registered, so indexed.** `HostRegistry::register_event(EventDesc { namespace, name,
  payload: Option<struct name>, doc, owner })` with the same rules as functions (defined
  namespace, owner from it, identifier name, doc, registered payload struct, no full-name clash
  with a function). `ApiIndex.events` is hashed, merged and written; `.efapi` is format
  version 2 and readers accept only 2. Reason: "registering is the only way to be
  script-visible" now holds for events too, and the assembler, LSP and debugger read events
  from the same file as everything else.
- **Resolved at link time, through section 14.** A module lists the events it sends or reads in
  a new known section 14 `events` (`namespace, name, payload struct + size`); `Imports::
  check_module` / `Instance::new` report `LinkError::UnknownEvent` and `EventPayload` by name,
  collected with the other link errors. Reason: the same guarantee as imports (a mismatch fails
  before anything runs, never at the first `send`), and the payload size the engine copies is
  agreed on once instead of being passed by scripts. A known section, not a custom one, for the
  same reason as section 13: it must never be silently skipped.
- **Ids are per module.** An event's id is its index in section 14; the engine ABI is
  `engine.events.send(ev: i32, payload: ptr)`, `count(ev: i32) -> i32` and `read(ev: i32,
  index: i32, out: ptr) -> i32`, payloads copied by the registered layout's size (no-payload
  events take ptr 0), at most 4096 events per frame with drops counted and logged once per
  frame. Reason: ids that do not depend on registration order or other modules, no strings on
  the hot path, and an id outside the module's list is a clean trap.
- **Permissions cover event namespaces.** Before instantiating a mod, the engine checks every
  namespace of `ModuleData::event_namespaces()` against the allow-list with the same prefix rule
  as imports; the grant is whole-package (section 13 item 2). Reason: sending or reading an
  event is an API use like a call, and section 14 makes the list visible before anything runs.
- **Mods declare their own events in section 13.** `Interface.events: Vec<ProvidedEvent { name,
  payload, doc }>` in the mod's own namespace; the payload struct must be listed in the
  interface. The engine registers them with `Interface::event_descs(id)` together with the
  mod's namespace and own structs **before** it link-checks the mod's own section 14 (with the
  usual rollback), so a mod can send its own events; `Interface::index` includes them.
  Dependents ("X depends on Y") and the hot-reload compatibility check also count section 14
  references to Y's events, not only imports.
- **Assembler (later task).** `.efa` writes `import event game.score_changed`; the payload struct
  and size come from `--api`, and the namespace is also recorded in section 12.

## 12. Later (not part of E8)

- An on-disk object format (`.efo`) for the linker's units, for build caching or prebuilt
  libraries linked statically.
- Optional dependencies (needs imports that may stay unresolved).
- Package signing; a mod repository or download tool.
- `str`/`bytes` results across mods (would need a callee-side allocator or an out buffer
  convention beyond `bytes` parameters).
- Direct VM-level linking between separately loaded modules.

## 13. Decisions confirmed by the project owner (2026-11-15)

These were open policy points in draft 1; the owner decided them before implementation, and
they override anything earlier in this document that disagrees:

1. **No backward compatibility.** The project is not in production. `engine.mods.export` is
   removed outright with no deprecation path or migration shim, and no design choice is made
   only to be "refused by older engines".
2. **Permissions apply to the whole package.** What the game grants a `.efmod` covers the
   package as one unit (its one module and everything in it); there is no per-file or
   per-entry permission.
3. **The default policy is deny.** `DefaultPolicy` grants no namespace; a game must install a
   `ModPolicy` to give mods any. This matches E5's rule that an empty allow-list allows
   nothing: the package only requests, the game grants.
4. **A mod id may not equal a registered namespace** (the game's own such as `game`, `engine`,
   `std`, `efe`, or another mod's). The loader refuses such a package when it is loaded, with
   an error naming the clash (`ModError::IdClash`), not only when its functions are
   registered.
5. **The container is 7z** (`sevenz-rust2`, reader only on the engine's path), replacing the
   restricted ZIP reader of draft 1 (D1).
6. Kept from draft 1: unload refused by default
   with an explicit cascade; one module per package; reserved words `use`, `as`, `pub`,
   `provide`, `struct` and `scratch`.

## 14. Changes recorded during implementation

T5 and T7 (linker and multi-file `.efa`), 2026-11-16:

1. **`Unit` shape.** Methods take `&mut self` (units keep their state between phases) and
   write to a diagnostics sink instead of returning `Result`, so module-level and body errors
   are still reported together, as the frozen single-file tests require. A third method,
   `resolve`, was added: global initial values, data bytes and table entries can hold data
   addresses and `A.f`, so they cannot be part of `Decls`, and they must be written before
   any body (builder order). `diagnostic(span, msg)` lets the linker render its own errors
   with the unit's source line.
2. **`Decls` has no `uses`.** The driver resolves `use` paths to unit indices before the units
   are built; the linker never needs them. `Decls::table` is the span only; `MemoryDecl::min`
   is optional (an operand error is already reported).
3. **Visibility is checked when a reference is resolved**, by `SymbolMap::lookup` and by the
   asm unit's `alias.item` tables (filled in `resolve`), not as a separate step of phase 1.
   Private items get ``` `x` is private to `file` ``` with the note "declared at FILE:LINE;
   mark it `pub` to use it from other files".
4. **`AsmOptions` is unchanged** (`{ api }`): adding `mod_id` (or `apis`) would break the
   frozen tests that build `AsmOptions { api: .. }` as a literal. Provide diagnostics name the
   provide without the mod id; several `--api` are merged by the CLI into one index (T3).
5. **Layout positions use own consts only**: `resb`, `memory`, `scratch` and struct array
   lengths accept literals and the file's own consts, not `A.X`, because they are evaluated
   in `declare`.
6. **File names.** `FileSource` has `display_name(path)`; `FsSource::for_file(path)` roots the
   source at the file's directory and keeps that directory as typed in front of every name,
   so `build examples/x.efa` records `examples/x.efa` exactly as before (byte identity) and
   used files appear as `examples/util/m.efa`, which backtraces can open. MemSource names are
   the normalised paths.
7. **Several roots** are supported (`assemble_programs`, `build a.efa b.efa`): the extra roots
   must lie in the first one's directory and are numbered after the first one's use graph.
8. **Section 13 is optional**: written only with a provide, an exported struct or a scratch
   region (an empty `Some` would change the bytes of old programs). All `export struct`s are
   listed (not only those a provide uses), then the index copies in first-use order.
9. A struct name shares the value name space (a data item or const of the same name is a
   duplicate). An explicit `scratch N > 0` reserves the region without any provide.
10. The parser message for a bad word after `export` keeps its E2 wording ("`fn` or
    `global`") although `export struct` is accepted, because a frozen parser test checks it.
11. `run`, `debug`, `dis` and the language server follow `use` since T14 (`lsp/project.rs`
    serves open buffers before disk; the program root is the nearest `mod.toml`'s
    `[build] entry`). `bundle` still embeds one file. Not covered by the language server: a
    file in the entry directory that nothing `use`s gets only its own syntax errors, and
    `mod.toml` edits take effect on save.

Other tasks, 2026-11-15/16:

12. **T3.** No `AsmOptions.apis`: the CLI merges every `--api` with `ApiIndex::merge` into one
    index. Merge conflicts name the item and both games.
13. **T1.** `Interface::index` sets the index's `game` to the mod id; verifier rule 7 also
    requires unique component ids, so a mod's index always validates.
14. **T4.** Partial comparators follow Cargo exactly: `>=1.2.5-rc.1, <=1.2` does not match
    `1.2.5-rc.2`, although a literal reading of MOD_PACKAGE.md 5.2 (`<=1.2` = `<1.3.0`) says
    it would.
15. **Container (T6).** Built first as the restricted ZIP of draft 1; an independent review
    (Gemini) found an unchecked `data_start + csize` overflow on wasm32 and a misplaced ZIP64
    check, both fixed, and then the owner switched to 7z (D1) and the ZIP code was deleted.
    Findings about `sevenz-rust2` 0.25.0 / `lzma-rust2` 0.25.0 that the reader guards against
    from the outside:
    - no dictionary cap by default (`Archive::read` passes `usize::MAX/1024` KiB,
      reader.rs:19, 456-457): efemod checks the LZMA2 property byte against
      `ReadLimits::dictionary_size` (64 MiB) before decoding and passes a matching memory
      limit;
    - an encoded header would run a full decoder chain (reader.rs:672-786): only a plain header
      of at most `ReadLimits::header_size` (4 MiB), CRC-checked, is accepted;
    - `files` (~112 bytes each) is allocated from the header count before per-file data is read
      (reader.rs:821-826): efemod's own header walk refuses more than `entries` (4096) files,
      folders or pack streams first;
    - a damaged start header makes the crate scan backwards with repeated `init_archive` and no
      CRC check (reader.rs:490-504, 578-627), a CPU and allocation denial of service: efemod
      checks the start-header CRC first;
    - only COPY or LZMA2, one coder and one entry per folder, packed streams contiguous;
      AES, encrypted headers, PPMd, BZip2, Deflate, LZMA, BCJ and Delta are refused (tested
      with the `7z` CLI and hand-edited coder ids);
    - every entry is read through `take(declared + 1)` and must match its size exactly;
    - the never-panics test (about 42 000 mutated and random inputs, ~15 000 of them reaching
      the crate's parser) found no panic. That is evidence, not proof.
    `sevenz-rust2`'s `ArchiveWriter` always LZMA-compresses the header (writer.rs:511-621),
    which the reader refuses, so efemod has its own canonical writer on
    `lzma_rust2::Lzma2Writer`; the `write` feature is `lzma-rust2/encoder`, not
    `sevenz-rust2/compress`. In a whole-workspace build Cargo unifies features, so core also
    gets the encoder compiled there; core's own and the web game's builds are encoder-free.
    `efemod` tests enable `write` through a self dev-dependency.
16. **T8.** `Package::write` validates the manifest and asset names but not the module; the
    packer calls `check_module`. Dependency ids follow the mod-id rule, so a dependency on
    `engine`, `std` or `efe` is invalid.
17. **T9/T10.** Registration of provides runs as a dry run on a scratch registry first, so a
    failed load registers nothing and uses no module id. `HostRegistry` removes only by
    owner, so a reload that drops a provide, an event or changes an own struct rebuilds
    everything the mod owns (refused if another owner's function uses one of its structs).
    New errors: `ScriptError::{StructMismatch, NotAMod, Needed, ReloadRefused}`. A review
    (Gemini) of the copy ABI found only that a zero-length `str`/`bytes` skipped the pointer
    check; fixed.
18. **T13.** A package whose id is already used by a loaded module fails `Script(NameTaken)`
    before the id-clash check; reserved ids give `IdClash` with owner `(reserved)`.
    `load_package` takes dependencies only from loaded packages (a bare `load_mod` module
    never satisfies one). A package reload also requires every loaded dependent's range to
    accept the new version.
19. **E8.13.** `engine.events` is a namespace like any other: a mod must be granted it to send
    or read events (the script_demo mods request it). Rust-side script events
    (`Engine::send_script_event`) are checked against the registry and do not count against
    the per-frame cap. `ModRequest` has a separate `events` list. A mod sends its own event
    by importing `<id>.name(&S)` with an explicit payload (the assembler does not know the
    id). The engine registers no `engine.*` events yet.
20. **T15.** The script_demo mods request `engine.log`/`engine.events` (inventory) and
    `engine.events`/`game` (shop); the game gained `game.report(v: i64)` so the test can read
    what a mod computed, because the engine has no Rust API to read a module's globals or call
    a provide. `efescript pack` writes to the current directory by default.
21. **Reversible decisions taken by the manager:** a declared dependency's namespace needs no
    grant (the policy sees the dependencies); `DefaultPolicy` returns an empty grant rather
    than refusing (so a package that only uses its dependencies loads).

