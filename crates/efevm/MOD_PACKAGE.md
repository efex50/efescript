# Mod packages (`.efmod`), format version 1

Status: implemented (E8), 2026-11-16 (container changed from ZIP to 7z).
How the engine loads, reloads and caches mods: [`docs/MOD_LOADING.md`](../../docs/MOD_LOADING.md).
Design and rationale: `crates/efescript/docs/e8-design.md`. Roadmap: E8.6–E8.11 in
`crates/efescript/ROADMAP.md`.

A mod package is one file that carries one mod: its manifest, its single compiled module and
its assets. Any game built with efeengine can load it from bytes (`Engine::load_package`), so
the web build works the same as native. The code that reads and writes packages lives in the
crate **`efemod`** (`crates/efemod`), which depends on `efevm`, `sevenz-rust2` (reader only:
`default-features = false`, so no AES, bzip2, PPMd or compressor) and `crc32fast`, does no I/O
and builds for `wasm32-unknown-unknown`. The writer is behind the feature `write` (it adds
`lzma-rust2`'s LZMA2 encoder); the packer enables it, the engine does not. The engine depends on
`efemod`, never on the efescript CLI crate.

This file is normative for: the container (1), the entries (2), the manifest in the package
(3), the authoring manifest `mod.toml` (4), versions and ranges (5), limits (6) and what a
reader must check (7). The module section the package's module carries (`interface`, section
13) is specified in `SPEC.md` 9.2.

Words: *reader* = `efemod::Package::read`; *writer* = `efemod::Package::write` (used by
`efescript pack`); *loader* = the engine's `load_package`.

## 1. Container: a restricted 7z archive

A `.efmod` file is a 7z archive (7-Zip `7zFormat.txt`, version 0.4) restricted to the profile
below, so `7z l mod.efmod` lists it and any 7z tool opens it. The profile is what the canonical
writer produces; a reader rejects anything outside it with `PackageError::Container(..)` (or
`TooLarge`, `BadEntryName`, `Crc` where those fit). It never panics, and it checks the structure
itself before the 7z library (`sevenz-rust2`) parses or decodes anything.

| Feature | Rule |
|---|---|
| Signature header | signature `7z BC AF 27 1C`, major version 0; the start-header CRC must match (a zeroed or damaged start header is rejected, never searched for) |
| Header | plain (`kHeader`): an encoded header (`kEncodedHeader`, compressed or encrypted) is rejected; at most `header_size` bytes (6); its CRC must match; it ends exactly at the end of the file (no trailing data) |
| Header content | no archive properties, no additional streams, no external data; every count (pack streams, folders, files) is checked against the entry limit while the header is walked, before anything is sized by it |
| Packed streams | start right after the signature header (pack position 0) and fill the space up to the header exactly; one packed stream per folder |
| Folders | one coder per folder, a simple coder (1 in, 1 out), no alternative methods; coder COPY (`00`, no properties) or LZMA2 (`21`, one property byte, dictionary at most `dictionary_size`); every other coder or filter (LZMA, PPMd, BZip2, Deflate, BCJ, BCJ2, Delta, AES, ...) is rejected |
| Non-solid | each folder holds exactly one entry (one substream; no substream sizes); an entry with data has a folder of its own, and no folder is left without an entry |
| Data | every entry with data has a CRC-32. Decoding never produces more than the declared size plus one byte; the output must equal the declared size exactly and its CRC must match |
| Empty files | an entry without data that is not a directory is an empty file (allowed) |
| Directory entries | ignored (not returned), but their names follow section 2 and take part in the duplicate check |
| Anti-items | rejected |
| Times, attributes | ignored |

A 7z archive made with the 7-Zip command line reads when it is non-solid with a plain header
and LZMA2 or COPY: `7z a -t7z -ms=off -mhc=off -m0=lzma2 mod.efmod manifest.json module.efvm
assets`. The defaults of `7z a` (solid, compressed header) are rejected.

The **writer** (`container::write`, `Package::write`, feature `write`) produces a canonical
archive, so the same inputs give the same bytes: entries in the order `manifest.json`,
`module.efvm`, then assets sorted by name (byte order); one folder per non-empty entry with
LZMA2 (`lzma-rust2`, preset 6, single-threaded, dictionary = the smallest LZMA2 size that holds
the entry, at most 8 MiB) unless the compressed form is not smaller, then COPY; CRCs in the
substreams info; empty entries as empty files without a stream; a plain header with sizes,
CRCs and names only (no times, no attributes, no directory entries, no archive properties);
version 0.4.

The file extension is `.efmod`. There is no signature; a hash of the bytes is change
detection, not trust (signing is listed under "Later" in the roadmap).

## 2. Entries

| Name | Required | Content |
|---|---|---|
| `manifest.json` | yes | the manifest (3) |
| `module.efvm` | yes | the mod's one module (`SPEC.md`), usually with sections 12 `api` and 13 `interface` |
| `assets/<path>` | no | any bytes; available to the mod and the game (e8-design 9.4) |
| anything else | no | ignored by the reader and listed in `Package::ignored()`; `efescript inspect` prints a warning (this tolerates `__MACOSX/` and similar from hand-made archives) |

A package has exactly one module: one mod is one instance, one namespace, one budget. A mod
written as many files is linked into one module at build time (e8-design 4).

The mod's own API index is **not stored**: it is a pure function of the manifest and section
13 (`efevm::Interface::index`), so it cannot disagree with the module. `efescript api-export
mod.efmod` writes it, and `--api mod.efmod` reads it directly from the package.

### Entry names

An entry name must:
- be valid UTF-8, 1–240 bytes, with `/` as the only separator;
- not start with `/`, and contain no `\`, no `:`, no NUL or other control character (U+0000–U+001F, U+007F);
- have 1–16 segments, none empty, `.` or `..`;
- be unique ignoring ASCII case (`Assets/A.png` and `assets/a.png` together are rejected, since
  they would collide when extracted on Windows or macOS).

The reader rejects a package with a bad name (`PackageError::BadEntryName { name, reason }`),
even for an entry it would otherwise ignore. Asset paths given to the mod or game are the
name without the `assets/` prefix.

## 3. `manifest.json`

UTF-8 JSON, one object, parsed with efevm's JSON reader (`efevm::api::json`, depth limit 64,
duplicate keys rejected). Readers ignore unknown keys but reject an unknown `format` or a
`format_version` other than 1. The writer emits the keys in the order below, two-space
indentation, `\n` line ends, a final newline; optional keys that are absent or empty are
omitted.

```json
{
  "format": "efmod",
  "format_version": 1,
  "id": "inventory",
  "version": "1.2.0",
  "name": "Inventory",
  "description": "Items, bags and a shop.",
  "authors": ["Efe"],
  "license": "MIT",
  "game": "script_demo",
  "api_version": 1,
  "dependencies": [
    { "id": "core_items", "req": "^1.2" }
  ],
  "permissions": {
    "namespaces": ["engine.log", "engine.world"],
    "budget": 2000000,
    "memory_pages": 64
  }
}
```

| Key | Type | Required | Rule |
|---|---|---|---|
| `format` | string | yes | `"efmod"` |
| `format_version` | integer | yes | `1` |
| `id` | string | yes | mod id: `[a-z][a-z0-9_]{0,63}`; not `engine`, `std` or `efe`. It is the module name, the namespace of everything the mod provides, and the owner in the index. The loader also refuses an id that equals or is the first segment of a namespace already registered in the running game (`game`, `engine.log`, another mod's) with `ModError::IdClash`, before the game's policy is asked |
| `version` | string | yes | a version (5.1) |
| `name` | string | no | display name, 1–128 chars, no control characters; default: `id` |
| `description` | string | no | at most 2048 chars; also the doc of the mod's namespace in the index (default doc: ``"Functions provided by the mod `id`."``) |
| `authors` | array of strings | no | at most 32 entries, each 1–128 chars |
| `license` | string | no | at most 64 chars (an SPDX expression is recommended, not checked) |
| `game` | string | no | 1–64 chars. When present, the loader refuses the package unless it equals the running game's name (`Engine::set_game_name` / config title), so a mod made for one game is not loaded by another |
| `api_version` | integer | no | `1 ..= 2^32-1`, default `1`: the version of the namespace `id`. Bump it on an incompatible change to any provided function or struct (API_INDEX.md "Versioning") |
| `dependencies` | array | no | at most 64 entries `{ "id": <mod id>, "req": <range> }`; ids unique and not the mod's own; `req` a range (5.2) |
| `permissions.namespaces` | array of strings | no | at most 64 namespace prefixes (`is_namespace`), unique. These are **requests**: the game's policy decides what is granted |
| `permissions.budget` | integer | no | requested VM units per frame, `1 ..= 10^12`; default: the engine default (1 000 000) |
| `permissions.memory_pages` | integer | no | requested maximum memory in 64 KiB pages, `1 ..= 65536`; default: the engine default (1024) |
| `meta` | object | no | free-form data for the game (tags, display data, limits). The engine stores it and never reads it. Must be a JSON object; at most 64 KiB of canonical JSON (no spaces, keys in the order written); nesting depth at most 16 (the object itself is depth 1). Written last |

Rules checked against the module by the reader (`PackageError::Manifest`):
- every namespace the module imports must match a requested prefix (the prefix rule of
  `ModOptions::allowed`: equal, or continuing with a dot) **or** be a dependency's namespace
  (a dependency id `d` covers `d` and `d.*`). An import that is neither is
  `PackageError::UndeclaredImport { namespaces }`. A mod cannot call another mod without
  declaring it as a dependency, which is what makes load order correct;
- the module may not import from its own namespace `id`.

There is no "built against" key: section 12 of the module already records the index hash and
namespace versions, and the loader reports incompatibilities from there by name.

## 4. `mod.toml` (the authoring manifest)

A mod's source folder:

```
inventory/
  mod.toml
  src/main.efa          ; the entry file; `use` pulls in the rest
  src/items.efa
  assets/items.json
```

`mod.toml` is TOML 1.0 and is read only by the build tools (`efescript pack`, `cargo xtask`),
which may use the `toml` crate; the engine never parses TOML. `efescript pack` turns it into
`manifest.json`. Unknown keys and tables are errors in `mod.toml` (typos are caught at build
time), unlike `manifest.json`.

```toml
[mod]
id = "inventory"
version = "1.2.0"
name = "Inventory"
description = "Items, bags and a shop."
authors = ["Efe"]
license = "MIT"
game = "script_demo"        # optional
api_version = 1             # optional, default 1

[dependencies]
core_items = "^1.2"         # id = range

[permissions]
namespaces = ["engine.log", "engine.world"]
budget = 2_000_000          # optional
memory_pages = 64           # optional

[build]                     # optional, not copied into the package
entry = "src/main.efa"      # default "src/main.efa"
api = ["game.efapi"]        # default --api inputs (`.efapi` or `.efmod`), relative to this file
assets = "assets"           # default "assets"; every file below it becomes `assets/<path>`

[meta]                      # optional, free-form; becomes the manifest's `meta` object
tags = ["items", "shop"]
max_stack = 99
```

`[mod]` maps to the top-level keys of `manifest.json`, `[dependencies]` to the
`dependencies` array (sorted by id), `[permissions]` to `permissions`, `[meta]` to `meta`; the
validation rules are those of section 3. `[meta]` values convert as follows: string → string,
integer and finite float → number, bool → bool, datetime → its TOML text as a string, array →
array, table → object (keys in TOML order). NaN and infinities are refused.

`[build]` paths (`entry`, `assets`, `api`) must stay inside the mod folder: an absolute path or
a `..` component is an error. `efescript pack` reads the `api` files and merges them with
`--api`. The engine ignores `[build] api` at runtime (it checks the paths, then compiles the
folder against the index the game supplies), so a mod folder loaded by the engine needs no
index file. The engine builds folder mods with these limits: 32 MiB of files read in total,
1024 files, `use` depth 64, nesting 128.

Asset files are taken in sorted order; names must satisfy the entry-name rules (2); symbolic
links are not followed. Asset names (relative to `assets/`) may not be `manifest.json`,
`module.efvm`, `engine` or anything under `engine/`; the build refuses them.

## 5. Versions and ranges

### 5.1 Version

`MAJOR.MINOR.PATCH` or `MAJOR.MINOR.PATCH-PRE` (Semantic Versioning 2.0.0 without build
metadata: a `+` is rejected).
- Each number is a decimal `u32` without leading zeros (`0` itself is fine).
- `PRE` is one or more dot-separated identifiers of `[0-9A-Za-z-]`, none empty; a purely
  numeric identifier has no leading zeros.
- At most 64 bytes in total.
- Order is SemVer precedence: numbers compare numerically; a pre-release is lower than the
  same version without one; pre-release identifiers compare left to right, numeric ones
  numerically and below alphanumeric ones, alphanumeric ones by ASCII; a shorter list is lower
  when all earlier identifiers are equal.

### 5.2 Range (`req`)

```
range      = "*" | comparator { "," comparator }      (at most 4 comparators)
comparator = [ op ] partial
op         = "=" | ">" | ">=" | "<" | "<=" | "^" | "~"
partial    = MAJOR [ "." MINOR [ "." PATCH [ "-" PRE ] ] ]
```
Spaces around tokens are ignored. A comparator without `op` means `^`. A version satisfies a
range when it satisfies every comparator. Meaning (the same as Cargo's):

| Comparator | Matches |
|---|---|
| `=1.2.3` / `=1.2` / `=1` | exactly 1.2.3 / `>=1.2.0, <1.3.0` / `>=1.0.0, <2.0.0` |
| `>1.2.3` / `>1.2` / `>1` | `>1.2.3` / `>=1.3.0` / `>=2.0.0` |
| `>=1.2.3` / `>=1.2` | `>=1.2.3` / `>=1.2.0` |
| `<1.2.3` / `<1.2` | `<1.2.3` / `<1.2.0` |
| `<=1.2.3` / `<=1.2` | `<=1.2.3` / `<1.3.0` |
| `^1.2.3` / `^1.2` / `^1` | `>=1.2.3, <2.0.0` / `>=1.2.0, <2.0.0` / `>=1.0.0, <2.0.0` |
| `^0.2.3` / `^0.2` | `>=0.2.3, <0.3.0` / `>=0.2.0, <0.3.0` |
| `^0.0.3` / `^0.0` / `^0` | `>=0.0.3, <0.0.4` / `>=0.0.0, <0.1.0` / `>=0.0.0, <1.0.0` |
| `~1.2.3` / `~1.2` / `~1` | `>=1.2.3, <1.3.0` / `>=1.2.0, <1.3.0` / `>=1.0.0, <2.0.0` |
| `*` | every version without a pre-release |

Pre-releases: a version with a pre-release satisfies a range only if at least one comparator
names the same `MAJOR.MINOR.PATCH` with a pre-release (so `^1.2.0-beta.1` matches
`1.2.0-beta.2` but `^1.0` never matches `1.3.0-rc.1`). As in Cargo, a partial comparator
(`=1.2`, `>=1.2`, `<=1.2`) never names a pre-release, so `>=1.2.5-rc.1, <=1.2` does not match
`1.2.5-rc.2`. Wildcards such as `1.*` and `1.x` are not supported; write `^1` or `=1`.

The package only **requests** permissions; what is granted is the game's `ModPolicy`'s
decision, and the default policy grants nothing. A grant applies to the whole package (its one
module, linked from all its source files); there is no per-file or per-entry permission.

## 6. Limits

`efemod::ReadLimits::default()`; an embedder may lower them (a web game may want smaller
packages) but not raise them above these hard caps.

| Limit | Default (= hard cap) |
|---|---|
| package file size | 64 MiB |
| 7z header (`ReadLimits::header_size`) | 4 MiB |
| LZMA2 dictionary a coder declares (`ReadLimits::dictionary_size`) | 64 MiB |
| entries (including ignored and directory entries) | 4096 |
| entry name | 240 bytes, 16 segments |
| `manifest.json` uncompressed | 64 KiB |
| `module.efvm` uncompressed | 16 MiB |
| one asset uncompressed | 32 MiB |
| all entries uncompressed together | 128 MiB |

Sizes are checked against the header before anything is decoded, and again while decoding
(each entry is read through a reader bounded to its declared size plus one byte, so a lying
header cannot make the reader allocate more). Counts are checked before anything is sized by
them. The dictionary limit is checked on the coder properties before a decoder exists; the
decoder's window also grows only with the decoded output.

## 7. What a reader checks, in order

1. File size limit; signature header and its CRC; header size limit, header position (ends
   at the end of the file) and CRC; plain header.
2. The header walk: every count against the entry limit, coders (COPY or LZMA2), the LZMA2
   dictionary limit, one entry per folder (1).
3. The 7z library parses the header; on its result: non-solid, packed streams fill the space
   before the header, one coder per folder (again), entry count.
4. For every entry: name rules and duplicates (2), no anti-item, a folder of its own and a
   CRC when it has data, size limits and the total (6).
5. Decode and CRC-check every entry (bounded, 6).
6. Required entries present (`manifest.json`, `module.efvm`).
7. Parse and validate `manifest.json` (3).
8. `efevm::Module::decode` of `module.efvm` (decode and verify, which includes section 13's
   rules).
9. Manifest-against-module rules (3).

The result is `efemod::Package { manifest: Manifest, module: efevm::Module, module_bytes,
assets: BTreeMap<String, Vec<u8>>, ignored: Vec<String> }`. Errors are
`efemod::PackageError` with variants `TooLarge { what, size, limit }`, `Container(String)`,
`BadEntryName { name, reason }`, `Missing(&'static str)`, `Crc { name }`, `Manifest(String)`
(with the key path, for example ``dependencies[2].req: `^1.x` is not a range``),
`Module(efevm::LoadError)` and `UndeclaredImport { namespaces }`. Linking against the
game's registry, granting permissions and resolving dependencies are the loader's job, not
the reader's (e8-design 9).

## 8. Tests the implementation needs

- Round trip: `write` then `read` gives the same manifest, module and assets; writing twice
  gives identical bytes; the plain `sevenz-rust2` reader and `7z t` accept the writer's output.
- A 7z command-line archive in the profile (`-ms=off -mhc=off`, LZMA2 and COPY, with directory
  entries, times and attributes) reads; the `7z a` defaults, a compressed header, solid, PPMd,
  BZip2, Deflate, LZMA, Delta, BCJ and AES (with and without an encrypted header) are rejected
  (skipped when no `7z` is installed).
- One rejection test per rule of sections 1, 2, 3, 5 and 6 (zeroed start header, header CRC,
  trailing data, encoded header, archive properties, other coder ids, two coders, LZMA2
  dictionary properties 28 accepted / 29 and 40 too large / 41 invalid, CRC mismatch, declared
  size lies, packed streams with a gap, anti-item, `../x`, `C:x`, unpaired UTF-16 surrogate,
  case-duplicate names, huge declared counts, missing manifest, bad id, bad range, undeclared
  import, each limit).
- Version order and every row of the range table, including pre-releases.
- A never-panics test like `crates/efescript/tests/v2_never_panics.rs`: truncation at every
  length, random bytes, and mutated packages (anywhere; in the header with the CRCs fixed up so
  the input reaches the header walk and the 7z library; in the packed data so it reaches the
  decoders), plus mutated manifests inside valid archives, only ever return errors. The test
  counts how many inputs get past the pre-checks and how many reach CRC verification.
