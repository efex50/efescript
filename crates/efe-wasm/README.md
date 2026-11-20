# efe-wasm

Translates a `.wasm` module into a verified efevm module (`efevm::Module`), so a mod written in Rust or C/C++ runs on the same VM as an `.efa` mod. This is roadmap item E7.4b; the design, the guest ABI and the custom sections are in `crates/efescript/docs/e7.4-wasm-design.md`.

```rust
let index = efevm::ApiIndex::from_json(&std::fs::read_to_string("game.efapi")?)?;
let module = efe_wasm::translate(&std::fs::read("mod.wasm")?, &index, &Default::default())?;
std::fs::write("mod.efvm", module.encode())?;
```

- Pure data, no I/O, `#![forbid(unsafe_code)]`; builds for `wasm32-unknown-unknown` too.
- The wasm subset: MVP, sign extension, non-trapping float-to-int, mutable globals, `memory.copy`/`memory.fill`, multi-value, one `funcref` table. Everything else (atomics, SIMD, exceptions, GC, tail calls, memory64, multi-memory, extended-const, `memory.init`/`data.drop`, other reference types, imported memory/table/globals) is rejected with an error that names the construct, the function and the byte offset.
- The module must be built without browser-style flags (`+atomics`, `--shared-memory`, `--import-memory`); the error for an imported memory says so.
- `TRANSLATOR_VERSION` goes into cache keys (wasm bytes, index hash, translator version).

Tests: `cargo test -p efe-wasm` (run it on the build server, `python3 scripts/remote.py test -p efe-wasm`). `tests/fuzz.rs` compares random modules against `wasmi` on the efevm interpreter and JIT; the long run is `EFE_WASM_FUZZ_SECS=600 cargo test -p efe-wasm --release --test fuzz -- --ignored fuzz_long --nocapture`. Layout and internals: `structure.md`.
