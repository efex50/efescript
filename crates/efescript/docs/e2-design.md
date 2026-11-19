# efescript E2 design: assembler v2 on efevm

Scope: roadmap E2 (`crates/efescript/ROADMAP.md`): a text assembly language for the
`efevm` VM (`crates/efevm/SPEC.md`), real diagnostics, debug info, a disassembler, and a
CLI that runs it. Plus three small `efevm` changes (section 9).

The old assembler (`nasm_efe`) and old VM (`runtime`) stay in place, untouched, as the
legacy path until E3.6 removes them.

| | Legacy | v2 |
|---|---|---|
| Source | `.efe` | `.efa` |
| Binary | `.efec` | `.efvm` |
| Code | `src/nasm_efe`, `src/runtime` | `src/asm`, `src/disasm`, `src/host_std`, crate `efevm` |

## 1. A first program

```
; hello.efa
import std.println(i64, i64)          ; pointer, length

msg db "hello world!"

export fn main() {
    call std.println(msg, msg.len)
    ret
}
```

## 2. Lexical rules

- `;` starts a comment that runs to the end of the line.
- Names: letters, digits, `_`, not starting with a digit. Mnemonics and qualified names use dots (`i64.add`, `std.println`, `msg.len`).
- Integers: decimal, `0x`, `0b`, `0o`, optional `-`, optional `_` separators. Floats: `1.5`, `-0.25`, `1e9`, `inf`, `nan`.
- Characters: `'a'`, `'\n'` (an integer). Strings: `"…"` with escapes `\n \t \r \0 \\ \" \xNN`.
- Statements end at the end of the line. Blocks use `{` `}`.

## 3. Top-level items

```
import NAMESPACE.NAME(type, …) -> type        ; result part optional; several results: -> (t, t)
memory MIN [MAX]                              ; pages of 64 KiB; default: enough for the data, at least 1
const NAME = 123                              ; assembly-time integer or float constant
global NAME: i64 = 0                          ; immutable
global mut NAME: f64 = 1.5                    ; mutable
table [f1, f2, f3]                            ; function table for call_indirect (index 0, 1, 2)
NAME db "text", 10, 0xff                      ; bytes and strings
NAME dw 1, 2      NAME dd 1, 2      NAME dq 1, 2     ; 16/32/64-bit little-endian values
NAME resb 64                                  ; 64 zero bytes
fn NAME(params) -> result { … }               ; `export fn` also exports it under NAME
```

### Data (the string system)
- Every data definition creates `NAME` (its address) and `NAME.len` (its size in bytes).
- Data is laid out in definition order starting at address **1024** (address 0 stays
  unused so 0 can mean "null"), each item aligned to 8 bytes. `resb` regions are zero.
- `NAME` and `NAME.len` are integer constants: usable wherever an immediate is allowed.

## 4. Functions and registers

```
export fn sum(n: i64) -> i64 {
    reg acc, t                    ; named registers (after the parameters)
    const.i64 acc, 0
loop:
    i64.eqz t, n
    br_if t, done
    i64.add acc, acc, n
    i64.sub n, n, 1               ; immediate in a source position (section 5)
    jmp loop
done:
    ret acc
}
```

- Parameters are named registers `r0 …` in order. `reg a, b` declares more named registers.
- Raw registers `r0`, `r1`, … may also be used directly. `nregs` is computed by the assembler.
- Labels are `name:` and are local to the function.
- Types on parameters and results: `i32`, `i64`, `f32`, `f64`.
- Every function must end in a terminator (`ret`, `jmp`, `br_table`, `unreachable`); the
  assembler reports a clear error otherwise (it does not insert one).

## 5. Instructions

Mnemonics and operand order are exactly those of `efevm` (`efevm::Op::name()`, see
`crates/efevm/OPCODES.md`): destination first, then sources.

Conveniences the assembler adds (all expand to plain instructions):

- **Immediates as sources.** Where an instruction expects a source register, an integer,
  float, character, `const` name or data symbol may be written. The assembler loads it into
  a hidden temporary register first, with the constant type the instruction expects.
- **Call syntax.** `call f(a, b) -> x` (also `-> (x, y)`, or no `->`) copies the arguments
  into a fresh register window, calls, and copies results out. `f` may be a function or an
  import. `ret a` / `ret (a, b)` / `ret` likewise copy into a window when needed.
  The raw forms `call f, rBASE` and `ret rBASE` remain available.
- **Indirect calls.** `call_indirect (i64, i64) -> i64, idx(a, b) -> x`; raw form
  `call_indirect (i64, i64) -> i64, idx, rBASE`.
- **Loads and stores.** `i64.load d, base, off` as in the spec; `off` may be omitted (0)
  and may be a constant or data symbol; `base` may be a data symbol (materialised).
- **`br_table`.** `br_table idx, [a, b, c], default`.
- **Globals** by name: `global.get d, counter`, `global.set counter, a`.

Hidden temporaries are allocated above the named registers and reused between statements.

## 6. Diagnostics (E2.3)

- One error type: file, line, column, length, message, optional note; `Display` prints
  ```
  error: unknown register `acx`
    --> hello.efa:7:19
     |
   7 |     i64.add acc, acx, n
     |                  ^^^ did you mean `acc`?
  ```
- The assembler collects as many errors as it can in one run (it recovers at the next line).
- No input panics the assembler. Verification errors from `efevm` are mapped back to a
  source line when possible.

## 7. Debug info (E2.4)

- The `names` section gets function, import and named-register names.
- The `lines` section maps every instruction to file, line and column.
- A trap at run time is reported with a backtrace of function names and source lines.

## 8. CLI and std host

```
efescript run   FILE.efa | FILE.efvm [--budget N]     assemble if needed, run export `main`
efescript build FILE.efa -o OUT.efvm
efescript dis   FILE.efvm [-o OUT.efa]
efescript FILE.efe | FILE.efec …                      legacy path, unchanged behaviour
```
- `main` may return nothing (exit 0) or one `i32` (the exit code).
- A trap prints the trap and backtrace and exits with code 3; an assembly or load error exits with code 2.
- `--budget` limits total instructions (default: unlimited for `run`).

Host functions the CLI and tests provide, namespace `std` (a temporary minimum; the real
registry is E3), built on the existing `Host` trait so `CaptureHost` works in tests:

| Import | Meaning |
|---|---|
| `std.print(i64, i64)` | write `len` bytes at `ptr` |
| `std.println(i64, i64)` | the same, then a newline |
| `std.print_i64(i64)` · `std.print_f64(f64)` · `std.print_char(i32)` | write a number / one character |
| `std.read_line(i64, i64) -> i64` | read a line into `ptr` (at most `cap` bytes); returns the length |
| `std.play_sound(i64, i64) -> i32` | play the file at the path; 0 = ok |
| `std.exit(i32)` | stop the program with this exit code |

Bad pointers or invalid UTF-8 paths trap or return a failure; they never panic.

## 9. Disassembler (E2.5)

- `dis` prints valid `.efa` text. With a `names` section it uses the names; without one it
  uses `f0`, `r0`, `data0`, ….
- It prints the raw forms (no call sugar), so **assemble → disassemble → assemble yields
  identical bytes**. That round trip is a test for every example.

## 10. efevm changes that come with E2

1. **Bulk memory cost.** `memory.copy`, `memory.fill` and `memory.grow` cost
   `1 + bytes / 256` budget units. If that exceeds the remaining budget the operation still
   completes, and the excess becomes a debt the fiber pays off at its next resumes before
   executing anything.
2. **Lower default limits.** `max_memory_pages` 1024 (64 MiB), `max_call_depth` 4096,
   `max_registers` 262144 per fiber, `max_fibers` 256. A fiber's register stack is released
   when the fiber finishes or traps.
3. **`Instance::call(export, args, host, budget) -> Result<Vec<u64>, CallError>`**:
   spawn, run to completion, return results, remove the fiber. `CallError` distinguishes
   trap, out-of-budget and yield.

`SPEC.md` is updated to match (draft 3).

## 11. Examples and tests

`examples/*.efa`: `hello`, `strings`, `jumps` (every comparison and branch form),
`recursion` (sum, factorial, fibonacci), `sound`, `floats`, `fibers` is not possible from
the CLI yet (no spawn host call) and is covered in efevm's own tests instead.

Tests (`tests/v2_*.rs`): each example's exact output and exit code through `CaptureHost`;
diagnostics tests asserting message, line and column; a "never panics" test over odd and
mutated inputs; the round-trip test of section 9; CLI tests for `run`, `build`, `dis`;
a trap backtrace test that shows the right source line.

## Changes (recorded during implementation, 2026-11-15)

The implementation follows this design; the points below are where it was ambiguous,
silent or slightly wrong, and what was chosen instead. The full textual contract is
`docs/efa-syntax.md` (its section 14 lists the smaller syntax decisions).

1. **Round trip (section 9).** "Identical bytes" is checked on `Module::encode()` with the
   `lines` section removed from both sides: disassembled text necessarily has different line
   numbers. The `names` section is kept and must match. A module without a `names` section
   round-trips only modulo names (the assembler always writes function names).
2. **Extra syntax needed for exact round trips:** `nregs N` in a body (the disassembler prints
   it only when the module's count differs from the computed one), `null` entries in `table`,
   `export global`, float literals in `dd`/`dq`, and `nan:0x…` for exact NaN bits.
3. **Lowering shape is fixed** (efa-syntax.md section 7): temporaries and call/ret windows start
   at the first register above all named and raw registers; call sugar always copies (no move
   elision); bare `ret` is `ret r0`; a window base counts as `base + max(P, R)` registers only.
   Exports, names and types are emitted in a canonical order so dis → asm reproduces the bytes.
4. **Diagnostics (section 6).** The design's sample shows column 19 with the caret under column
   18; the implementation reports the operand's own column (18). The lowering runs only when
   lexing and parsing produced no errors, so one syntax error hides semantic errors of the same
   run; at most one parser error is reported per line and at most 100 diagnostics per run.
5. **CLI (section 8).** Also `efescript FILE.efa` as shorthand for `run`; `dis` accepts `.efa`
   too. Out of budget exits 3 like a trap. If `main` yields (guest `yield`) the runner resumes
   it. `std.exit` records the code and yields; the recorded code wins over `main`'s result.
   Exit codes are reduced modulo 256 by the OS. Long backtraces are cut to the 16 innermost and
   4 outermost frames. For a `.efvm`, source lines in a trap report are read only from regular
   files of at most 16 MiB named in the `lines` section.
6. **std host.** `read_line` returns -1 at end of input; a line longer than `cap` is truncated
   and the rest of that line is discarded. `print_f64` uses Rust `{}` formatting. Bad pointers
   trap with `OutOfBounds`; an invalid UTF-8 sound path returns 1. Input for `read_line` comes
   from `StdEnv` (the legacy `Host` trait has no input method and was not changed).
7. **efevm (section 10).** `memory.grow` is charged for the requested size even when it fails.
   Added `Instance::resume_units` (budget units consumed per resume) so embedders can account a
   total budget across yields, because `resume_counted` counts instructions only. A hidden
   `fiber_stack_capacity` observer exists for tests. The introduction's reference to "section 9"
   for the efevm changes means section 10.
