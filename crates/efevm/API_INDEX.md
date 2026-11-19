# The API index file (`.efapi`), format version 2

The API index lists everything a script can call or see: host-function namespaces,
host functions with parameter names, types and docs, the byte layouts of structs and
engine components, and the script events with their payload structs. It is the "PDB" of a game built with efeengine: a mod developer, the
assembler, the language server and the debugger need this file and nothing else.

The index is not a second table. It is a snapshot of `efevm::HostRegistry`, the only way
to make a host function callable or a script event usable (`HostRegistry::imports()` is the
only producer of the `Imports` an `Instance` links against, and it carries both). Whatever is
callable or sendable is therefore in the index, and what is in the index is exactly what was
registered.

Producers: `HostRegistry::index()` → `ApiIndex`, `ApiIndex::to_json()` → text.
Consumers: `ApiIndex::from_json(&str)`. Both live in `efevm::api` and do no I/O; callers
read and write the file.

## File

- Name: `<game>.efapi` (the std library of the efescript CLI: `std.efapi`).
- Encoding: UTF-8 JSON, one object. The writer emits keys in the order shown below, two-space
  indentation, `\n` line ends, and a final newline, so the same registry always produces the
  same bytes.
- Readers must ignore unknown keys (forward compatibility) but must reject an unknown
  `format` or a `version` greater than they support.

```json
{
  "format": "efapi",
  "version": 2,
  "game": "script_demo",
  "hash": "fnv1a64:9f0c2b5e17a4d3c8",
  "namespaces": [
    { "name": "engine.world", "version": 1, "owner": "engine", "doc": "Entities and components." }
  ],
  "types": [
    {
      "name": "Transform", "owner": "engine", "doc": "Position, rotation and scale.",
      "size": 20, "align": 4, "component": 2166136261,
      "fields": [
        { "name": "position", "type": "f32", "count": 2, "offset": 0, "doc": "" },
        { "name": "rotation", "type": "f32", "count": 1, "offset": 8, "doc": "Radians." },
        { "name": "scale", "type": "f32", "count": 2, "offset": 12, "doc": "" }
      ]
    }
  ],
  "functions": [
    {
      "namespace": "engine.world", "name": "get", "owner": "engine",
      "doc": "Copy component `component` of `entity` to `out`; 0 ok, 1 stale entity, 2 no such component.",
      "params": [
        { "name": "entity", "type": "entity" },
        { "name": "component", "type": "i32" },
        { "name": "out", "type": "ptr" }
      ],
      "results": [ { "name": "status", "type": "i32" } ],
      "signature": "(i64, i32, i64) -> i32"
    }
  ],
  "events": [
    { "namespace": "game", "name": "score_changed", "owner": "game", "payload": "Score", "doc": "A player's score changed." },
    { "namespace": "game", "name": "round_reset", "owner": "game", "payload": null, "doc": "The round was reset." }
  ]
}
```

### Keys

| Key | Meaning |
|---|---|
| `format` | always `"efapi"` |
| `version` | format version, `2` (readers accept only `2`) |
| `game` | name of the game (or `"std"`); informational, not hashed |
| `hash` | `"fnv1a64:"` + 16 lower-case hex digits, see below |
| `namespaces` | sorted by `name` |
| `types` | sorted by `name`; `component` is a number for engine components, `null` for plain structs |
| `functions` | sorted by `(namespace, name)` |
| `events` | sorted by `(namespace, name)`; `payload` is a struct name listed in `types`, or `null` for an event without payload |
| `signature` | the lowered VM signature, `(params) -> result` or `(params) -> (r1, r2)` or `(params)`; derived from `params`/`results`, written for humans and checked by the reader |

Names: a namespace is identifiers joined by dots (`std`, `engine.world`, `inventory`); a
function, event, struct or field name is an identifier `[A-Za-z_][A-Za-z0-9_]*`. A script imports
`namespace.name`; the last dot separates the function name. A function and an event never
share a full name `namespace.name`.

### Parameter types

| Spelling | VM slots | Meaning |
|---|---|---|
| `i32` `i64` `f32` `f64` | one | plain values |
| `bool` | `i32` | 0 = false |
| `entity` | `i64` | `Entity::to_bits`; a stale or forged handle fails with a status code, never a panic |
| `ptr` | `i64` | untyped guest address |
| `str` | `i64, i64` | pointer and length of UTF-8 text in guest memory (parameters only) |
| `bytes` | `i64, i64` | pointer and length of a byte buffer in guest memory (parameters only) |
| `&Name` | `i64` | pointer to a struct with the registered layout `Name` (parameters only) |

Results may only be `i32 i64 f32 f64 bool entity ptr`.

### Field types

`u8 i8 u16 i16 u32 i32 u64 i64 f32 f64 bool entity`, each aligned to its own size;
`count` > 1 means a fixed array (`Vec2` = `f32` × 2). Fields never overlap and lie within
`size`; `align` is a power of two dividing `size`. Padding is not listed.

## Hash

`hash` is 64-bit FNV-1a over the bytes of the *canonical body*: the same JSON text the
writer produces, with the `game` and `hash` lines left out (so the hash depends only on the
API, not on the game's name). Readers recompute it and reject a file whose hash does not
match (it was edited by hand or truncated). FNV-1a is a change detector, not a security
measure.

## Events

A script event is registered with `HostRegistry::register_event(EventDesc { namespace, name,
payload, doc, owner })` (owner filled from the namespace, as for functions). Rules: the
namespace is defined, the name is an identifier, the doc is not blank, the payload (if any) is
a registered struct, the full name is not registered yet and is not the full name of a
function (`RegistryError::{UnknownEventNamespace, InvalidEventName, EmptyEventDoc,
UnknownEventPayload, DuplicateEvent, EventFunctionClash}`; `replace_event` for hot reload,
`NoSuchEvent`). `unregister_owner` removes the owner's events and refuses
(`StructInUseByEvent`) while an event of another owner carries one of its structs. A mod
declares the events of its own namespace in section 13 (SPEC.md 9.2); the engine registers
them from `Interface::event_descs(id)` when the mod loads.

`ApiIndex::validate` checks every event: listed namespace, identifier name, unique, not a
function's full name, payload struct listed. `ApiIndex::merge` keys events by `(namespace,
name)`: identical entries are kept once, different content under one name is an error naming
both games (``event `game.reset` differs between `a` and `b` ``).

### How modules use events (`.efvm` section 14 "events")

A module lists every event it sends or reads in section 14 (SPEC.md 9.3): `(namespace, name,
payload)` with the payload as `(struct name, size)` taken from the index it was built against.
**An event's id inside the module is its index in that list**; ids are per module, so they
never depend on registration order in the engine. The assembler (later task) writes
`import event game.score_changed` and takes the payload struct and its size from `--api`; it
also records the event's namespace in section 12 with its version.

At link time (`Imports::check_module`, `Instance::new`) every reference must be registered
(`LinkError::UnknownEvent { namespace, name }`) with the same payload struct name and size
(`LinkError::EventPayload { namespace, name, module, host }`, both sides spelled `&Score (8
bytes)` or `no payload`). These are collected with the other link errors, after the imports
and before `ApiHash`. Permissions: before it instantiates a mod, the engine checks every
namespace of `ModuleData::event_namespaces()` against the mod's allow-list with the same prefix
rule as imports (the grant covers the whole package, e8-design section 13 item 2).

### Engine ABI (`engine.events`, implemented by the engine)

| Function | Signature | Meaning |
|---|---|---|
| `send` | `(ev: i32, payload: ptr)` | Queue event `ev`; copies `size` bytes of the registered payload layout from `payload` (an event without payload ignores `payload`; pass 0). Readable during the next frame. |
| `count` | `(ev: i32) -> (count: i32)` | Number of events `ev` readable this frame. |
| `read` | `(ev: i32, index: i32, out: ptr) -> (status: i32)` | Copy the payload of the `index`-th (0-based) event `ev` of this frame to `out` (`size` bytes; nothing for no payload); returns 0, or -1 if there is no such event. |

- `ev` is an id of the calling module's section 14; an id outside it traps (`Trap::Host`
  naming the id). Out-of-bounds `payload`/`out` ranges trap with `OutOfBounds`.
- Payloads are copied by the registered layout's `size`, never by a length the script passes;
  the link check guarantees module and host agree on it.
- At most 4096 events are queued per frame (all events together). Further sends are dropped;
  the engine counts the drops and logs the count once per frame in which any were dropped.
- Rust code sees the same events (typed by name and payload struct) through the engine API.

## How modules use the hash (`.efvm` section 12 "api")

A module assembled against an index records it in section 12 of the `.efvm` format
(SPEC.md section 9.1): the index hash (`u64`, 8 bytes little-endian) and, for every
namespace it imports from, `(name, version)`. At link time:

- every import is resolved by `namespace.name` and checked against the lowered signature
  (`missing import`, `signature mismatch` errors name the function and both signatures);
- every recorded namespace must exist with the same version (`namespace engine.world is
  version 2, module was built against version 1`);
- a different index hash alone is not an error: if all imports and versions match, the
  module links (functions were added or docs changed). `Imports::check_module` lists every
  incompatibility at once, by name, and says which hash the module was built against.

## A mod's own index (`.efvm` section 13 "interface")

A module that provides functions as a mod carries section 13 (SPEC.md 9.2): its provides
(name, function, doc, named and typed parameters and results) and the struct layouts they
use. `Interface::index(id, version, doc)` derives the mod's `.efapi` from it: one namespace
`id` (that version, owner `id`, that doc), every provide as a function of `id` owned by `id`,
own structs (empty owner in the section) owned by `id`, copied structs with the owner recorded
in the section, every provided event as an event of `id` owned by `id`, and `game` = `id`. It
is sorted like a registry index and is exactly what a `HostRegistry` holds after the embedder
registers the mod with `Interface::host_fns(id)` and `Interface::event_descs(id)`.


Version 2 (E8.13) added the required `events` key; readers accept only version 2, since no
compatibility with older files is kept. Additive changes (new optional keys) keep the version;
a change that a reader would misread bumps it. Namespace versions are owned by the namespace owner and are bumped on an
incompatible change to any function in the namespace.

## Reader and registry details (recorded during implementation)

- The reader accepts a file if its `hash` matches either the definition above (the file text
  with the top-level `game` and `hash` lines removed) or the hash of its canonical
  re-serialisation. The first covers keys a newer version-2 writer may add; the second lets a
  reformatted file with the same content load. It rejects duplicate keys and any `version`
  other than 2, and sorts what it reads instead of requiring sorted input.
- `HostRegistry::unregister_owner` returns `Result<usize, RegistryError>`: it refuses
  (`StructInUse`) and removes nothing when another owner still uses one of the owner's structs.
- `Imports::check_module` appends `LinkError::ApiHash { module, host }` after real problems,
  to say which index the module was built against; `Instance::new` never fails with it.
