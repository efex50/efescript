# proc_macros

Tiny proc-macro crate that belongs to `efescript`. It exposes two function-like macros that expand to `pub const` definitions, so values can be fixed at compile time of the consuming crate.

## At a glance
| | |
|---|---|
| Path | `crates/efescript/proc_macros` |
| Kind | proc-macro (`[lib] proc-macro = true`) |
| Depends on (workspace) | none |
| Used by | `efescript` (invoked at the top of `src/lib.rs`) |
| Key external deps | none (only the compiler-provided `proc_macro`) |
| Targets | runs on the host at compile time, so target-independent; the generated code is plain `usize` consts |
| Features | none |

## Layout
```text
crates/efescript/proc_macros/
├── Cargo.toml    # package proc_macros 0.1.0, [lib] proc-macro = true, no dependencies
└── src/lib.rs    # the two macros (27 lines)
```

## Main concepts
Both macros ignore their input tokens and build a string that is parsed back into a `TokenStream`.

- `max_addr!()` expands to `pub const MAX_ADDR_SIZE:usize = 281474976710656;` (2^48, computed with `2_usize.pow(48)`). The VM's maximum addressable size.
- `stack_page_count!()` expands to `pub const STACK_PAGE_COUNT:usize = N;` where `N` is read from the environment variable `STACK_PAGE_COUNT` at macro-expansion time (default `2`). An unparsable value prints the parse error to stderr and panics, which aborts compilation. Note `env::var` is read when the macro is expanded, but cargo does not know about this dependency, so changing the variable does not trigger a rebuild of `efescript` by itself. The doc comment says "PAGE_SIZE * STACK_PAGE_COUNT" is the max stack size.

Usage (as in `efescript/src/lib.rs`):
```rust
proc_macros::max_addr!();
proc_macros::stack_page_count!();
// now MAX_ADDR_SIZE and STACK_PAGE_COUNT exist as crate-level pub consts
```
Build with a custom stack size: `STACK_PAGE_COUNT=8 cargo build -p efescript` (after a clean/touch of `efescript`).

## Public API
- `max_addr!()` -> `pub const MAX_ADDR_SIZE: usize`
- `stack_page_count!()` -> `pub const STACK_PAGE_COUNT: usize`

## Status and known gaps
- The generated constants are not used anywhere in `efescript` yet (no stack bound checks or address validation reference them).
- An older design that read `efearena_size` from the environment is left as commented-out code (`src/lib.rs`).
- No `cargo:rerun-if-env-changed` mechanism, so the env override can be stale (see above); there are no tests.
