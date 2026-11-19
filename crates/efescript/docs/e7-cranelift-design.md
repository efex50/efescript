# E7: native code through Cranelift (design)

Status: proposal, 2026-11-19. Replaces E7.1–E7.3 of `ROADMAP.md`; E7.4 (wasm → efescript)
stays as written.

## 0. Summary

A new crate, `crates/efevm-jit`, compiles verified `efevm` modules to native code with
Cranelift (`cranelift-codegen`, `cranelift-frontend`, `cranelift-jit`; `cranelift-object` later
for AOT). Native only: the web build keeps the interpreter. The JIT must be **observably
identical** to the interpreter: the same `Exit`, results, memory, globals, traps (kind and
location) and budget accounting, so a game can switch backends per module with no other change,
and lockstep multiplayer stays deterministic on mixed machines.

Main decisions:

1. **Registers become Cranelift variables of type `i64`.** The VM's registers are untyped 64-bit
   slots (SPEC 2), so every register is one `i64` `Variable`. `i32` ops `ireduce` their inputs and
   `uextend` the result. Floats are `bitcast` in and out. This is exact by construction; a typing
   pass that keeps values in their native type comes later (J7) as an optimisation.
2. **Traps are explicit branches, not hardware traps.** Division checks, bounds checks, float
   to int range checks and table checks branch to a cold block. The cold block stores the trap kind
   and the instruction index in the context and returns a status. There are no signal handlers.
   Cranelift's own `trapz`/`trapnz` and trapping `sdiv` are never emitted.
3. **Suspension is stackless re-entry, not stack switching.** `yield`, a host `Yield` and running
   out of budget all write the live registers of every JIT frame on the stack into the fiber's
   existing `Stack` (`regs`, `frames`, `pc`). Then they return a status up the native stack.
   Resuming calls the outermost function again with a *resume point id*. A dispatch block jumps
   straight to that point, reloads the registers, and re-enters the callee the same way. Fibers
   stay plain data (needed for E6.2 restart snapshots, the debugger and `fiber_backtrace`), and no
   `corosensei`-style platform stack code is needed.
4. **Budget is charged per basic block**, in both backends (a spec change, J0). The interpreter
   charges 1 unit per instruction and can stop between any two instructions. The JIT can't afford
   that check, so the unit of "may stop here" becomes the basic block. The rule, the same in both
   backends:
   - The budget is a signed counter during a run. On entering a block leader (including resuming
     at one and returning to the instruction after a call), if the counter is `<= 0` the fiber
     stops with `OutOfBudget` *at that leader*. Otherwise the block's cost (its instruction count)
     is subtracted, and the block runs even if the counter goes negative.
   - Bulk ops (`memory.copy/fill/grow`) subtract their extra byte-based cost the same way, with no
     immediate exit.
   - When a run ends with a negative counter, the overrun becomes the fiber's debt, paid first by
     the next resume (the existing debt rule). So a budget smaller than a block never livelocks,
     and both backends agree exactly on where every exit happens and on the units used.
   - `Instance::step` (for the debugger) runs exactly one instruction and charges nothing. A fiber
     stepped to the middle of a block is resumed by the interpreter, because every JIT resume
     point is a leader.
5. **`efevm` stays `#![forbid(unsafe_code)]`.** All `unsafe` (mapping code, calling it, the
   context pointer) lives in `efevm-jit`. `efevm` gains a small backend seam (section 3) but no
   unsafe and no Cranelift dependency.

## 1. Why Cranelift

- It is a pure Rust code generator built for JIT use. It compiles fast (wasmtime's baseline),
  targets x86-64, aarch64, riscv64 and s390x, and is heavily fuzzed.
- Its IR (SSA, typed values, blocks with parameters) is close to what `efevm` code already is:
  three-operand instructions over registers, with labels as block starts.
- `cranelift-frontend`'s `Variable` API builds SSA from mutable registers, so registers need no
  SSA construction of our own.
- The same lowering feeds `cranelift-object` for AOT `.o` files later.
- Not for the web: Cranelift emits machine code, not wasm. Native code on the web would need an
  efevm → wasm translator (a separate item, J8), so the browser keeps the interpreter for now.

## 2. Execution model of compiled code

### 2.1 The context

Every compiled function receives one pointer, `vmctx: *mut JitCtx`, a `#[repr(C)]` struct owned by
the running `Instance` for the duration of one `resume`. It holds:

- `mem_base: *mut u8` and `mem_len: u64`. These are reloaded after every call, host call and
  `memory.grow`, because memory can move when it grows.
- `globals: *mut u64`.
- `budget: i64`, the units left. Blocks subtract their cost and test the sign.
- `depth: u32` and `max_depth: u32`, the call depth for `StackOverflow`, plus `stack_limit: usize`
  for Cranelift's native stack-limit check (so deep recursion never overflows the thread stack).
- `table: *const TableEntry` and `table_len`. A `TableEntry` holds a canonical type id and a code
  pointer, or null for an unset slot.
- The trap record: `trap_kind: u32`, `trap_func: u32` and `trap_inst: u32`.
- The suspend record: a pointer to the fiber's `Stack`, which the cold paths write frames into.
- `host_call: extern "C" fn(vmctx, import: u32, window: *mut u64) -> u32`, the trampoline into
  `HostFunc<H>`. The trampoline is generic over `H`, but the compiled code is not.

### 2.2 Function signature

```text
fn f(vmctx: i64, window: i64, resume: i32) -> i32   // status
```

- `window` points to `max(P, R)` `u64`s: the arguments on entry, and the results on a normal
  return. This is the interpreter's call window (SPEC 6), so a host call or a call into the
  interpreter uses the same layout.
- `resume` is 0 for a normal call. On re-entry it is the id of the suspension point to continue
  from.
- Status: `0` returned, `1` suspended (yield, host yield or out of budget, with the reason in the
  context), `2` trapped.

### 2.3 Lowering

- **Blocks.** A Cranelift block starts at instruction 0, at every label target and after every
  terminator, call, host call and `yield` (those are the suspension points). Each block's cost (its
  instruction count) is known at compile time.
- **Budget.** Each block starts with `if budget <= 0 { suspend } budget -= cost` (section 0,
  item 4). Two cheap instructions and a well-predicted branch. A later optimisation merges the checks of
  straight-line blocks with no back edge between them; the charge points are part of the spec, so
  this needs a spec rule too.
- **Integer ops** map one to one (`iadd`, `imul`, `rotl`, `clz`, `popcnt`, `icmp` + `uextend`
  …). The 32-bit forms reduce, operate and zero-extend. `div/rem` get explicit zero and
  `MIN / -1` checks, and `rem_s` of `MIN % -1` is 0 (SPEC 5).
- **Floats**: `fadd` … `fsqrt`, `ceil/floor/trunc/nearest`, and `fmin/fmax`, checked against the
  interpreter's NaN and ±0 behaviour (the interpreter is the reference, J6). `trunc_*` gets range
  checks to the trap block, and `trunc_sat_*` uses `fcvt_to_*_sat`.
- **Memory.** `addr = reg[base] + off` uses `uadd_overflow`. Then it checks
  `addr + size <= mem_len` and does a `load`/`store` with `little` endianness from `mem_base +
  addr`, unaligned allowed. Guard-page tricks need 32-bit addressing, and addresses are `u64`
  (SPEC 3), so the check stays explicit. Bulk ops and `memory.grow` call Rust helpers.
- **Control**: `jmp` → `jump`, `br_if/br_ifz` → `brif`, `br_table` → `br_table` (index as
  `u32`), `select` → `select`, `unreachable` → trap block.
- **Calls.** A direct `call` stores the window registers to a stack slot, then calls the callee's
  native function (`depth += 1`, checked at the callee's entry), checks the status, and loads
  the results back. `call_indirect` checks bounds, the null entry and the type id, then calls
  indirectly. An import is the `host_call` trampoline plus the same status handling. Registers
  outside the window stay in SSA across the call (SPEC 6: no other caller register changes).
- **Suspension (cold path).** Every suspension point has an id. When a callee returns status 1, or
  a budget check or `yield` suspends locally, the cold block appends this function's frame to the
  fiber `Stack`: function index, pc of the suspension point, and all `nregs` registers (stores
  from the current variables). Then it returns 1. The innermost frame is written first, so the
  `Stack` is rebuilt innermost-first and gets reversed once by the Rust side. The result is
  exactly the interpreter's layout, so a fiber suspended by the JIT can be resumed by the
  interpreter and the other way round.
- **Re-entry.** The entry block switches on `resume`. Point `k` reloads all registers from the
  frame the Rust side hands it (via the context) and jumps to the point's block. If that point is
  a call, it calls the callee with the callee's resume id, so the whole native stack is rebuilt
  without re-running any instruction. A script that yields every frame inside an endless loop stays
  in compiled code.
- **Traps (cold path).** The trap block stores the kind, function and instruction index, then
  returns 2. Each caller on the way out adds its own `(func, call inst)` to the trace in the
  `Stack`. This is the same backtrace the interpreter produces for `trap_location` and
  `backtrace`.

### 2.4 What stays interpreted

- The **debugger** (E4.5): breakpoints and single-step always use the interpreter. Because frames
  are interchangeable, attaching the debugger to a fiber suspended by the JIT just works.
- Functions the JIT refuses to compile (a size cap, or an op not yet lowered during bring-up).
  Mixed calls go through the window layout: compiled code calls an `interp_call` trampoline, and
  the interpreter calls compiled code through `Backend::call`.

## 3. Changes in `efevm` (no unsafe)

- `pub trait Backend<H>`: `compile(&Module) -> Result<Box<dyn Compiled<H>>, ..>`, and
  `Compiled::resume(&mut self, view: InstanceView<'_, H>, fiber, budget) -> (Exit, u64)`.
  `InstanceView` gives safe access to memory, globals, the fiber `Stack`, the host functions and
  limits. `Instance::new_with(.., backend)` picks the backend; plain `Instance::new` stays
  interpreter only.
- The fiber types (`Stack`, `Frame`) become `pub` with documented invariants, or get accessor
  methods, so the JIT crate can write frames.
- J0's per-block budget: precompute block costs per function in `FuncInfo` at load (it is already
  pre-decoded), and charge at block entry in `interp::run`.
- Memory exposes `as_mut_ptr`/`len` only through `efevm-jit`'s unsafe wrapper. `efevm` only
  needs `Memory::bytes_mut(&mut self) -> &mut [u8]`, and the jit crate takes the pointer from that
  slice.

## 4. Engine integration

- `efeengine-core` feature `jit` (off by default at first; never on `wasm32`). `EngineConfig::
  scripting.backend: Backend::{Interpreter, Jit, Auto}`. `Auto` uses the JIT where it is
  available and falls back with a log line if compiling fails.
- **Mods are untrusted input** (E8.8). A JIT lowering bug is a memory-safety bug in the host, while
  an interpreter bug is only a wrong result. So `ModPolicy`'s `Grant` gets `jit: bool`, default
  `false` until J6's differential fuzzing has run for a while. Game code (`.efa`, `.efvm` by
  path) may use the JIT from J5.
- The `ScriptCache` keeps caching `.efvm` bytes only. Compiled code is not cached on disk: Cranelift
  is fast enough, and native code from disk would be a new attack surface. AOT is the place for
  that (J8).

## 5. Phases

| Phase | Content | Done when |
|---|---|---|
| **J0** | Spec: per-block budget charging, suspension points, the frame layout as a public contract (SPEC draft 8). Interpreter switched to per-block charging. | Existing tests updated for the new budget exits; `resume_units` semantics documented. |
| **J1** | `efevm-jit` skeleton: `Backend` seam, the context, compile all functions of a module, integer/const/mov/select/control ops, direct calls, `ret`. Runs only to completion (no suspension yet; out of budget traps for now). | The `efevm` integer and control tests pass under the JIT. |
| **J2** | Floats and conversions, memory loads/stores/bulk/grow, globals, all traps with locations and backtraces. | All `efevm` opcode tests pass under both backends. |
| **J3** | Host calls (trampoline, `Caller` access), `call_indirect`, call depth and native stack limit. | The engine's `engine.*` host tests pass with the JIT. |
| **J4** | Suspension and re-entry: budget, `yield`, host `Yield`, fibers suspended by one backend and resumed by the other. | Fiber, budget and yield tests pass, plus a test resuming every suspension point across backends. |
| **J5** | Engine integration: feature `jit`, `EngineConfig` backend choice, `Grant.jit`, `script_demo` on the JIT, the debugger forcing the interpreter. | `cargo xtask test` with `--features jit` green. |
| **J6** | Differential testing: every existing module test runs under both backends, comparing exits, results, memory, globals, traces and units used. A fuzz target generates random *verified* modules (from the builder, not mutated bytes) and compares the backends. Benchmarks against the interpreter's 3.1 ns/instruction. | Fuzzed for a set time with no difference; numbers in `structure.md`; `Grant.jit` default reconsidered. |
| **J7** | Optimisations: register typing (no bitcasts for registers that only ever hold one type), merged budget checks, hoisted bounds checks, lazy compile (tier up after N calls or back edges). | Measured wins only. |
| **J8** | AOT with `cranelift-object` (native `.o` for shipped game code) and/or efevm → wasm for the web build. | Separate design. |

## 6. Risks

- **Budget determinism.** J0 changes when out-of-budget happens. Any test or game logic that relies
  on exact per-instruction budget stops has to be updated. This is acceptable: there is no backward
  compatibility.
- **Float NaN and ±0 corner cases** of `min/max/nearest` differ between instruction sets.
  Lowering must match the interpreter bit for bit where SPEC fixes the result (J6 catches the
  rest).
- **Frame size.** Writing all `nregs` registers on suspension is O(nregs) per frame. That is fine
  for per-frame yields; huge-register functions can be capped (interpreted) if it ever shows up.
- **Compile time** of very large modules: compile lazily per function (J7), or on a worker thread
  from the engine's pool.
- **Unsafe surface.** It is kept to one crate. `miri` cannot run JIT code, so the safety of the
  generated code rests on J6.
