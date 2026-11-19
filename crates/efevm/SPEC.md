# efevm v1 specification

Status: draft 6, 2026-11-15 (draft 1 plus the clarifications in "Changes from draft 1" and the changes in "Changes from draft 2", "Changes from draft 3", "Changes from draft 4" and "Changes from draft 5" at the end). This is roadmap item E1.1 (`crates/efescript/ROADMAP.md`).
`efevm` is the new core VM for efescript. It lives in its own crate (`crates/efevm`)
next to the old VM in `crates/efescript`, which keeps working until the assembler
moves over (E2).

Decisions taken: per-function **virtual registers**, **three-operand** instructions with
no flags, **compact variable-width** file encoding (decoded once at load into a
fixed-width in-memory form), cooperative **coroutines** (fibers).

## 1. Principles

1. Same behaviour on every host, including `wasm32-unknown-unknown`: no `usize` in semantics.
2. Nothing a module does can panic or corrupt the host. Loading returns `LoadError`; running returns `Exit`.
3. Code is not data: instructions live outside the script's memory and are immutable.
4. No I/O in this crate. Everything outside the VM is a host function.
5. No `unsafe` in the interpreter. No global or static state. No dependencies beyond `core`/`alloc`/`std` collections (a small error-derive crate is acceptable).
6. One calling convention (section 6), shared later by JIT and AOT.

## 2. Values and registers

- Value types: `i32`, `i64`, `f32`, `f64`.
- A register is a 64-bit slot (`u64`). Registers are untyped at run time:
  - `i32` operations read the low 32 bits and write a **zero-extended** result;
  - `f32` is stored as its bits in the low 32 bits (upper bits zero); `f64` as its bits.
- Each function declares `nregs` (0–65535). Its registers are `r0 … r(nregs-1)`, private
  to each activation. Parameters arrive in `r0 … r(nparams-1)`; all other registers start at 0.
- Integer arithmetic wraps. Shifts and rotates use the count modulo the bit width.
- Floats follow IEEE 754; NaN payloads are not specified beyond "some NaN".

## 3. Memory

- One linear memory per instance: little-endian bytes, addressed by `u64`, sized in
  64 KiB pages. A module declares `min_pages` and optional `max_pages`; the embedder sets
  a hard cap. Memory starts zeroed.
- Effective address of a load/store = `reg[base] (u64) + offset (u32 immediate)`. If the
  access does not lie fully inside memory (or the addition overflows) → `Trap::OutOfBounds`.
- No alignment requirement.
- Data segments are copied into memory at instantiation (`offset` + bytes); a segment that
  does not fit is a `LoadError`.

## 4. Module

A module contains, in this order of indices:

| Part | Content |
|---|---|
| types | function types `(params: [ValType]) -> (results: [ValType])` |
| imports | host functions: `(namespace: String, name: String, type index)` |
| functions | `type index`, `nregs`, code (instruction list) |
| table | one list of function indices for indirect calls (may be empty) |
| memory | `min_pages`, `max_pages?` |
| globals | `ValType`, mutable flag, initial constant |
| exports | `name → function index` (and `name → global index`) |
| data | `offset`, bytes |
| names (optional) | function, import and register names |
| lines (optional) | per function: instruction index → (file id, line, column) |
| api (optional) | the API index hash and `(namespace, version)` of every namespace the module was assembled against |
| interface (optional) | what the module provides to other modules when loaded as a mod: functions with typed, named parameters and docs, the struct layouts they use, and the exchange (scratch) region for copied arguments (9.2) |

Function index space: imports first (`0 … nimports-1`), then module functions. `call`
targets any function index; calling an import is a host call.

## 5. Instructions

Notation: `d` destination register, `a b c` source registers, `imm` immediate, `L` label
(an instruction index inside the same function), `f` function index. `T` ∈ {`i32`, `i64`},
`F` ∈ {`f32`, `f64`}. Every instruction costs 1 unit of budget, except that `memory.copy`, `memory.fill` and `memory.grow` also pay for their size (see "Changes from draft 2", item 1).

**Constants and moves**
- `const.i32 d, imm32` · `const.i64 d, imm64` · `const.f32 d, imm` · `const.f64 d, imm`
- `mov d, a` (copies all 64 bits)

**Integer (for each T)**
- `T.add d,a,b` `sub` `mul` `and` `or` `xor` `shl` `shr_s` `shr_u` `rotl` `rotr`
- `T.div_s` `T.div_u` `T.rem_s` `T.rem_u`: divisor 0 → `Trap::DivideByZero`; `div_s` of `MIN / -1` → `Trap::IntegerOverflow`; `rem_s` of `MIN % -1` = 0.
- `T.clz d,a` `T.ctz d,a` `T.popcnt d,a` `T.eqz d,a`
- Comparisons, result `i32` 0 or 1: `T.eq` `ne` `lt_s` `lt_u` `gt_s` `gt_u` `le_s` `le_u` `ge_s` `ge_u`

**Float (for each F)**
- `F.add` `sub` `mul` `div` `min` `max` `copysign` (three operands)
- `F.abs` `neg` `sqrt` `ceil` `floor` `trunc` `nearest` (two operands)
- Comparisons, result `i32`: `F.eq` `ne` `lt` `gt` `le` `ge`

**Conversions** (two operands)
- `i32.wrap_i64` · `i64.extend_i32_s` · `i64.extend_i32_u`
- `T.extend8_s` · `T.extend16_s` · `i64.extend32_s`
- `T.trunc_F_s` · `T.trunc_F_u`: NaN → `Trap::InvalidConversion`; out of range → `Trap::IntegerOverflow`
- `T.trunc_sat_F_s` · `T.trunc_sat_F_u` (saturating, NaN → 0, never trap)
- `F.convert_T_s` · `F.convert_T_u` · `f32.demote_f64` · `f64.promote_f32`
- `i32.reinterpret_f32` · `i64.reinterpret_f64` · `f32.reinterpret_i32` · `f64.reinterpret_i64`

**Select**
- `select d, cond, a, b`: `d = if reg[cond] as i32 != 0 { a } else { b }`

**Memory**
- `i32.load d, base, off` · `i64.load` · `f32.load` · `f64.load`
- `i32.load8_s` `i32.load8_u` `i32.load16_s` `i32.load16_u`
- `i64.load8_s` `i64.load8_u` `i64.load16_s` `i64.load16_u` `i64.load32_s` `i64.load32_u`
- `i32.store src, base, off` · `i64.store` · `f32.store` · `f64.store`
- `i32.store8` `i32.store16` · `i64.store8` `i64.store16` `i64.store32`
- `memory.size d` (pages, i64) · `memory.grow d, pages` (old size in pages, or -1 as i64 on failure)
- `memory.copy dst, src, len` · `memory.fill dst, value, len` (trap if either range is out of bounds; `copy` handles overlap)

**Globals**
- `global.get d, g` · `global.set g, a` (setting an immutable global is a `LoadError` at verification)

**Control**
- `jmp L` · `br_if a, L` (taken when `reg[a] as i32 != 0`) · `br_ifz a, L`
- `br_table a, [L0 … Ln-1], Ldefault` (index `reg[a] as u32`)
- `call f, base` · `call_indirect type, a, base` (table index in `reg[a]`; out of range or unset → `Trap::UndefinedElement`; wrong type → `Trap::IndirectCallTypeMismatch`)
- `ret base`
- `yield` (suspends the fiber: `Exit::Yield`; resuming continues after it)
- `unreachable` (`Trap::Unreachable`) · `nop`

Running past the last instruction of a function is impossible: the verifier requires
every function to end in `ret`, `jmp`, `br_table` or `unreachable`.

Opcode numbers are explicit constants in the source (`#[repr(u16)]` with a written
discriminant on every variant) and are never reused or renumbered once released. The
opcode table is generated into `OPCODES.md` by a test.

## 6. Calling convention

`call f, base` (the callee has `P` parameters and `R` results):
- arguments are the caller's registers `base … base+P-1`;
- after the call, results are in the caller's registers `base … base+R-1`;
- the verifier requires `base + max(P, R) <= nregs` of the caller;
- no other caller register is changed by the call.

`ret base` returns the callee's registers `base … base+R-1` as results.

Host functions use the same window: they receive `args: &[u64]` (length `P`) and write
`results: &mut [u64]` (length `R`).

The call stack (return positions and register frames) is held by the VM, outside script
memory. Its depth limit is set by the embedder; exceeding it → `Trap::StackOverflow`.

## 7. Execution, fibers and exits

```rust
pub enum Exit {
    Done,                 // the fiber's entry function returned; results are available
    Yield,                // `yield` executed, or a host function asked to yield
    OutOfBudget,          // budget used up; resume to continue
    Trap(Trap),           // the fiber is dead; the instance stays usable
}
pub enum Trap {
    OutOfBounds, DivideByZero, IntegerOverflow, InvalidConversion, Unreachable,
    StackOverflow, UndefinedElement, IndirectCallTypeMismatch,
    Host(String),         // a host function failed
}
```

- An `Instance<H>` owns memory, globals and its fibers. `H` is the embedder's host state.
- A **fiber** is one coroutine: its own call stack and registers; memory and globals are shared.
- API shape:
  ```rust
  let module = Module::decode(bytes)?;            // or built with ModuleBuilder; always verified
  let mut reg = HostRegistry::<H>::new();
  reg.define_namespace("env", 1, "game", "Console output.")?;
  reg.register(HostFn::new("env", "print").doc("Print `text`.").param("text", ApiType::Str),
      |cx, args, results| { let s = cx.read_str(args.str(0)?)?; … Ok(HostOk::Continue) })?;
  let mut inst = Instance::new(&module, &reg.imports(), Limits::default())?; // name + type + versions
  let fiber = inst.spawn("main", &[])?;                                  // export name + arguments
  match inst.resume(fiber, &mut host, 10_000) { Exit::Done => inst.results(fiber), … }
  ```
- Host functions are registered in a `HostRegistry` (namespace, name, typed and named
  parameters and results, doc); `HostRegistry::imports()` is the only producer of a non-empty
  `Imports`, and `HostRegistry::index()` is the same table as an API index
  (`crates/efevm/API_INDEX.md`). A host function receives typed `Args`/`Results` (by logical
  parameter index; a `str` is one parameter, two slots) and a `Caller` with access to `&mut H`,
  the instance memory (checked `read`/`write`, `read_bytes`, `read_str`, `read_struct`,
  `read_u32` … helpers) and globals. It returns `Ok(HostOk::Continue)`, `Ok(HostOk::Yield)`
  or `Err(Trap::Host(..))`. It cannot call back into the VM in v1.
- Fibers are scheduled by the embedder (resume whichever it wants); the VM never switches by itself.
- `Limits`: max memory pages, max call depth, max register stack size, max fibers (defaults in "Changes from draft 2", item 2).
- `Instance::call` runs one export to completion in a single resume ("Changes from draft 2", item 3).

## 8. Verification (at load, before anything runs)

- All indices in range: types, functions, imports, globals, table entries, exports, register numbers `< nregs`.
- Every label is a valid instruction index of the same function.
- Call windows fit (section 6); `call_indirect` type index valid.
- `global.set` only on mutable globals; data segments and memory limits consistent.
- Every function ends in a terminator; `br_table` has a default.
- Import resolution: every import must exist in `Imports` with an identical type, and every
  namespace recorded in the module's `api` section must exist with the same version; every
  event of the `events` section must be registered with the same payload (9.3; this and the
  import rule are link-time checks).
- The `interface` section's rules (9.2) and the `events` section's rules (9.3).

Because registers are untyped 64-bit slots and memory is bounds-checked, a verified
module cannot break host memory safety even if it mixes types.

## 9. Binary format (`.efvm`, version 1)

- Header: bytes `EFVM`, then `u16` little-endian version (`1`).
- Then sections: `id: u8`, `size: uleb`, payload. Known ids in ascending order, each at
  most once: 1 types, 2 imports, 3 functions, 4 table, 5 memory, 6 globals, 7 exports,
  8 data, 9 code, 10 names, 11 lines, 12 api, 13 interface, 14 events. Unknown ids ≥ 128 are skipped (custom sections).
- Integers are LEB128 (`uleb` unsigned, `sleb` signed). Strings are `uleb` length + UTF-8.
  Floats are fixed little-endian IEEE bytes.
- Code section: for each function, `uleb` instruction count, then instructions.
- Instruction: `opcode` as `uleb`, then operands in the order of section 5: registers,
  function/type/global indices and labels as `uleb`; `imm32`/`imm64` as `sleb`;
  load/store offsets as `uleb`; `br_table` as `uleb` count then that many labels then the default.
- A decoder must reject: truncated input, over-long LEB values, sections out of order,
  trailing bytes, counts that exceed the remaining input (no pre-allocation by untrusted counts).

### 9.1 Section payloads (added in draft 2)

LEB widths: every `uleb` is a `u32` unless marked `u64`; `imm32` is an `i32` `sleb`, `imm64` an
`i64` `sleb`. A LEB is over-long (rejected) if it uses more than `ceil(bits/7)` bytes (5 for 32-bit,
10 for 64-bit) or if its last byte sets bits that do not fit the type (for `sleb`: the unused
bits must equal the sign bit). Zero-padded encodings within the maximum length are accepted.
`ValType` is one byte: `0` i32, `1` i64, `2` f32, `3` f64. Booleans/flags are one byte `0` or `1`.
`vec(X)` = `uleb` count, then that many `X`.

| id | section | payload |
|---|---|---|
| 1 | types | `vec(functype)`; functype = `vec(ValType)` params, `vec(ValType)` results |
| 2 | imports | `vec(import)`; import = `string` namespace, `string` name, `uleb` type index |
| 3 | functions | `vec(func)`; func = `uleb` type index, `uleb` nregs (≤ 65535) |
| 4 | table | `vec(uleb u64)`; `0` = unset entry, otherwise function index + 1 (at most 2^32) |
| 5 | memory | flag byte (`0` no max, `1` has max), `uleb` min_pages, then `uleb` max_pages if flag is 1 |
| 6 | globals | `vec(global)`; global = `ValType`, mutable flag, init: `sleb` i32 / `sleb` i64 / 4 bytes f32 / 8 bytes f64 |
| 7 | exports | `vec(export)`; export = `string` name, kind byte (`0` function, `1` global), `uleb` index |
| 8 | data | `vec(segment)`; segment = `uleb u64` offset, `vec(byte)` bytes |
| 9 | code | `vec(body)`, count equal to the functions count; body = `vec(instruction)` |
| 10 | names | `vec(fname)` then `vec(rname)`; fname = `uleb` function index, `string`; rname = `uleb` function index, `uleb` register, `string` |
| 11 | lines | `vec(string)` file names, then `vec(flines)`; flines = `uleb` function index, `vec(entry)`; entry = `uleb` instruction index, `uleb` file id, `uleb` line, `uleb` column |
| 12 | api | 8 bytes index hash (`u64` little-endian), then `vec(ns)`; ns = `string` namespace, `uleb` version |
| 13 | interface | see 9.2 |
| 14 | events | see 9.3 |

- A missing section means "empty" (memory: `min_pages` 0, no max). A functions section without a
  code section (or the other way round with a non-zero count) is rejected.
- The encoder writes a section only when it is non-empty (memory: when `min_pages != 0` or a max
  is set; api and interface: whenever the module has one, even when empty; events: when non-empty), so encoding is
  deterministic: `decode(encode(m)) == m`.
- The section `size` must equal exactly the bytes its payload uses. Custom sections (id ≥ 128) may
  appear anywhere and are skipped; ids 0 and 15–127 are rejected.
- The verifier rejects an api section whose namespace names are not identifiers joined by dots
  or appear twice (`VerifyError::ApiNamespace`).
- Register operands are `uleb` values ≤ 65535 (larger is a decode error). Function indices in
  names/lines use the full function index space (imports first).
- Instruction operands follow section 5 exactly, e.g. `select d, cond, a, b` is four register
  `uleb`s, `i32.store src, base, off` is two registers then the `uleb` offset, `const.f32` is the
  register then 4 raw bytes, `global.set g, a` is the global index then the register,
  `call_indirect type, a, base` is type index, register, register.

In memory, instructions are decoded once into a fixed-size form (for example
`struct Inst { op: Op, a: u32, b: u32, c: u32 }` with 64-bit constants and `br_table`
targets in side tables). That form is an implementation detail, not part of this spec.

### 9.2 Section 13 `interface`

Design: `crates/efescript/docs/e8-design.md` sections 5–7; packages: `MOD_PACKAGE.md`. The
binary format version stays `1`: the section is additive, and a module without it means
exactly what it meant before.

The section lists what a module provides when it is loaded as a mod: each **provide** maps a
name in the mod's namespace to one of the module's own functions, with the typed and named
signature and the doc that go into the API index. The engine registers one host function per
provide (the trampoline) when the mod loads, so declared exports go through the registry like
every other callable function.

```
interface = scratch, vec(struct), vec(provide), vec(pevent)
scratch   = flag byte; if 1: uleb u64 offset, uleb size
struct    = string name, string owner, cflag byte; if 1: u32 component id (4 bytes LE),
            string doc, uleb size, uleb align, vec(field)
field     = string name, prim byte, uleb count, uleb offset, string doc
provide   = string name, uleb function index, string doc, vec(param) params, vec(param) results
param     = string name, apitype
apitype   = byte: 0 i32, 1 i64, 2 f32, 3 f64, 4 bool, 5 entity, 6 str, 7 bytes,
            8 struct (followed by string struct name)
prim      = byte: 0 u8, 1 i8, 2 u16, 3 i16, 4 u32, 5 i32, 6 u64, 7 i64, 8 f32, 9 f64, 10 bool, 11 entity
pevent    = string name, pflag byte; if 1: string payload struct name, string doc
```

- `ptr` has no encoding: a raw address means nothing in another module's memory, so it can
  be neither a parameter nor a result of a provide. The verifier rejects it in an in-memory
  `Interface` (rule 5); the encoder, which cannot fail, writes it as byte `0xFF`, which the
  decoder rejects with `BadApiType(255)`.
- `owner` empty means the struct is defined by this module (it is registered under the mod's
  name when the mod loads). A non-empty owner (`engine`, a game name, a dependency's id) means
  a **copy** of a struct defined elsewhere, recorded by the assembler from the index it built
  against; at load the registry must hold a struct of that name with the same size, alignment
  and fields (names, types, counts, offsets; docs and owner are not compared), otherwise the
  load fails by name. Copies make the mod's own index self-contained.
- In memory: `ModuleData.interface: Option<Interface>` with `Interface { scratch:
  Option<(u64, u32)>, structs: Vec<StructDesc>, provides: Vec<Provide>, events:
  Vec<ProvidedEvent> }` (`Default`; construct it with `..Default::default()`), `Provide { name,
  func, doc, params: Vec<Param>, results: Vec<Param> }`, `ProvidedEvent { name, payload:
  Option<String>, doc }`, reusing `StructDesc` and `Param` of `efevm::api`. A provided event is
  an event of the mod's own namespace (`<id>.name`) that the engine registers when the mod
  loads; it is what other modules then reference in their section 14. The encoder writes the section whenever `interface` is `Some`, even when empty.
- Decode errors: `BadApiType(u8)`, `BadPrim(u8)`; bad flag bytes are `BadFlag`. Vectors of
  this section are never pre-allocated from their counts.

Verifier rules (each a `VerifyError::Interface { what, reason }` naming the provide or struct):

1. Counts: at most 1024 provides, 256 structs, 1024 events, 64 parameters and 16 results per
   provide, 256 fields per struct (the decoder also never pre-allocates by these counts).
2. Provide names are identifiers and unique. Parameter and result names are identifiers and
   unique within their provide (parameters and results together). `doc` is not blank.
3. The function index refers to a function defined by the module (not an import).
4. The lowered signature (`ApiType::lower` of the parameters, then of the results) equals the
   function's type exactly.
5. Results are scalar (`i32 i64 f32 f64 bool entity`); `str`, `bytes` and `struct` are
   parameters only; `ptr` is neither.
6. Every `struct` parameter names a struct listed in this section.
7. Structs: `StructDesc::validate` holds, names are unique, an own struct (empty owner) has no
   component id, component ids are unique, and owner strings are empty or namespaces
   (`is_namespace`).
8. If any provide has a `str`, `bytes` or `struct` parameter, `scratch` must be present with
   `size >= 1`. A present scratch has `offset % 8 == 0`, `size <= 1_048_576`, lies inside
   `min_pages * 65536`, and overlaps no data segment (empty ranges overlap nothing).
9. Provided events (`what` = ``event `name` ``): names are identifiers, unique among the events
   and different from every provide name (a function and an event never share a full name);
   `doc` is not blank; a payload names a struct listed in this section (own or copied).

Provides are independent of section 7: a provided function need not be a module export, and
provide names do not collide with export names. The calling ABI across mods (copying `str`,
`bytes` and structs through the scratch region) is the embedder's: e8-design section 7.

Helpers (`efevm::api::interface`, pure data): `Interface::host_fns(namespace) -> Vec<HostFn>`
(one description per provide, `owner` empty for the registry to fill),
`Interface::event_descs(namespace) -> Vec<EventDesc>` (one per provided event, `owner` empty)
and `Interface::index(id, version, doc) -> ApiIndex` (the mod's own index: namespace `id` owned
by `id`, the provides as functions owned by `id`, the provided events as events owned by `id`,
own structs owned by `id`, copied structs with their recorded owner, `game` = `id`, sorted),
plus `provide(name)`, `event(name)`, `struct_desc(name)`, `needs_scratch()`, `own_structs()`,
`copied_structs()`, `Provide::{func_type, needs_scratch, host_fn}` and
`ProvidedEvent::event_desc`.

### 9.3 Section 14 `events`

Design: `crates/efescript/docs/e8-design.md` "E8.13 Typed script events"; index side and the
engine ABI: `API_INDEX.md` "Events". The section lists every script event the module sends or
reads. **An event's id inside the module is its index in this list**; the engine's
`engine.events.send(ev, payload)`, `count(ev)` and `read(ev, index, out)` take that id.

```
events   = vec(eventref)
eventref = string namespace, string name, pflag byte; if 1: string payload struct name, uleb size
```

- In memory: `ModuleData.events: Vec<EventRef>` with `EventRef { namespace, name, payload:
  Option<(String, u32)> }` (struct name and size in bytes, as in the index the module was built
  against). `ModuleBuilder::event_ref(namespace, name, payload) -> u32` adds one and returns the
  id (an identical entry is reused). The encoder writes the section only when the list is
  non-empty. Vectors are never pre-allocated from their counts; a bad flag is `BadFlag`.
- Verifier rules (`VerifyError::Events { what, reason }`, `what` = ``event 3 `game.x` `` or
  `events`): at most 1024 entries; namespaces are namespace names (`is_namespace`), event names
  and payload struct names are identifiers; `(namespace, name)` is unique.
- Link time (`Imports::check_module`, so also `Instance::new`): every entry must be a registered
  event (`LinkError::UnknownEvent`) whose payload struct name and size equal the entry's
  (`LinkError::EventPayload`, both payloads as text). The errors come after the import errors
  and before `ApiHash`. The assembler also records each event's namespace in section 12.
- Permissions: the namespaces of the entries (`ModuleData::event_namespaces()`) are checked by
  the embedder against the module's namespace allow-list with the same prefix rule as imports,
  before the module is instantiated.

## 10. Tests required for E1

- One or more tests per instruction, including every trap condition and the boundary
  values (`MIN / -1`, shift counts ≥ width, NaN conversions, memory edge addresses).
- Calls: recursion (factorial/fibonacci), mutual recursion, indirect calls, stack overflow trap.
- Budget: an endless loop returns `OutOfBudget`, and resuming continues correctly.
- Fibers: two fibers interleaved through `yield` sharing memory; host-requested yield.
- Host calls: arguments, results, memory access, host trap.
- Verifier: one rejecting test per rule in section 8.
- Binary: encode → decode round trip for every instruction; a "never panics" test that
  feeds truncated and randomly mutated modules to `Module::decode`.
- A benchmark (not a pass/fail test) of a counting loop, reporting ns per instruction;
  target under 10 ns in release.
- The crate builds for `wasm32-unknown-unknown`.

## Changes from draft 1

Draft 2 records what was built. None of these changes alters an instruction's semantics; they
pin down points draft 1 left open.

1. **Binary payloads (new 9.1).** Draft 1 named the sections but not their byte layout. Section
   9.1 now fixes every payload, the ValType bytes, the LEB width of every field and the
   "over-long LEB" rule. Missing sections mean empty, and the encoder omits empty sections, so
   encoding is deterministic. Register operands are at most 65535 in the file. Table entries are
   `uleb u64` holding index + 1, so every `u32` index can be represented, and a value above
   2^32 is rejected.
2. **Errors.** `LoadError` has four kinds: `Decode { offset, error: DecodeError }`,
   `Verify { func, inst, error: VerifyError }` (one `VerifyError` variant per rule of section
   8), `Link(LinkError)` for import resolution and the memory cap at `Instance::new`, and
   `Build(BuildError)` from `ModuleBuilder`. `Instance::spawn` returns `SpawnError`:
   `NoSuchExport`, `NotAFunction`, `ImportEntry` (an export that is an import cannot be a
   fiber entry), `ArgCount`, `TooManyFibers` and `StackOverflow`.
3. **Verifier rules made explicit.** These are additions to section 8:
   - `nregs >= nparams`;
   - `ret base` needs `base + R <= nregs`;
   - operand fields an instruction does not use must be zero, so the in-memory form is
     canonical;
   - a 32-bit global's init has its upper 32 bits zero;
   - name and line entries refer to existing functions, registers, instructions and files;
   - export names are unique across both kinds;
   - data segments must fit in `min_pages` (checked at verification, so a module that
     verifies always instantiates).
4. **Fiber lifecycle.** Resuming a fiber that is `Done` returns `Exit::Done` again, and a
   trapped fiber returns the same `Exit::Trap` again. An unknown or removed `FiberId` returns
   `Exit::Trap(Trap::Host("invalid fiber"))`. `FiberId` carries a generation, so stale ids
   are detected. `Instance::remove_fiber` frees a slot. Finished fibers count toward
   `max_fibers` until they are removed. `Instance::status` returns `Ready`, `Suspended`,
   `Done` or `Trapped(trap)`. The trap location and backtrace of a trapped fiber are
   available through `Instance::trap_location` / `Instance::backtrace`.
5. **Budget.** Budget 0 executes nothing and returns `OutOfBudget`. An instruction that traps
   still costs 1, and so does a host call. (Superseded by draft 3, item 1: `memory.copy`, `memory.fill` and `memory.grow` now cost in proportion to their size.) `Instance::resume_counted` also returns the number of instructions executed.
6. **Limits.** `max_call_depth` counts live activations including the entry function. A call
   that would exceed it traps with `StackOverflow`; with 0, `spawn` fails. `max_registers` is
   the total register slots of one fiber across all its frames. A register stack that cannot be
   allocated is also a `StackOverflow`. The defaults of draft 2 (16384 pages, depth
   10_000, 2^20 registers, 1024 fibers) are superseded by draft 3, item 2.
7. **Host boundary.** Values crossing into the VM keep section 2's invariant: `i32`/`f32` spawn
   arguments, host results and globals set through `Caller::set_global` /
   `Instance::set_global` have their upper 32 bits cleared according to the declared type. Host
   result slots start at 0, so a result the host does not write is 0. A host function can call
   `Caller::fiber()` to learn which fiber is calling. `Instance::set_global` (embedder) may set
   immutable globals; `Caller::set_global` (host function) may not.
8. **Small operand details.**
   - `call_indirect` and `br_table` read their index register as `u32`.
   - `memory.grow` reads its page count as `u64`.
   - `select`, `br_if` and `br_ifz` test the low 32 bits.

## Changes from draft 2

Draft 3 makes these three changes.

1. **Bulk memory cost and budget debt.** Budget is now in units. Every instruction costs 1 as
   before; in addition
   - `memory.copy` and `memory.fill` cost `1 + len / 256` (integer division of the `u64`
     length register);
   - `memory.grow` costs `1 + (pages * 65536) / 256`, that is `1 + pages * 256`, computed with
     saturating arithmetic. It is charged for the requested size even when the grow fails
     and returns -1.
   - A bulk op that traps (out of bounds) costs only 1 and creates no debt.

   If the cost is more than the budget left, the operation still completes, the remaining
   budget becomes 0, and the excess (cost minus what was left) becomes a per-fiber **debt**.
   The resume returns `Exit::OutOfBudget` right after that instruction, so the fiber is resumable.
   At the start of every later `resume`, before anything executes, the debt is paid:
   if `budget <= debt`, then `debt -= budget` and the resume returns `OutOfBudget` having
   executed nothing; otherwise `budget -= debt`, the debt is 0 and the rest of the budget
   runs normally. Budget 0 therefore never pays debt but never executes either. A cost exactly
   equal to the budget left creates no debt. A fiber with an absurd grow (for example `u64::MAX`
   pages) owes about 2^64 units, so it is effectively dead; the embedder can still remove it.
   `Instance::resume_counted` keeps counting executed **instructions**, not units: the extra
   units of bulk operations and debt payments are not included, so a resume that executes
   2 instructions and pays 10 units of debt reports 2.
2. **Lower default limits and stack release.** `Limits::default()` is now
   `max_memory_pages = 1024` (64 MiB), `max_call_depth = 4096`, `max_registers = 262144` (per
   fiber) and `max_fibers = 256`. When a fiber finishes (`Done`) or traps, its register stack,
   call stack and scratch buffer are freed at once; `results`, `status`, `trap_location` and
   `backtrace` keep working, and resuming the fiber returns the same `Exit` as before.
3. **`Instance::call`.** `call(&mut self, export: &str, args: &[u64], host: &mut H, budget: u64)
   -> Result<Vec<u64>, CallError>` spawns the export, resumes it once with `budget`, returns the
   results on `Done`, and removes the fiber in every case. `CallError` has the variants
   `Spawn(SpawnError)`, `Trap { trap, location: Option<(u32, u32)>, backtrace: Vec<(u32, u32)> }`
   (copied from `trap_location` / `backtrace` before the removal), `OutOfBudget` (including
   debt) and `Yield`. It implements `Debug`, `Clone`, `PartialEq`, `Eq`, `Display` and
   `std::error::Error`.
4. **`Instance::resume_units`.** `resume_units(&mut self, fiber: FiberId, host: &mut H, budget: u64)
   -> (Exit, u64)` is `resume` that also returns the budget units consumed by that call:
   executed instructions, plus the extra units of bulk operations, plus debt paid at entry. It
   never exceeds `budget`; an over-budget bulk op reports the whole remaining budget (the rest
   is debt, paid and counted by later resumes). Budget 0, a `Done`/`Trapped` fiber or an invalid
   id report 0. Summing it over resumes gives the total cost, so an embedder can keep a total
   budget across yield/resume loops.

## Changes from draft 3

Draft 4 adds the host registry and the API index (efescript roadmap E3/E4,
`crates/efescript/docs/e3-design.md`). The binary format version stays `1`: every draft 3 file
is still valid and means the same.

1. **Host functions only through the registry.** `Imports` can no longer be filled directly;
   `HostRegistry::imports()` builds it (a snapshot of `Arc`s) together with the namespace
   versions and the index hash. `Imports::new()` is the empty set. `Imports::names()` lists
   what is callable and `Imports::check_module(&Module)` returns every link problem at once.
2. **Section 12 `api`** (9, 9.1): the index hash and `(namespace, version)` pairs. Modules
   without it link exactly as before.
3. **Linking.** `Instance::new` also checks the namespace versions of section 12.
   `LinkError` gains `UnknownNamespace { namespace }`, `NamespaceVersion { namespace, module,
   host }` and `ApiHash { module, host }` (informational, only listed by `check_module` after
   real errors); `ImportTypeMismatch` now carries both signatures as text (`module`, `host`).
   A different index hash alone is not an error.
4. **Debugger access.** For a `Ready` or `Suspended` fiber, `Instance::fiber_location` (the next
   instruction), `fiber_registers` (the current frame) and `fiber_backtrace` (innermost first,
   callers at their `call` instruction). `resume` with budget 1 executes exactly one instruction
   (if no budget debt is owed).

## Changes from draft 4

Draft 5 implements section 13 `interface` (efescript roadmap E8, task T1). The binary format
version stays `1`.

1. **Section 13** (9, 9.1, 9.2): `ModuleData.interface: Option<Interface>` with `Interface` and
   `Provide` (`efevm::module`), encoded whenever it is `Some`. Only ids 0 and 14–127 are
   rejected now. New `DecodeError::{BadApiType, BadPrim}`.
2. **Verifier** (8, 9.2): rules 1–8 as `VerifyError::Interface { what, reason }`. Rule 5 also
   rejects `ptr`, rule 7 also requires unique component ids (so `Interface::index` always
   validates), and rule 8 treats empty ranges as overlapping nothing.
3. **Builder**: `ModuleBuilder::interface(Interface)`.
4. **API data**: `Interface::host_fns` and `Interface::index` (9.2).

## Changes from draft 5

Draft 6 adds typed script events (efescript roadmap E8.13). The binary format version stays
`1`; section 13 changed incompatibly and no compatibility is kept (the project has no released
users).

1. **Section 14 `events`** (9, 9.1, 9.3): `ModuleData.events: Vec<EventRef>`, the event id is the
   index. Only ids 0 and 15–127 are rejected now. `ModuleData::event_namespaces()`.
2. **Section 13** gains `vec(pevent)` after the provides: `Interface.events: Vec<ProvidedEvent>`,
   verifier rule 9 and an events count in rule 1; `Interface::event_descs` and `Interface::index`
   include them.
3. **Verifier**: `VerifyError::Events { what, reason }` for section 14.
4. **Linking**: `LinkError::{UnknownEvent { namespace, name }, EventPayload { namespace, name,
   module, host }}`; `Imports` carries the registered events (`Imports::event`, `events`).
5. **Builder**: `ModuleBuilder::event_ref(namespace, name, payload) -> u32`.
6. **API**: `EventDesc`, `HostRegistry::{register_event, replace_event, event, events}`, new
   `RegistryError` variants, `ApiIndex.events` and `.efapi` format version 2 (API_INDEX.md).
