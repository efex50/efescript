# efe-wasm

`efe-wasm` translates a `.wasm` module into a verified efevm module (roadmap E7.4b, design `crates/efescript/docs/e7.4-wasm-design.md`, sections 3, 4 and 7). It is pure data: bytes in, `efevm::Module` out. The engine's package path never links it; `efescript pack` (E7.4c) and development builds do. **Used by: nothing yet (E7.4c wires it into `efescript pack` and the engine's `wasm-mods` feature).**

## At a glance
| | |
|---|---|
| Path | `crates/efe-wasm` |
| Kind | library (`src/lib.rs`) plus integration tests |
| Depends on | `efevm` (path), `wasmparser` 0.245, `thiserror` 2. Dev: `wat` (wasm text for tests and the fuzz generator), `wasmi` 2 (reference runtime), `efevm-jit` (native only) |
| Targets | native and `wasm32-unknown-unknown` (`cargo check -p efe-wasm --lib --target wasm32-unknown-unknown`) |
| Safety | `#![forbid(unsafe_code)]`; never panics on input (the module is validated by `wasmparser` first) |
| Entry points | `translate(wasm, &ApiIndex, &TranslateOptions) -> Result<Module, TranslateError>`, `TRANSLATOR_VERSION` |

## Layout
```text
crates/efe-wasm/
├── Cargo.toml
├── README.md
├── structure.md
├── src/
│   ├── lib.rs       translate(): validation, efe.api check, import resolution, layout of the efevm function index space, the module builder calls, sections 12/13/14, error mapping
│   ├── parse.rs     module-level reader (types, imports, tables, memory, globals, exports, elements, data, start, custom sections); rejects what is outside the subset
│   ├── func.rs      function bodies: operand stack to registers, control flow to labels, host call lowering, the lazy-initialisation prologue
│   ├── abi.rs       wasm lowering of API types, `efe.provide` record decoder
│   └── error.rs     TranslateError, Loc (function index and name, byte offset)
└── tests/
    ├── common/      mod.rs (wat to Module, run on interpreter and JIT, `Pair`), host.rs (mock host from an ApiIndex), prov.rs (efe.provide writers), wgen.rs (random module generator), fuzz.rs (differential case runner)
    ├── numeric.rs   every integer/float operator, conversions (trapping and saturating) against Rust models
    ├── control.rs   block/loop/if, br/br_if/br_table with values, multi-value, select, locals hazards, calls, recursion, call_indirect traps, names
    ├── memory.rs    loads/stores of all widths, out of bounds, size/grow/fill/copy, data segments, start function and ctors
    ├── reject.rs    one rejection test per unsupported feature, imports that break the ABI, size limit
    ├── abi.rs       imports and events, efe.api hash, efe.provide records, sections 12/13/14, provides run on a mock host
    ├── guest.rs     the real guests of E7.4a (Rust and C) built for wasm32, translated and run
    ├── hardening.rs components, GC groups, block types, dead multi-value blocks, multi-value moves, data/memory limits, malformed-input mutation test
    ├── fuzz.rs      `fuzz_short` (150 cases) and `fuzz_long` (ignored) against wasmi
    └── smoke.rs
```

## How a module is translated
1. `parse.rs` reads the module parts (an allow-list; components, GC type groups and the like are rejected) and `scan_body` checks every operator, so a module outside the subset gets an error naming the construct. Then `wasmparser::Validator` runs with only the features the subset uses.
2. `efe.api` (8 bytes, the index hash) must equal `index.hash()`, else `ApiHash`. A module without it is accepted with the given index's hash (`require_efe_api` insists).
3. Imports. Module `<ns>@<v>`, name `f` is the index function `ns.f` (signature compared with the wasm lowering of the index types, a mismatch names both). Name `E.send|count|read` is an event operation. Section 12 records `(ns, v)` per namespace as imported; a namespace not in the index is `UnknownNamespace`. `ptr`, `&Struct`, `str`, `bytes` are `i32` on wasm and `i64` in efevm: an `i32` register is already zero-extended, so call sites pass them unchanged, and only a `ptr` result gets `i32.wrap_i64`.
4. Events. Each distinct `(namespace, event)` imported, sorted, is a section 14 entry (its position is the id); the three operations are lowered inline to `engine.events.send|count|read` calls with the id as a constant (`lower_host`).
5. Function index space: `[host imports][wasm defined functions][wrappers for imports used by the table or exports][provide wrappers]`. A provide whose record has `str`, `bytes` or `&Struct` gets a wrapper with the efevm (i64) signature that wraps the pointers and calls the wasm function, so section 13's lowered type check passes.
6. Bodies (`func.rs`). Registers: locals `r0..`, one scratch register, then one register per operand stack slot (`sbase + height`); wasm validation fixes the height, so there is no allocator, and call arguments and results are the top slots (no copying). `local.get` pushes a symbolic entry that is written to its slot only when needed (before the local is overwritten, before a call or branch, at block boundaries). Blocks and `if` materialise everything at entry; `br` moves the carried values down to the target's base slot (ascending moves, safe because destination is not above source); `br_table` with moves goes through one trampoline per target. Unreachable code after `br`/`return`/`unreachable` is skipped up to the matching `else`/`end`. A function that needs a register above 65534 fails with `TooLarge`.
7. Sections: memory 0 (same pages, data segments at their offsets), globals (an extra mutable `initialized` i32 when there is a start function or `__wasm_call_ctors`), table 0 and its active element segments (null stays unset), exports (functions and globals), names (wasm `name` section, else export names), 13 from `efe.provide`, 14 from the event imports. `ModuleBuilder::finish` verifies.
8. Start function and `__wasm_call_ctors`: every exported function starts with `if flag == 0 { flag = 1; call start; call ctors; flag = 2 }` (`emit_prologue`); the two initialisers themselves return at once once the flag is 2, so each runs exactly once.

## Notes
- Traps map one to one (out of bounds, divide by zero, integer overflow, invalid conversion, unreachable, undefined element, indirect type mismatch, stack exhaustion). `memory.grow` wraps efevm's `i64` result to `i32`.
- Not copied: DWARF, `producers`, `target_features`. Mod-defined structs (record kind 3) and unknown record kinds are skipped; importing the mod's own namespace is an `UnknownNamespace` error.
- The differential fuzz generator (`tests/common/wgen.rs`) emits WAT; floats are NaN-canonicalised after every operation that can produce a NaN, so results compare bit for bit. It was checked by mutation: removing the `local.tee` hazard handling in `func.rs` makes `fuzz_short` fail.
- clang 21 with lld 22 puts `.custom_section.X` variables in data segments, not custom sections. The generated C header therefore writes `efe.api` with inline asm for wasm, and C mods give their provides through `TranslateOptions::fallback` (`Fallback`, design doc 4.4); `tests/guest.rs` does that for the C guest and checks section 13 in full.
