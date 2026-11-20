> **Mirror.** This repository is generated from
> [efeengine](https://github.com/efex50/efeengine) (`crates/efescript`, `efevm`, `efevm-jit`,
> `efa`, `efemod`, `calc`, `efe-guest`, `efe-wasm`) by `scripts/sync_efescript.py`. Send changes there; edits here are
> overwritten by the next sync.

# efescript

The scripting layer of efeengine. Scripts are written in `.efa`, a text assembly language. The
assembler turns them into `efevm` modules (`.efvm`, verified bytecode with fibers and an
instruction budget), and the `efescript` CLI runs, builds, disassembles, bundles and debugs
them. It also serves a language server (LSP) for editors.

The engine runs the same `.efvm` modules through `crates/core` (`engine::scripting`); this crate
is the toolchain and the standalone `std` host. The old `.efe` format was removed (ROADMAP E3.6):
`.efe` and `.efec` files are rejected with exit 2.

See `structure.md` for the code layout, `docs/e2-design.md` (assembler and runner),
`docs/e3-design.md` (host registry, API index, tooling), `docs/efa-syntax.md` and
`docs/efa-instructions.md` for the language, and `ROADMAP.md` for what comes next.

## CLI

```text
efescript run   <file.efa|file.efvm> [--budget N] [--api FILE.efapi] [--fs-root DIR]
efescript build <file.efa> -o <out.efvm> [--api FILE.efapi]
efescript dis   <file.efvm|file.efa> [-o out.efa]
efescript bundle <file.efa|file.efvm> -o <out> [--runner PATH]
efescript api   <file.efapi>
efescript api-export [-o FILE.efapi]
efescript api-export <file.efmod|file.efvm> [--id ID] [-o FILE.efapi]
efescript pack  <DIR> [--api F]... [-o out.efmod]
efescript inspect <file.efmod|file.efvm>
efescript lsp   [--api FILE.efapi]          (language server on stdin/stdout)
efescript debug <file.efa|file.efvm> [--api FILE.efapi] [--fs-root DIR]
efescript <file.efa> [--budget N]           (shorthand for run)
```

- `run` assembles if needed and runs the exported `main`. `--budget N` limits the number of
  instructions (unlimited by default). `--fs-root DIR` lets `std.fs` read and write below `DIR`.
- `build` writes the encoded module to `-o`.
- `dis` prints a module as `.efa` (names come from the `names` section when present; otherwise
  `f0`, `r0`, `data0`, ...).
- `bundle` writes a standalone executable (see below).
- `api` prints the namespaces, types and functions of an index file. `api-export` writes the
  index of the CLI's std host (default: stdout).
- `pack DIR` builds the mod in `DIR` (`mod.toml`, `src/main.efa`, `assets/`) into a `.efmod`
  package (default name `<id>-<version>.efmod` in the current directory). `inspect` prints a
  package or a bare module; `api-export FILE` writes the index of a `.efmod`, or of a bare
  `.efvm` with `--id`. Every `--api` value may be a `.efapi` file or a `.efmod`.
- `lsp` and `debug` are described below.

Exit codes:

- `main` returns nothing: 0. `main` returns one `i32`: that value. `std.exit(code)`: that code.
  The OS takes exit codes modulo 256.
- 2: a load or assembly error (missing file, diagnostics, no exported `main` or a wrong `main`
  signature, an unresolved import, an unreadable or invalid `--api` file, a `.efe`/`.efec` file).
- 3: a trap (the trap and its backtrace go to stderr), or the `--budget` ran out.

## Mods in Rust and C (E7.4a)

`efescript api-export --rust GAME.efapi [DEP.efmod]... -o api.rs` and `--c ... -o efe_api.h`
generate bindings for a guest mod compiled to wasm (Rust: crate `efe-guest`; C/C++:
`crates/efe-guest/include/efe_guest.h`). The wasm ABI, the custom sections and the plan for the
translator are in `docs/e7.4-wasm-design.md`; the translator itself (E7.4b) is not built yet.

## API index (`--api`)

An API index (`.efapi`) lists everything a script can call or see: namespaces, host functions
with parameter names and docs, and the layouts of structs and components. A game writes one
with `cargo xtask api-export <example>`; the format is in `crates/efevm/API_INDEX.md`.

- `build FILE --api GAME.efapi` (and `run`, `debug`, `lsp`) checks each `import ns.name(...)`
  against the index by name and signature, with "did you mean" suggestions.
- With an index, `import ns.name` may leave out the signature.
- Every struct in the index provides constants: `S.size`, `S.align`, `S.<field>` (byte offset),
  and `S.id` for components.
- The module records the index hash and the version of each namespace it imports (section 12
  of `.efvm`), and the engine checks those at link time.

`run` links against the std host only, so it can run an `--api` module only when the module
imports nothing but `std`. Modules that import game functions run inside the engine.

## Host functions

| Namespace | Import | Meaning |
|---|---|---|
| `std` | `std.print(i64, i64)` | write `len` bytes at `ptr` |
| `std` | `std.println(i64, i64)` | the same, then a newline |
| `std` | `std.print_i64(i64)` · `std.print_f64(f64)` · `std.print_char(i32)` | write a number or one character |
| `std` | `std.read_line(i64, i64) -> i64` | read a line into `ptr` (at most `cap` bytes); returns the length |
| `std` | `std.play_sound(i64, i64) -> i32` | play the file at the path; 0 = ok (real playback needs the `audio` feature) |
| `std` | `std.exit(i32)` | stop the program with this exit code |
| `std.time` | `std.time.now_ms() -> i64` | monotonic milliseconds since the program started |
| `std.time` | `std.time.unix_ms() -> i64` | Unix time in milliseconds |
| `std.fs` | `std.fs.read_file(i64, i64, i64, i64) -> i64` | read `path` into `buf`; returns the bytes copied (at most the buffer size); -1 on error, -2 when denied |
| `std.fs` | `std.fs.write_file(i64, i64, i64, i64) -> i32` | write `data` to `path`; 0 ok, -1 I/O error, -2 denied or escaping the root |

`std.fs` is denied unless `--fs-root DIR` is given. Paths are relative and may not contain `..`.

## Language reference (short)

The full reference is `docs/efa-syntax.md`. The mnemonics are the `efevm` ones, listed in
`crates/efevm/OPCODES.md`.

- **Lexical rules.** `;` starts a comment. A dotted name is one token (`i64.add`, `std.println`,
  `msg.len`). Integers: decimal, `0x`, `0b`, `0o`, `_` separators, optional `-`. Floats: `1.5`,
  `1e9`, `inf`, `nan`, `nan:0x7fc00001`. Characters: `'a'`, `'\n'`. Strings: `"..."` with the
  escapes `\n \t \r \0 \\ \" \xNN`. A statement ends at the end of its line; blocks use `{` `}`.
- **Top-level items.** `import std.println(i64, i64) -> i64`, `memory MIN [MAX]` (64 KiB pages),
  `const NAME = 10`, `global [mut] NAME: i64 = 0`, `table [f1, null, f2]`, data
  (`NAME db "text", 10`, `dw`, `dd`, `dq`, `NAME resb 64`), and `[export] fn NAME(a: i64) -> i64 { ... }`.
  Types are `i32 i64 f32 f64`.
- **Functions, registers, labels.** Parameters are registers `r0`, `r1`, ... named by the
  parameter list. `reg a, b` declares more named registers. Raw `rN` registers can always be used.
  Labels are `name:` and are local to the function. Every function must end in `ret`, `jmp`,
  `br_table` or `unreachable`; the assembler does not add one.
- **Instructions.** The `efevm` mnemonic with the destination first: `i64.add d, a, b`,
  `i64.load d, base, off`, `i64.store src, base, off`.
- **Conveniences** (each expands to plain instructions):
  - immediates in source positions: `i64.sub n, n, 1`; `const` and data names work too;
  - call sugar: `call f(a, b) -> x` or `-> (x, y)`, where `f` is a function or an import;
    `call_indirect (i64) -> i64, idx(a) -> x` for indirect calls;
  - returns: `ret`, `ret a`, `ret (a, b)`;
  - loads and stores with an optional offset; the base may be a data name;
  - `br_table idx, [a, b, c], default`;
  - globals by name: `global.get d, NAME`, `global.set NAME, a`.
- **Data.** Every data item defines `NAME` (its address) and `NAME.len` (its size in bytes).
  Items are laid out from address 1024 in definition order, each 8-byte aligned.

Hello world:

```text
; hello.efa
import std.println(i64, i64)          ; pointer, length

msg db "hello world!"

export fn main() {
    call std.println(msg, msg.len)
    ret
}
```

Sum of n, n-1, ..., 1 (a loop; `examples/recursion.efa` has the recursive version):

```text
export fn sum(n: i64) -> i64 {
    reg acc, t
    const.i64 acc, 0
loop:
    i64.eqz t, n
    br_if t, done
    i64.add acc, acc, n
    i64.sub n, n, 1
    jmp loop
done:
    ret acc
}
```

## Examples

Run from the crate directory (`crates/efescript`), so relative paths resolve, e.g.
`cargo run -q -p efescript -- run examples/hello.efa`. Each file has its expected output in its header comment.

- `examples/hello.efa`: one import, one string, one call (`hello world!`).
- `examples/strings.efa`: `db` strings and escapes, `NAME.len`, byte loads, a slice, `dw`/`dd`/`dq` tables.
- `examples/jumps.efa`: every comparison and branch form on i64 and i32, `eqz`, `br_if`, `br_ifz`, `jmp`, `br_table`, `select`.
- `examples/recursion.efa`: recursive sum, factorial and fibonacci with call sugar.
- `examples/floats.efa`: f64 and f32 arithmetic, comparisons, conversions, reinterpret, a float global.
- `examples/sound.efa`: `std.play_sound` on `examples/ominous_worse.ogg`; `main` returns the status (0 or 1).
- `examples/exit.efa`: `call`/`ret` between two functions, then `std.exit(5)`.
- `examples/echo.efa`: reads one line from stdin with `std.read_line` and prints it back.
- `examples/write_file.efa`: writes `haha` below `--fs-root`; without a root it prints `denied` and exits 1.

## Standalone executables

```text
efescript bundle examples/hello.efa -o hello      # a copy of the runner with the module appended
./hello
```

- The runner is `--runner PATH`, else `efescript-runner` next to the `efescript` executable,
  else the executable itself.
- The embedded module runs with `std.fs` denied and the command line is ignored.
- A bundle for another platform needs that platform's runner, passed with `--runner`.
- The embedded module is not protected: anyone can cut it out and disassemble it with `efescript dis`.
- On macOS the binary must be re-signed after bundling (`codesign --force --sign - OUT`).

## Language server and debugger

`efescript lsp [--api FILE]` speaks LSP over stdio. Without `--api` it uses the single `*.efapi`
in the workspace root. Features: diagnostics (assembler and `--api` checks), go to definition for
labels, functions, data and constants, completion of host functions (`ns.name` with signature and
doc), mnemonics and local names, hover for host functions, struct constants and mnemonics.
The `.efapi` file is reloaded when it changes.

`efescript debug FILE [--api FILE] [--fs-root DIR]` is a line-oriented debugger on stdin and
stdout. Commands: `break LINE|FUNC`, `delete N`, `step`, `next`, `continue`, `regs`, `mem ADDR LEN`,
`bt`, `list`, `quit`; `help` lists them. A call to an import is shown by name with its parameters
(`call std.println(text = r0..r1)`).

## Try it

From the repository root. These run the `script_demo` game's API index and scripts:

```
cargo xtask api-export script_demo                     # writes target/efapi/script_demo.efapi
cargo run -p efescript -- api target/efapi/script_demo.efapi
cargo run -p efescript -- build examples/script_demo/scripts/mover.efa --api target/efapi/script_demo.efapi -o mover.efvm
cargo run -p efescript -- lsp --api target/efapi/script_demo.efapi   # for an editor's LSP client
printf 'step\nstep\nregs\ncontinue\n' | cargo run -q -p efescript -- debug crates/efescript/examples/hello.efa
cargo test -p script_demo                              # the end-to-end test
```

## Tests

```text
cargo test -p efescript
```

Library unit tests and the integration tests in `tests/` (examples against their expected output,
CLI and exit codes, diagnostics, bundles, debugger, LSP, index, round trips, and malformed input).
The engine end-to-end test is `cargo test -p script_demo`.
