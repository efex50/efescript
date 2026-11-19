# efescript E0 design: strings, symbols, host, tests

Scope: roadmap phase E0 (make the current VM testable) plus a string system.
This applies to the **current** byte format; the v1 redesign (E1+) replaces it later.

## 1. Data definitions (the string system)

NASM-style definition, usable anywhere in a source file:

```
msg  db "hello world!", 10        ; strings and byte values, comma separated
tbl  db 1, 2, 0xff, "ab"
```

- `NAME db item, item, …` where an item is a string literal or a number 0–255.
- String literals: double quotes, UTF-8 bytes, escapes `\n \t \r \0 \\ \" \xNN`. No implicit terminator.
- The definition creates two symbols:
  - `NAME` = address of the first byte (the start pointer);
  - `NAME.len` = length in bytes (replaces NASM's `len equ $ - msg`).
- Usage:
  ```
  mov ra, 2          ; Println
  mov rb, msg        ; start pointer
  mov rc, msg.len    ; length
  syscall
  ```
- A symbol may be used wherever an immediate number is allowed, and as the single item
  of a pointer (`BYTEPTR [msg]`) or the static part of `[reg+sym]` / `[sym+reg]`.
- Code labels are symbols too: `mov ra, _loop` loads a label's address.

### Placement in the output
All data is placed **after the last instruction**, in definition order, preceded by one
`0x00` byte. That zero byte is the end-of-program marker, so execution that runs off the
end of the code stops before reaching data. Where the `db` line appears in the source
does not matter. `program_size` includes the data, so the stack still starts after it.

### Symbol rules
- Labels and data names share one namespace. Defining a name twice is an error.
  Using an undefined name is an error. Both report line and column; neither panics.
- Names: letters, digits, `_`; not a mnemonic, register or keyword. `NAME.len` is only
  valid for `db` names.

### Encoding of symbol references
Every reference to a symbol (jump/call targets and symbol immediates) is encoded as a
static operand with a **fixed 4-byte value**: `00 04 b3 b2 b1 b0` (big-endian, not
trimmed). Instruction sizes therefore never depend on symbol values, addresses are known
after one sizing pass, and the three-pass size guessing is removed. Plain numeric
immediates keep the trimmed encoding. `NAME.len` is a plain number known at parse time of
the definition but is also encoded with the fixed 4-byte form for simplicity.

`strtohx` stays in the repo for now but is no longer needed; examples move to `db`.

## 2. Host interface (minimal, for tests and sound)

The runtime stops talking to the process directly for output and sound:

```rust
pub trait Host {
    fn write_out(&mut self, bytes: &[u8]);                       // Printchar / Print / Println
    fn play_sound(&mut self, path: &str) -> Result<(), String>;  // PlaySound
}
pub struct StdHost;        // stdout; play_sound uses the `audio` cargo feature, else Err
pub struct CaptureHost { pub out: Vec<u8>, pub sounds: Vec<String> }   // for tests
```

- `ProgramRuntime::new()` uses `StdHost`; `ProgramRuntime::with_host(Box<dyn Host>)` sets another.
- New syscall **12 `PlaySound`**: `rb` = path pointer, `rc` = path length. Result in `ra`:
  `0` = ok, `1` = failed (the error text goes to stderr). It must never panic.
- `StdHost::play_sound`: behind the optional feature `audio` (off by default). Without the
  feature it returns `Err("efescript was built without the `audio` feature")`.
- `Print` keeps its trailing space and `Println` its newline (existing behaviour), minus the `dbg!`.

## 3. Running and exit

```rust
pub enum RunResult { Finished(u8), EndOfProgram, LimitReached }
impl ProgramRuntime {
    pub fn run(&mut self) -> RunResult;                         // until Finish or end
    pub fn run_with_limit(&mut self, max_instructions: u64) -> RunResult;
}
```

- `tick` returns the instruction's signal instead of discarding it; syscall `Finish`
  (5050, `rb` = code) ends the program with that code.
- The CLI exits with the `Finished` code (0 for `EndOfProgram`).
- `addN` / `subN`, pointer arithmetic and stack-pointer updates use wrapping arithmetic,
  so debug and release builds behave the same.

## 4. CLI

```
efescript <file.efe>            assemble and run
efescript <file.efec>           run precompiled bytes
efescript <file.efe> -o out     assemble to a file, do not run
  -d  dump runtime after the run      -r  print memory usage
```
No `./out.efec` is written unless `-o` is given. A missing file or an assembly error
prints a message with file, line and column and exits with code 2 (no Rust panic).

## 5. Tests

`crates/efescript/tests/` integration tests, all using `CaptureHost` and `run_with_limit`:
- every example assembles and runs to an expected output and exit code;
- `examples/jumps.efe`: conditional jumping (each of `je jne jgt jlt jge jle jz jnz`, taken and not taken);
- `examples/recursion.efe`: a recursive function (sum of 1..n by recursion through `call`/`ret` with stack-saved state);
- `examples/sound.efe`: `PlaySound` on `examples/ominous_worse.ogg`; the test asserts the host received that path (no audio device needed);
- one regression test per bug fixed in E0.2 / E0.3;
- assembler error tests (undefined symbol, duplicate symbol, bad operand) asserting `Err` with a position.
