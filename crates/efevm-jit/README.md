# efevm-jit

A Cranelift JIT backend for [`efevm`](../efevm). It compiles a verified `efevm::Module` to native
code and plugs into `Instance::set_backend`. The JIT is observably identical to the interpreter:
the same exits, results, memory, globals, budget accounting, trap kinds, trap locations and
backtraces; a fiber suspended by one backend can be resumed by the other.

```rust
let mut inst = efevm::Instance::new(&module, &registry.imports(), efevm::Limits::default())?;
efevm_jit::enable(&mut inst, &efevm_jit::JitOptions::default())?; // falls back: keep the interpreter on Err
let fiber = inst.spawn("main", &[])?;
let exit = inst.resume(fiber, &mut host, 10_000);
```

- Native only (x86-64 and aarch64 through Cranelift); the crate is empty on `wasm32`.
- All `unsafe` of the JIT lives here; `efevm` stays `#![forbid(unsafe_code)]`.
- Mods are untrusted input: a lowering bug is a host memory-safety bug. The engine (`efeengine-core`, feature `jit`) keeps
  it off for mods unless their `Grant::jit` is true. Differential fuzzing (J6) found no difference in
  14 minutes / 9k programs; see `structure.md`.
- Design: `crates/efescript/docs/e7-cranelift-design.md`. Layout, ABI and tests: `structure.md`.
- Benchmark: `cargo run -p efevm-jit --release --example bench`.
