# efescript E3–E5 design: host registry, API index, tooling, engine integration

Scope: roadmap items E3, E4 and E5 (`crates/efescript/ROADMAP.md`) and the parts of the
repository roadmap they need (3.1–3.4, 5.1–5.5). The rule that drives everything is
"The API index" in the efescript roadmap: **registering is the only way to make a host
function callable, and registering puts it in the index.** The file format is
`crates/efevm/API_INDEX.md`.

## 1. Where things live

| Piece | Crate / path | Depends on |
|---|---|---|
| Type descriptions (`ApiType`, `HostFn`, `StructDesc`, …) | `efevm::api::types` | – |
| `HostRegistry<H>`, `Args`, `Results` | `efevm::api::registry` | – |
| `ApiIndex`, JSON writer and reader, hash | `efevm::api::index`, `efevm::api::json` | – |
| Module section 12 `api` (index hash + namespace versions) | `efevm` binary format | – |
| `std` host on the registry (`std`, `std.time`, `std.fs`) | `efescript::host_std` | efevm |
| Assembler `--api`, `efescript api`, `api-export` | `efescript::asm`, CLI | efevm |
| Language server | `efescript::lsp` (`efescript lsp`) | efevm, `lsp-server`, `lsp-types` |
| Debugger | `efescript::debugger` (`efescript debug`) | efevm |
| World, `Commands`, components, type registry | `efeengine-core` (`engine::world`, `engine::commands`, `engine::types`) | hecs, glam, bytemuck |
| Engine host functions, `Script`, `ScriptSystem`, mods | `efeengine-core` (`engine::scripting`) | efevm only |
| `cargo xtask api-export`, `.efapi` next to builds | `xtask` | runs the game |

`efevm` keeps zero dependencies and no I/O and still builds for `wasm32-unknown-unknown`.
The engine never depends on the `efescript` crate.

## 2. The registry (E3.1, E3.3)

```rust
let mut reg: HostRegistry<MyHost> = HostRegistry::new();
reg.define_namespace("std", 1, "std", "Console output and process control.")?;
reg.register(
    HostFn::new("std", "println")
        .doc("Write `text` and a newline to standard output.")
        .param("text", ApiType::Str),
    |caller, args, _res| {
        let text = caller.read_str(args.str(0)?)?;   // checked, never panics
        caller.host().out.push_str(&text);
        Ok(HostOk::Continue)
    },
)?;
let imports = reg.imports();                         // snapshot; the only way to get Imports
let inst = Instance::new(&module, &imports, Limits::default())?;
```

- `define_namespace(name, version, owner, doc)`. Redefining with the same owner updates
  version and doc; with a different owner it is an error.
- `register(HostFn, f)`: the namespace must exist; the function's `owner` is the namespace's
  owner; the doc must be non-empty; `Str`/`Bytes`/`Struct` only as parameters; a
  `Struct(name)` must be a registered type; parameter and result names are identifiers and
  unique within the function; registering an existing `namespace.name` is an error
  (`replace` exists for hot reload: same rules, replaces in place).
- `register_type(StructDesc)`: validated with `StructDesc::validate`; names are unique;
  component ids are unique.
- `unregister_owner(owner)` removes that owner's namespaces, functions and types (a mod unload).
  Functions in *other* namespaces that take `&Type` of a removed type keep working; the
  type is only refused if another owner still references it (then the call is an error and
  nothing is removed).
- `imports()` builds an `Imports<H>`: a cheap snapshot (`Arc` clones). `Imports` has no
  public way to add a function. `Imports::names()` lists what is callable; the index test
  (E4.6) compares it with `index()`.
- `index()` returns the plain-data `ApiIndex`; `index().to_json()` is the file.

Host function form: `Fn(&mut Caller<H>, &Args, &mut Results) -> Result<HostOk, Trap>`.
`Args` knows the declared parameter types, so `args.i32(i)`, `args.i64(i)`, `args.f32(i)`,
`args.f64(i)`, `args.bool(i)`, `args.entity(i)` (u64), `args.ptr(i)`, `args.str(i)` /
`args.bytes(i)` (a `GuestSlice { ptr, len }`) take the *logical* parameter index and return
`Err(Trap::Host)` on a wrong index or type. `Results` has the matching `set_*`. Results not
set are 0.

Linking (E3.3): `Instance::new` resolves every import by `namespace.name` and exact lowered
signature; it also checks namespace versions recorded in the module's `api` section.
`Imports::check_module(&Module) -> Vec<LinkError>` returns every problem at once, by name,
including the hash the module was built against when it differs. New `LinkError`
variants: `UnknownNamespace`, `NamespaceVersion { namespace, module, host }`, and
`ImportTypeMismatch` now carries both signatures as text.

## 3. Guest memory helpers (E3.4)

On `Caller`: `read_bytes(GuestSlice) -> Vec<u8>`, `read_str(GuestSlice) -> String`
(invalid UTF-8 is `Trap::Host`), `write_bytes(ptr, &[u8])`, `read_struct(ptr, &StructDesc)`
and `write_struct(ptr, &StructDesc, &[u8])` (length must equal `size`), `read_u32/u64/f32/f64`
and `write_*` (little-endian). All are bounds-checked with checked arithmetic; negative
`i64` pointers or lengths are out of bounds; nothing allocates more than the guest memory size.

## 4. std host (E3.5)

`efescript::host_std` builds a `HostRegistry<StdEnv<H>>` (`std_registry(&StdOptions)`),
owner `std`. Namespaces:

| Namespace | Functions |
|---|---|
| `std` (v1) | `print(text: str)`, `println(text: str)`, `print_i64(v)`, `print_f64(v)`, `print_char(c: i32)`, `read_line(buf: bytes) -> (len: i64)`, `play_sound(path: str) -> (status: i32)`, `exit(code: i32)` |
| `std.time` (v1) | `now_ms() -> i64` (monotonic, since the program started), `unix_ms() -> i64` |
| `std.fs` (v1) | `read_file(path: str, buf: bytes) -> (len: i64)`, `write_file(path: str, data: bytes) -> (status: i32)` |

The lowered signatures of the `std` functions are the same as before (`str` = `i64, i64`),
so existing `.efa` files keep working. `std.fs` is always registered (so the index does not
depend on flags) but is **denied unless a root directory is given** (`--fs-root DIR` on the
CLI, `StdOptions::fs_root`): denied calls return `-2`. Paths are relative, may not contain
`..` or be absolute, and are resolved under the root after canonicalisation; anything that
escapes returns `-2`. I/O errors return `-1`. The engine never links `std`.

## 5. Legacy removal (E3.6)

`src/nasm_efe`, `src/runtime`, `ops.rs`, `instruction.rs`, `syscalls.rs`, `funs.rs`,
`macros.rs`, the `.efe` examples and their tests are deleted, and so is the
`efepages`/`proc_macros` dependency of efescript (the crates stay in the workspace). The
`Host` output/sound trait with `StdHost` and `CaptureHost` moves to `src/host.rs`. Examples
without an `.efa` equivalent are ported: `call.efe` → `exit.efa` (`std.exit`), `test.efe`
→ `echo.efa` (`std.read_line`), `create_e.efe` → `write_file.efa` (`std.fs`). `heap.efe`
(a documented fault) is dropped. The CLI answers `.efe`/`.efec` with "the legacy format
was removed".

## 6. Index export and modules (E4.1, E4.2)

- `efescript api FILE.efapi` prints namespaces, types and functions with signatures and docs.
- `efescript api-export [-o std.efapi]` writes the index of the CLI's std registry.
- A module assembled with `--api` gets section 12 (`api`): the index hash and the version of
  every namespace it imports from. Modules assembled without `--api` have no section 12 and
  link exactly as before.

## 7. Assembler against an index (E4.3)

`efescript build|run FILE.efa --api GAME.efapi`, library `asm::assemble_with(src, file, &AsmOptions { api })`:
- each `import ns.name(...)` must exist in the index ("did you mean" over all qualified
  names) and its declared signature must equal the lowered one; the diagnostic shows the
  index signature with parameter names;
- with an index, `import ns.name` may omit the signature, which is then taken from the index;
- every struct `S` in the index provides constants `S.size`, `S.align`, `S.<field>` (byte
  offset) and, for components, `S.id`;
- call arity is checked against the import as before;
- section 12 is written.

## 8. Language server (E4.4) and debugger (E4.5)

`efescript lsp [--api FILE]` speaks LSP over stdio (`lsp-server` + `lsp-types`). Without
`--api` it looks for a single `*.efapi` in the workspace root. Features: diagnostics on open
and change (the assembler's, with `--api` checks), go to definition for labels, functions,
data and constants, completion of host functions (`ns.name` with signature and doc) and
mnemonics, hover for host functions, struct constants and mnemonics. The `.efapi` file is
reloaded when its modification time changes (checked on every request) and on
`workspace/didChangeWatchedFiles`.

`efescript debug FILE.efa|FILE.efvm [--api FILE]` is a line-oriented debugger on stdin/stdout
(scriptable, so it is testable): `break LINE|FUNC`, `delete N`, `step`, `next`, `continue`,
`regs`, `mem ADDR LEN`, `bt`, `list`, `quit`. Single-stepping is `resume` with budget 1;
`efevm` exposes the current location, frame registers and call stack of a paused fiber.
A call to an import is shown by its index name and parameter names
(`call std.println(text = r0..r1)`).

## 9. Engine world (repository roadmap 3.1–3.4)

- **3.1** `glam` with its `bytemuck` feature; components `Transform { position: Vec2,
  rotation: f32, scale: Vec2 }`, `Velocity { linear: Vec2, angular: f32 }`,
  `MeshHandle { id: u32 }` are `#[repr(C)]`, `Pod`, and derive `ScriptType`; `Name(String)`
  is a Rust-only component.
- **3.2** `EngineState.world` is a `hecs::World` (re-exported as `efeengine_core::hecs`).
  `GameObject`, `EngineWorld` and `add_object` are removed. The placeholder renderer draws one
  triangle per `(Transform, MeshHandle)` at `transform.position` (the thinnest adapter; the
  real extract is 3.5).
- **3.3** `Commands` (in `Ctx` as `cx.commands`) holds engine control requests (quit, title,
  frame rate, cursor, text input: the old `EngineCommands`, kept as a type alias) and world
  changes: `cx.spawn(bundle) -> Entity` (the entity is reserved at once), `despawn(e)`,
  `insert(e, component)`, `remove::<T>(e)`, plus byte-level `insert_raw(e, id, bytes)` /
  `remove_raw(e, id)` used by scripts. They are applied in order after every stage, so
  despawning inside a query is safe; operations on a dead entity are ignored.
- **3.4** `TypeRegistry` (`engine.register_component::<T>()`), per type: stable name
  (`T::NAME`), numeric id (`fnv1a32(name)`, a collision is an error), size, alignment, the
  field layout as an `efevm::StructDesc`, and byte-level read / write / insert / remove
  functions built from `bytemuck::pod_read_unaligned` (lengths checked first). The derive
  macro `#[derive(ScriptType)]` in `efeengine-macros` generates the layout from
  `offset_of!` and field types implementing `ScriptField` (integers, floats, arrays, glam
  vectors; `#[script(entity)]` marks a `u64` field as an entity). Doc comments become docs.
  Registering a component also registers its `StructDesc` in the scripting registry: one
  call, both tables, no hand-kept copy. Types without a hecs component (for mod APIs) can be
  registered at runtime with `register_struct(StructDesc)`; hecs component types are never
  created at runtime.

## 10. Scripting in the engine (E5)

- **One VM per world.** `Scripting` (an engine field, reachable from systems through
  `Ctx`) owns the `HostRegistry<ScriptHost>`, the `TypeRegistry` link, and every loaded
  module with its own `Instance` (own memory). There is no instance per entity; script data
  lives in the module's memory.
- **Host context.** While scripts run, `ScriptHost` owns the world (moved in with
  `mem::take` and moved back afterwards), a `Commands` buffer (appended to the stage's
  commands), a copy of `Time`, a snapshot of input, a log buffer and script events. No
  `unsafe`, no raw pointers.
- **Engine namespaces** (owner `engine`, version 1; full list and docs come from the index):
  `engine.world` (`spawn`, `despawn`, `alive`, `get`, `set`, `has`, `insert`, `remove`,
  `query`, `component_id`, `changed`, `tick`), `engine.input` (`key_down`, `key_pressed`, `key_released` by key
  name, `mouse_x`, `mouse_y`, `mouse_down`), `engine.time` (`delta`, `elapsed`, `frame`,
  `fixed_delta`), `engine.events` (`send(name: str, data: bytes)`, `count(name: str)`,
  `read(name: str, index: i64, buf: bytes) -> len`), `engine.log` (`info`, `warn`, `error`),
  `engine.mods` (`export(func: str, doc: str) -> status`). Status codes: 0 ok, 1 stale
  entity, 2 component missing, 3 unknown component id, 4 wrong size, -1 other.
- **`Script { module: ModuleId, entry: String, state: i64 }`** component; the `ScriptSystem`
  runs first in `Update` and calls `entry(entity: entity, dt: f32, state: i64) -> i64` for
  every entity with a `Script`, storing the returned state. A module may also export
  `frame(dt: f32)` (once per frame) and `init()` (once after loading).
- **Budgets.** Each module has a per-frame budget (default 1 000 000 units). A call that runs
  out is abandoned for this frame with a warning, and the rest of that module's scripts are
  skipped this frame. A trap is logged with its backtrace; the script runs again next frame.
- **Mods (E5.3).** `Scripting::load_mod(bytes, ModOptions { name, version, budget,
  allowed: Vec<namespace prefix> })`. Load fails with a clear error if an import's
  namespace is not allowed, or on any link error. During `init` a mod may call
  `engine.mods.export(func, doc)`: after `init` returns, the export is registered as
  `<mod name>.<func>` in namespace `<mod name>` (owner = the mod), with parameter names from
  the module's `names` section (or `p0`, `p1`, …) and scalar types from the export's
  signature. Calling it from another module runs the mod's instance (from the mod's
  budget); re-entering a module that is already running is a clean trap. `unload_mod`
  removes the instance and calls `unregister_owner`, so its functions leave the index;
  modules that still import them get a clean trap on call. Hot reload (native only):
  `watch(module, path)`: when the `.efvm` file's modification time changes, the new module
  is linked and swapped in (the old one keeps running if that fails), its mod exports are
  re-registered, and the states of its `Script`s are reset to 0.
  *Note (E8):* `engine.mods.export` was removed without a shim. A mod now declares the
  functions it offers as `provide` items in its section 13 `interface`, and the engine
  registers them when the mod loads (see `e8-design.md` D6).
- **Index export.** `Engine::api_index()` / `Engine::export_api_index() -> String` (JSON)
  include everything registered so far, mods included. With the environment variable
  `EFE_API_EXPORT=<path>` the engine runs headless and `game_loop()` writes the index to
  `<path>` and returns instead of running; `cargo xtask api-export` uses that.

## 11. Build integration (E5.4)

`cargo xtask api-export <example> [-o FILE]` builds the example natively and runs it with
`EFE_API_EXPORT`. `cargo xtask build` and `dist` write `<example>.efapi` next to the
binary / into `dist/<example>/`, and assemble `examples/<example>/scripts/*.efa` against it
into `scripts/*.efvm` beside the game. The example `script_demo` loads them at run time.

## 12. Tests

- E4.6 index test, for the `std` registry (`crates/efescript/tests/api_index.rs`) and the
  engine registry (`crates/core/tests/api_index.rs`): every name in `imports().names()` has
  an index entry with a non-empty doc, and the JSON round-trips with the same hash.
- End to end (`examples/script_demo/tests/e2e.rs`): export the index, assemble a `.efa`
  against it, run it under `ScriptSystem` headless and check that a `Transform` changed; load
  a mod that exports a function, see it in a re-exported index, call it from another module,
  unload, and see it gone.

## Changes recorded during implementation (std host, CLI, assembler `--api`)

- `std_registry` parameter names: `print(text)`, `println(text)`, `print_i64(v)`,
  `print_f64(v)`, `print_char(c)`, `read_line(buf) -> (len)`, `play_sound(path) -> (status)`,
  `exit(code)`, `now_ms() -> (ms)`, `unix_ms() -> (ms)`, `read_file(path, buf) -> (len)`,
  `write_file(path, data) -> (status)`. The index game name is `std`. Lowered signatures are
  unchanged. `StdEnv` stores the start `Instant`; `Imports::func`, `std_imports`,
  `add_std_imports` and `STD_FUNCS` are gone (nothing wraps them).
- `std.fs.read_file` returns the number of bytes copied (at most the buffer size, not the file
  size). A directory, a missing file or any I/O error is -1. `write_file` needs an existing
  parent directory (-1 otherwise), follows an existing symlink at the target only when it stays
  inside the root, and returns -2 for a dangling symlink. Empty paths are -2. The root is
  canonicalised on every call (a missing root is -1). Checks are made before the write, so a
  concurrent symlink swap is not defended against (time-of-check/time-of-use).
- `std.print_char` with an invalid code point still traps; `std.play_sound` with a non-UTF-8
  path returns 1 as before.
- `run_v2::run` keeps its signature (no `std.fs` root); `run_with(prog, env, budget,
  &StdOptions)` takes the options, `load_path_with` / `load_source_with` take `AsmOptions`.
  Link problems of `run` are all printed, one per line, from `Imports::check_module`
  (`error: cannot link the module:` followed by the `LinkError` texts). A bundled executable
  (`run_embedded`) runs with `std.fs` denied.
- CLI: `run|build FILE --api FILE.efapi`, `run --fs-root DIR`, `api FILE.efapi`,
  `api-export [-o FILE]`. `run` links against the std host only, so a module that imports
  engine functions fails to link there with the by-name list.
- Assembler: `ast::ImportDecl` gained `has_sig: bool` (`import ns.name` without a parameter
  list; `params`/`results` are then empty). `asm::{AsmOptions, assemble_with}` as specified.
  Without an index such an import is the diagnostic "has no signature". With an index the
  struct constants are ordinary integer `const`s (also valid for `memory`/`resb` sizes); a
  field named `size`, `align` or `id` loses against the structural constant of the same name;
  a data symbol `S` is an error when a field of struct `S` is named `len` (it would collide
  with the derived `S.len`). Section 12 lists only the namespaces of the module's imports
  (sorted), with the versions from the index.
- The disassembler prints section 12 as the first line, `; api fnv1a64:<hash> ns@ver ...`. A
  comment cannot restore the section, so `dis` then `asm` without `--api` drops it; with the
  same index the round trip is byte-identical (tested in `tests/api_index.rs`). The example
  round-trip tests assemble without an index, so they are unaffected.
- `examples/write_file.efa` (port of `create_e`) writes `haha` below `--fs-root`; without a root
  it prints `denied` and exits 1.

## Changes recorded during implementation: section 10 (engine scripting, `crates/core`)

- **Where it lives.** `EngineState.scripting` (crate-private field, `Engine::scripting()` /
  `scripting_mut()`) and `Ctx.scripting`. The script system is added by `Engine::with_config`,
  so it is the first system of `Stage::Update`. `Scripting::registry()` is read-only: no public
  `&mut HostRegistry`, so `register_type` is not reachable except through
  `Engine::register_component[_owned]` / `register_struct`. `TypeRegistry::register*` are now
  `pub(crate)` (a `compile_fail` doctest checks it); `TypeRegistry` and `TypeEntry` are `Clone`
  and carry a generation counter, and `ScriptHost` keeps a copy refreshed when the generation
  changes. `engine.world.*` only resolve ids through that copy, which has exactly the types in
  the index (`Engine::unload_module` removes a mod's types from both).
- **Loading.** `Scripting::load_module` / `load_mod` only instantiate; `init` runs at the next
  script-system run. `Engine::load_module` / `load_mod` / `load_module_file` additionally run
  pending `init`s at once with the engine's world, so a mod's exports are in the index (and
  linkable by the next module) when the call returns. `unload` is `pub(crate)` behind
  `Engine::unload_module`. Module names must be identifiers; `engine` and names that clash with
  another owner's namespace are refused (`ScriptError::NameTaken`). Module ids are never
  reused: an unloaded id stays valid and says "module X unloaded".
- **`ModOptions`.** `allowed` is strict: an empty list allows nothing (a mod that exports needs
  `engine.mods`). A prefix matches a namespace equal to it or continuing with a dot. All
  disallowed namespaces are listed in one error (`ScriptError::Disallowed`); link problems are
  `ScriptError::Link { errors }` with every `Imports::check_module` entry.
- **Budgets.** `remaining` is per module and per frame (reset before `init` and before the
  frame's calls); `init`, `frame`, every entry and every cross-module call to a mod's export
  spend from it. A cross-module call that runs out makes the caller trap with a host message.
  An entry or `frame` that runs out skips the module's remaining scripts this frame (one warning
  per frame); a trap in one entry does not skip the others.
- **Mod exports.** Registered after `init` returns; params named from the names section
  registers `r0..` when they are identifiers and unique, else `p0..`; results `ret` (one) or
  `ret0..`. A second `register` with the same name uses `replace` (hot reload). Hot reload
  unregisters the owner's entries first unless the owner also has types in the registry, so a
  function the new version stopped exporting can stay in the index until unload (its call traps
  cleanly with "no such export").
- **Entry points.** `entry(i64, f32, i64) -> i64`, `frame(f32)`, `init()` are checked by type; a
  missing entry or a type mismatch is one warning (per module and name), then skipped.
  `Script` is a Rust-only component.
- **Host functions.** `engine.world.query` returns the number written (at most `cap`), -1 for an
  unknown component. `component_id` returns the id as `i32` (`u32` bit pattern), -1 unknown.
  `get`/`set`/`insert` take the size from the component's `StructDesc`; the guest pointer is
  bounds-checked (out of bounds is a trap, not a status), and a stale entity or unknown id is
  answered before guest memory is touched. `remove` and `despawn` are deferred and return 0 for
  a live entity (a missing component is ignored when applied). `engine.input` key names are the
  `KeyCode` variant names ignoring case (`Space`, `W`, `Key0` or `0`, `LShift`, ...); an unknown
  name is never down. Mouse buttons: 0 left, 1 middle, 2 right, 3 back, 4 forward, 5..255 other.
  Script events are `ScriptEvent { name, data }` in the ordinary `Events<T>` double buffer; at
  most 4096 sent and 1000 log lines kept per frame.
- **Logging.** Script log lines and diagnostics are collected in the host, sent through
  `Engine.logger` after each stage, and kept (last 256) for `Scripting::log_lines()`.
- **Index export.** `EFE_API_EXPORT` is read in `Engine::with_config` (forces headless);
  `game_loop()` writes the file and returns. `set_api_export_path` does the same without the
  environment, `write_api_index(path)` is the writer. The game name is the config title unless
  `set_game_name` is used. Namespace owner `engine` is reserved for the engine
  (`Engine::define_namespace` refuses it).
- LSP (E4.4): `src/lsp/` (`Server::handle(Message) -> Vec<Message>`, `run_stdio`/`run_stdio_code`);
  `lsp-server` 0.7, `lsp-types` 0.97, `serde_json`. Synchronous loop, no threads. Full and
  incremental sync both accepted (full is advertised). Definition and local completion use the
  last AST that parsed, so they can be stale while the text has syntax errors. A word is
  `[alnum_.]+`, so `std.println`, `msg.len` and `Pt.size` are single words. The index is
  stamped by (mtime, length) and re-checked before every message; a failed reload keeps the old
  index and logs an error. A dynamic `**/*.efapi` watcher is registered when the client supports it.

### Changes recorded during implementation: efevm (sections 2 and 6)

- `unregister_owner` returns `Result<usize, RegistryError>` (it can refuse with `StructInUse`).
- New `LinkError::ApiHash`, used only by `Imports::check_module` as context after real errors.
- The `.efapi` reader accepts the raw-text or the canonical hash, rejects duplicate keys and
  `version` != 1 (details in `crates/efevm/API_INDEX.md`).
- `Args` accessors are type-exact (`i64()` on an `entity` parameter is an error); `ptr()` also
  accepts `&Struct`. `replace` may change a signature; existing `Imports` snapshots keep the old one.
- The engine keeps two stores behind one registration path: the `HostRegistry` (the index) and
  the `TypeRegistry` (byte access for scripts). `Engine::register_component`/`register_struct`
  are the only public way into either, and a failure in the second step rolls the first back;
  `crates/core/tests/api_index.rs` checks both directions.
