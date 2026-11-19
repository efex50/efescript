# strtohx

Small standalone command-line helper (not a library). It turns text arguments into lines of efescript assembly (`push64`/`push32`/`push16`/`push8 0x...`) that, when run, place that string on the VM stack. It is a developer tool for writing string constants in `.efe` programs.

## At a glance
| | |
|---|---|
| Path | `crates/efescript/strtohx` |
| Kind | binary (`src/main.rs`) |
| Depends on (workspace) | none (no Cargo dependency on `efescript`; only its output format is efescript-specific) |
| Used by | nobody in the build; used by hand when authoring `.efe` files |
| Key external deps | none |
| Targets | native CLI; no target-specific code |
| Features | none |

## Layout
```text
crates/efescript/strtohx/
├── Cargo.toml    # package strtohx 0.1.0, no dependencies
└── src/main.rs   # entire tool (125 lines)
```

## Main concepts
Usage: `strtohx "hello world" "second string"` (each argument is handled separately; at least one is required, otherwise it panics with "not enough args").

For each argument it takes the UTF-8 bytes and splits them greedily into chunks of 8, then 4, then 2, then 1 bytes, producing one push instruction per chunk. The chunk bytes are printed as a big-endian hex number (first byte of the text is the most significant), which matches how efescript's `pushN` writes big-endian into the stack, so the text appears in order in VM memory. It then prints the input, the instructions and `total len is N` (the byte length, which is what to put in `rc`).

Example (actual output):
```text
$ strtohx "hello world"
push64 0x68656C6C6F20776F
push16 0x726C
push8 0x64

total len is 11
```
(A 11-byte string is split 8 + 2 + 1; a 3-byte string becomes `push16` + `push8`.)
The output is meant to be pasted into an `.efe` file after `mov rbp,rsp`, then `mov rb,rbp` / `mov rc,<len>` and a Print/Println syscall. Hex digits are upper-case; efescript accepts both cases.

## Public API
None; it is a binary with no library target. The only interface is the command line above and stdout.

## Status and known gaps
- No arguments -> `panic!("not enough args")` rather than a usage message.
- No newline escaping: a trailing newline has to be added by hand (for example an extra `push8 0xA`, or a literal newline inside the argument).
- Unused `KalanType` plumbing and an unused `as_le_bytes!` copy are rough edges; helper function is named `ayıkla` (Turkish for "extract").
- No tests, no output-file mode.
