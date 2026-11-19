# efescript

efescript is the scripting layer of efeengine. A text assembler (`.efa`) lowers source to an `efevm` module (`.efvm`); the CLI assembles, runs, disassembles, bundles, exports and checks the API index, and serves an LSP server and a line debugger. The engine does not depend on this crate: `efeengine-core` (`engine::scripting`) runs `.efvm` modules on `efevm` itself. The old `.efe` path (x86-flavoured assembly and its VM) was removed in roadmap E3.6; `.efe`/`.efec` files are rejected with exit 2. Design notes: `docs/e2-design.md` (assembler, runner) and `docs/e3-design.md` (host registry, API index, tooling, engine integration).

## At a glance
| | |
|---|---|
| Path | `crates/efescript` |
| Kind | library (`efescript`) plus two binaries: `efescript` (`src/main.rs`, the CLI) and `efescript-runner` (`src/bin/runner.rs`, runs an embedded module) |
| Depends on (workspace) | `efevm`, `efa` (assembler and linker, E10), `efemod` (with `write`, for `pack`) |
| Key external deps | `lsp-server` 0.7 and `lsp-types` 0.97 (LSP), `serde_json` (index and LSP messages), `rodio` (optional, `audio` feature: real sound for `std.play_sound`) |
| Used by | Nothing in the workspace. `examples/script_demo` no longer dev-depends on it (E10) |
| Targets | native. The `std` host, the CLI and the LSP are native only; `efevm` itself has no I/O and builds for `wasm32-unknown-unknown` |
| Features | `audio` (needs ALSA headers on Linux) |

## Layout
```text
crates/efescript
├── Cargo.toml           package efescript; bins efescript and efescript-runner
├── README.md            user guide: CLI, exit codes, host functions, language, tools
├── ROADMAP.md           E-items (E0-E5) and what is left
├── docs/
│   ├── efa-syntax.md        full `.efa` syntax reference
│   ├── efa-instructions.md  guide to every instruction (efevm mnemonics)
│   ├── e0-design.md         original design (the removed `.efe` path; history)
│   ├── e2-design.md         `.efa` assembler and v2 runner
│   ├── e8-design.md         modules (multi-file `.efa`, linker) and mod packages
│   ├── e10-design.md        engine loading API, script cache, mod sources (E10; the assembler is in `crates/efa`)
│   └── e3-design.md         host registry, std host, API index, LSP, debugger, engine integration
├── examples/            `.efa` programs with their expected output in the header comment:
│   │                    hello, strings, jumps, recursion, floats, sound (+ ominous_worse.ogg),
│   │                    exit (std.exit code), echo (std.read_line), write_file (std.fs, needs --fs-root)
├── src/
│   ├── main.rs          CLI: run, build (follows `use`; several roots), dis, bundle, api, api-export (std host, or a `.efmod`/`.efvm`), pack, inspect, lsp, debug (USAGE text is the reference)
│   ├── lib.rs           module list (below)
│   ├── prelude.rs       common re-exports (StdHost, ...)
│   ├── host.rs          `Host` trait (output, sound), `StdHost` (process stdio), `CaptureHost` (tests)
│   ├── host_std.rs      `std_registry(&StdOptions)`: HostRegistry of `std`, `std.time`, `std.fs`; `StdEnv<H>`
│   ├── run_v2.rs        load (path, bytes, source, with or without `AsmOptions`), `run`/`run_with`, `run_embedded`, trap reports
│   ├── disasm.rs        `.efvm` (or `.efa`) to `.efa` text; names from the `names` section
│   ├── pack.rs          mod packaging: `pack_dir(dir, &[&ApiIndex]) -> Packed` (`.efmod`; reads `[build] api`, then `efa::mod_folder::build_mod`), `read_api_input` (`.efapi` or `.efmod`); re-exports `read_mod_toml`, `mod_index`, `BuildToml`, `ModToml` from `efa::mod_folder`; behind `pack`, `inspect`, `api-export FILE`
│   ├── bundle.rs        standalone executables: runner plus appended module and trailer
│   ├── debugger/mod.rs  `Debugger`: the `efescript debug` command set on stdin/stdout
│   ├── lsp/             `Server::handle(Message) -> Vec<Message>`, `run_stdio`
│   │   ├── mod.rs       message loop, capabilities, requests (definition, completion, hover)
│   │   ├── analysis.rs  diagnostics and symbol tables from the last AST that parsed
│   │   ├── project.rs   multi-file programs: buffer-first `BufSource` (open buffers win over disk), program root from the nearest `mod.toml` `[build] entry`, per-file diagnostics
│   │   └── text.rs      text and position helpers
│   ├── bin/runner.rs    `efescript-runner`: runs the module embedded in its own executable
│   └── compiler/        high-level language front end: lexer and empty AST stubs, not wired to anything
├── proc_macros/         sibling crate in the workspace, not a dependency (build-time constants)
├── strtohx/             dev tool that emitted push instructions for the removed `.efe` format
└── tests/
    ├── api_index.rs         std registry: every name has an index entry with a doc; JSON round trip; disasm/asm with `--api`
    ├── pack.rs              mod folders to `.efmod`: determinism, unknown keys, undeclared imports, merged apis, `.efmod` as `--api`, CLI pack/inspect/api-export
    ├── v2_backtrace.rs      trap backtraces with source lines
    ├── v2_bundle.rs         bundle and embedded run
    ├── v2_cli.rs            CLI options, exit codes, `.efe` rejection
    ├── v2_debugger.rs       debugger commands on scripted stdin
    ├── v2_diagnostics.rs    assembler diagnostics
    ├── v2_events.rs         `import event` and `provide event` (efa-syntax section 17): checks, section 14 and section 13 output
    ├── v2_examples.rs       every example against its expected output
    ├── v2_lsp.rs            LSP messages (diagnostics, definition, completion, hover, index reload)
    ├── v2_multifile.rs      multi-file programs: `use`, visibility, imports, layout, names/lines, provides, structs, scratch, reserved words, FsSource
    ├── v2_multifile_tools.rs  language server (definition, hover, completion, event hover), debugger and `run` on multi-file programs
    ├── v2_never_panics.rs   malformed input gives errors, not panics
    ├── v2_ported.rs         the ported legacy examples (exit, echo, write_file)
    └── v2_roundtrip.rs      `build`, then `dis`, then `build` gives the same bytes
```

`lib.rs` exports `asm`, `bundle`, `compiler`, `debugger`, `disasm`, `host`, `host_std`, `link`, `lsp`, `pack`, `prelude`, `run_v2`. `asm` and `link` are `pub use efa::{asm, link}`: the code lives in `crates/efa` (`crates/efa/structure.md`).

## Main concepts
- **Pipeline.** `.efa` text goes through `efa::asm` (lexer and parser per file of the use graph, one `asm::Unit` per file) and `efa::link` (two-phase linking into one `efevm::ModuleBuilder`) to an `efevm::Module`. `run_v2` links it against `host_std`'s registry (`Imports::check_module` reports every link problem at once, by name), creates an `Instance` and runs `main` under an instruction budget. `dis` prints a module back as `.efa`.
- **Host functions.** `efevm::HostRegistry` is the only way to make a host function callable. efescript's registry (`host_std::std_registry`, owner `std`) has:

  | Namespace | Functions |
  |---|---|
  | `std` (v1) | `print(text)`, `println(text)`, `print_i64(v)`, `print_f64(v)`, `print_char(c)`, `read_line(buf) -> (len)`, `play_sound(path) -> (status)`, `exit(code)` |
  | `std.time` (v1) | `now_ms() -> (ms)` (monotonic, since start), `unix_ms() -> (ms)` |
  | `std.fs` (v1) | `read_file(path, buf) -> (len)`, `write_file(path, data) -> (status)` |

  `std.fs` is always registered but denied unless a root is given (`--fs-root`, `StdOptions::fs_root`); denied calls return -2. Paths are relative, may not contain `..`, and are resolved under the root after canonicalisation. The engine never links `std`.
- **API index.** `efescript api FILE.efapi` prints an index; `api-export [-o FILE]` writes the std index. `build` and `run` accept `--api FILE.efapi` (`AsmOptions::api`): imports are checked by name and signature, `import ns.name` may omit the signature, `S.size`, `S.align`, `S.<field>` (and `S.id` for components) are constants, and section 12 of the `.efvm` records the index hash and namespace versions. `run` links against the std host only, so it runs such a module only when it imports nothing else. The index format is in `crates/efevm/API_INDEX.md`.
- **Mod packages.** `pack DIR` reads `DIR/mod.toml` (unknown keys are errors), assembles `[build] entry` against `--api` plus `[build] api`, runs `efemod::check_module`, adds the `assets/` files and writes a deterministic `.efmod` (default `<id>-<version>.efmod` in the current directory). `inspect` prints a package or bare module; `api-export FILE [--id ID]` writes a mod's index. Any `--api` value may be a `.efmod` (its index is the mod's provides, structs and events). Specified in `crates/efevm/MOD_PACKAGE.md` and `docs/e8-design.md` sections 8 and 10.
- **Standalone executables.** `bundle` copies the runner and appends the compiled module with a trailer. An executable with an embedded module runs it (with `std.fs` denied) and ignores its arguments.
- **LSP.** `lsp-server` over stdio, synchronous, no threads. Diagnostics on open and change (with `--api` checks), go to definition and completion for labels, functions, data and constants, hover for host functions and struct constants. The `.efapi` file is reloaded when its mtime or length changes and on `didChangeWatchedFiles`; a failed reload keeps the old index.
- **Debugger.** `debug FILE [--api] [--fs-root]` reads commands from stdin (`break`, `delete`, `step`, `next`, `continue`, `regs`, `mem`, `bt`, `list`, `quit`). It uses `efevm`'s fiber hooks (`fiber_location`, `fiber_registers`, `fiber_backtrace`, `resume` with budget 1).
- **Exit codes.** `run`: `main`'s return value or the `std.exit` code. 2: load, assembly or API index error, `.efe` input. 3: trap or exhausted `--budget`.

## Tests
`cargo test -p efescript` (the integration tests above). The assembler and linker unit tests are in `crates/efa` (`cargo test -p efa`). The end-to-end test with the engine is `examples/script_demo/tests/e2e.rs` (`cargo test -p script_demo`).

## Known gaps
- `std.fs.write_file` checks before it writes, so a concurrent symlink swap is not defended against (time-of-check/time-of-use).
- `std.print_char` with an invalid code point traps; `std.play_sound` with a non-UTF-8 path returns 1.
- `dis` then `asm` without `--api` drops section 12 (a comment cannot carry it); with the same index the round trip is byte-identical.
- The LSP's definition and completion use the last AST that parsed, so they can be stale while the text has syntax errors.
- The `compiler/` front end is a stub. There is no high-level language yet (ROADMAP).
- Bundles are not protected: the module bytes can be cut out and disassembled. On macOS a bundle must be re-signed.
