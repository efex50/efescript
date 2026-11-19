# efa

efa is the `.efa` compiler that the engine runs at load time: the assembler (`asm`), the linker (`link`) and the mod-folder builder (`mod_folder`). It was moved out of efescript in E10 so that games can compile `.efa` sources without the efescript toolchain. The design is in `crates/efescript/docs/e10-design.md`.

## At a glance
| | |
|---|---|
| Path | `crates/efa` |
| Kind | library (`efa`); `fuzz/` is a separate cargo-fuzz crate, excluded from the workspace |
| Depends on (workspace) | `efevm`, `efemod` (reader only, never the `write` feature), `calc` (no default features: constant expressions) |
| Key external deps | `toml` 1 (reads `mod.toml`) |
| Used by | `efeengine-core` (optional feature `efa`, re-exported as `efeengine_core::efa`) and `efescript` (re-exports `asm` and `link`) |
| Targets | wasm32-unknown-unknown; the file-system helpers (`FsSource`, `list_assets`) are native only |
| Features | none |

## Layout
```text
crates/efa
├── Cargo.toml           package efa 0.1.0, edition 2021
├── src/
│   ├── lib.rs           `pub mod asm, link, mod_folder`; `VERSION` (the engine's disk cache key includes it)
│   ├── asm/             the assembler (unchanged from efescript except the limits and `read_bytes`)
│   │   ├── lexer.rs, parser.rs, ast.rs   tokens, syntax tree (`use`, `pub`, `provide`, `struct`, `scratch`, `import event` / `provide event`)
│   │   ├── diag.rs      diagnostics: file, line, column and a caret under the span
│   │   ├── source.rs    `FileSource` (`read`, `read_bytes`, `display_name`, `list`: the engine hashes a mod folder from it), `MemSource`, `FsSource` (native: `new`, `for_file`, `with_prefix`; refuses escapes and symlinks out of the root)
│   │   ├── lower/       one file as `asm::Unit`: mod.rs (declare, resolve, define), func.rs, value.rs, consts.rs (constant expressions, `Fold`: lazy evaluation of consts, struct layouts and data sizes)
│   │   └── mod.rs       `parse`, `assemble_with(src, file, &AsmOptions)`, `assemble_program(root, &dyn FileSource, opts)`, `assemble_programs`; `AsmOptions { api, max_use_depth, max_nesting }`
│   ├── link/            the linker: mod.rs (`Unit`, `Decls`, `link`), layout.rs (phase 1: indices, data, scratch, memory, program-wide checks), tests.rs
│   └── mod_folder.rs    mod folders: `read_mod_toml`, `inside`, `BuildLimits`, `prepare_mod`, `finish_mod`, `build_mod`, `LimitedSource`, `asm_options`, `mod_index`, `list_assets` (native)
├── tests/
│   ├── const_expr.rs    constant expressions: positions, precedence, names, typing, errors and spans, other files
│   └── never_panics.rs  mutated seed folders, long `use` chains and deep nesting: errors, never panics
└── fuzz/
    └── fuzz_targets/build_mod.rs   cargo-fuzz target for `build_mod`
```

## Main concepts
- **One input abstraction.** The assembler reads everything through `FileSource`. The engine passes `FsSource` (native files), `MemSource` (tests and in-memory mods), and wrappers: `LimitedSource` (limits on files and bytes) and the recording wrapper in core's disk cache.
- **Limits before recursion.** `AsmOptions::max_use_depth` and `max_nesting` turn over-limit input into a diagnostic. The nesting check runs before any recursive parsing. `BuildLimits` (defaults: 32 MiB of files read, 1024 files, use depth 64, nesting 128) apply to mod-folder builds.
- **Folder builds are split.** `prepare_mod` reads `mod.toml`, validates the manifest and the `[build]` paths; `[build] api` entries are checked with `inside()` and then ignored. The engine compiles the entry between `prepare_mod` and `finish_mod`, so the compile can go through its cache. `finish_mod` runs `efemod::check_module` and reads the assets as bytes. It refuses invalid asset names and the reserved names `manifest.json`, `module.efvm`, `engine` and `engine/*`. `build_mod` runs all three steps.
- **Constant expressions** (efa-syntax 18). `parser::const_operand` converts the efa tokens of an expression (in the positions `memory`, `const`, `global`, data values, struct counts, `scratch`, load/store offsets, `const.T` values) to `calc` tokens whose byte offsets are synthetic (`k..k+1`) and runs `calc::Parser::parse_expr`, which stops at the first token that cannot continue it. The AST keeps `Operand::Expr(Box<ConstExpr>)` = the `calc::Expr` + `spans[k]` (the efa span of token `k`); a lone literal or name stays `Int`/`Float`/`Char`/`Name`, so its output is byte-identical to before. A negative literal after an operand is split into `-` and the number. Evaluation: `lower::consts::Fold` evaluates consts, struct layouts and data sizes on demand in `declare` (cycles reported at the closing use), the names a const reads come from `env.values` (API consts, struct consts, `alias.NAME` of used files), and a const that needs a data address or event id is "late" (computed in `resolve`, never `pub`). Other files' `pub` consts are known at declare time because `link::declare_all` declares a unit's `Unit::deps()` first and passes their `Decls` (`link::Declared`, `link::static_values`). The folded `calc::Value` becomes a `Val` (floats as `f64` text) and is narrowed by `type_val` at the use site.
- **Diagnostics name the file.** `FsSource::with_prefix` sets the display name of a mod folder, so errors name the folder.

## Tests
`cargo test -p efa` runs the unit tests in `lexer.rs`, `parser.rs`, `diag.rs`, `link/tests.rs` and `mod_folder.rs`, plus `tests/const_expr.rs` and `tests/never_panics.rs`. The assembler's end-to-end tests are in `crates/efescript/tests`. `cargo fuzz run build_mod` from `crates/efa/fuzz` needs cargo-fuzz. The engine's load tests are in `crates/core/tests/load.rs`.
