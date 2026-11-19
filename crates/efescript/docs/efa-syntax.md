# `.efa` syntax and assembler reference (E2)

Authoritative reference for the efescript v2 assembler: `.efa` text to an `efevm` module.
It refines `e2-design.md` and folds in the manager's decisions; where this document and the
design differ, this document wins. Section 14 lists every decision that goes beyond the
design and the decisions file, for review.

Code that implements this document: `src/asm/{ast,diag,lexer,parser,source}.rs`,
`src/asm/lower/` (one file as a `link::Unit`), `src/link/` (the linker), the disassembler
(`src/disasm.rs`) and the CLI.

Contents: 1 lexical rules, 2 grammar, 3 top-level items, 4 functions and registers,
5 operand forms per instruction shape, 6 immediates and their types, 7 lowering,
8 data layout, 9 names and lines, 10 extras, 11 round trip, 12 disassembler output,
13 diagnostics, 14 decisions beyond the design, 15 complete example, 16 multi-file programs
and the mod interface (E8), 17 events, 18 constant expressions.

## 1. Lexical rules

Source is UTF-8 text. Line terminator `\n` (a `\r` before it is ignored; a lone `\r` is
whitespace). Columns count chars (a tab is one column), lines and columns are 1-based.

- **Whitespace**: space, tab, `\r`. Insignificant except as a separator.
- **Comment**: `;` to the end of the line (not inside a string or character literal).
- **Newline** is a token: a statement ends at the end of the line. The lexer emits no Newline at
  the start of the file, collapses a run of newlines (blank and comment-only lines included)
  into one token, **does not emit a Newline that directly follows a `,`** (a list continues on
  the next line, blank and comment lines in between are skipped), and always ends the stream
  with `Newline, Eof` (just `Eof` for an empty file).
- **Names**: `[A-Za-z_][A-Za-z0-9_]*` segments joined by single `.` where the segment after the
  dot starts with a letter or `_`. A dotted name is ONE token (`i64.add`, `const.i32`,
  `std.println`, `msg.len`, `i64.trunc_f64_s`). `a.` and `r0.1` are not names (the stray `.`
  is an error). Only ASCII letters are allowed in names.
- **Integers**: decimal, `0x` hex, `0b` binary, `0o` octal (lowercase prefix), `_` separators,
  optional leading `-` that must touch the first digit. Value is an `i128` (error beyond
  `i128`). `007` is 7. A number immediately followed by a letter/digit/`_` run (`12abc`) is
  one invalid literal.
- **Floats**: `digits.digits[exp]` or `digits exp` (`1e9`, `1.5e-3`, `2E+4`), `_` allowed in
  the digit runs, optional leading `-`; the words `inf`, `nan` (also `-inf`, `-nan`);
  and the exact-bits form `nan:0xHEX` (1 to 16 hex digits; the sign lives in the bits, so
  `-nan:0x..` is an error). `1.` and `.5` are not floats. The token carries the SOURCE TEXT
  with underscores removed and the sign included, e.g. `1_0.5` becomes `10.5`. `inf` and `nan`
  are literals only as whole names (`info`, `inf.x` are names, and so `inf` can never be a
  register or label name).
- **Characters**: `'a'`, `'\n'`; escapes `\n \t \r \0 \\ \" \' \xNN`. Value is one Unicode scalar
  (`\xNN` gives U+00NN). It is an integer everywhere an integer is accepted.
- **Strings**: `"..."` with escapes `\n \t \r \0 \\ \" \xNN` (`\'` is NOT valid in strings),
  unescaped to bytes (non-ASCII chars give their UTF-8 bytes). A string cannot span lines.
- **Punctuation tokens**: `{ } ( ) [ ] , : = ->`. `->` is one token and wins over `-`.
- **Operator tokens** (constant expressions, section 18): `+ - * / % & | ^ ~ << >>`. A `-` that
  touches a digit, `inf` or `nan` is part of the literal (`-5`, `-inf`); any other `-` is the
  token `Minus`. In a constant expression the parser splits a negative literal that follows an
  operand into `-` and the number (`4-1` is `4 - 1`). A single `<` or `>` is an error.
  `i64.sub n, n, 1` is still just the mnemonic `i64.sub` and three operands.
- **Errors recover**: the bad character or literal is skipped and lexing continues.
- **Reserved words** (cannot be declared as names of anything): `import memory const global
  table fn export mut reg nregs null db dw dd dq resb i32 i64 f32 f64 inf nan use as pub
  provide struct scratch` (the last six since E8, section 16.9).
  `r<digits>` (`r0`, `r17`) is always a raw register and cannot be declared either (the
  exception is a parameter `rK: T` at position K, see 4).

Token kinds (`asm::lexer::TokenKind`): `Name(String)`, `Int(i128)`, `Float(String)`,
`Char(char)`, `Str(Vec<u8>)`, `LBrace RBrace LParen RParen LBracket RBracket Comma Colon
Equals Arrow Amp Plus Minus Star Slash Percent Pipe Caret Tilde Shl Shr Newline Eof` (`Amp` is
`&`: the reference sigil of 16.9 and bitwise and in section 18).

## 2. Grammar

EBNF-ish. `NL` = Newline token, `NAME` a (dotted) name token, `QNAME` a dotted name with at
least one dot. Lists of items separated by `,` may continue on the next line after a comma.

```
module     = { NL | item } EOF
item       = import | memory | const | global | table | data | func

import     = "import" QNAME "(" [ type { "," type } ] ")" [ "->" results ] NL
memory     = "memory" cexpr [ cexpr ] NL
const      = [ "pub" ] "const" NAME "=" cexpr NL
global     = [ "export" ] "global" [ "mut" ] NAME ":" type "=" cexpr NL
table      = "table" "[" [ tentry { "," tentry } ] "]" NL
tentry     = NAME | "null"
data       = NAME ( "db" | "dw" | "dd" | "dq" ) dvalue { "," dvalue } NL
           | NAME "resb" cexpr NL
dvalue     = cexpr | STRING
func       = [ "export" ] "fn" NAME "(" [ param { "," param } ] ")" [ "->" results ] "{" NL
             { stmt } "}" NL
param      = NAME ":" type
results    = type | "(" type { "," type } ")"          (* "->" "()" is not allowed *)
type       = "i32" | "i64" | "f32" | "f64"

stmt       = { label } [ regdecl | nregs | inst ] NL      (* a statement may be only labels *)
label      = NAME ":"
regdecl    = "reg" NAME { "," NAME }
nregs      = "nregs" operand
inst       = MNEMONIC [ operand { "," operand } ]          (* MNEMONIC is a NAME *)

operand    = NAME                                         (* register, label, function, global,
                                                             const or data symbol *)
           | INT | FLOAT | CHAR | STRING
           | callee "(" [ operand { "," operand } ] ")" [ "->" ( operand | tuple ) ]   (* call sugar *)
           | "[" [ NAME { "," NAME } ] "]"                (* label list *)
           | tuple "->" sigres                            (* signature *)
           | tuple                                        (* tuple *)
callee     = NAME | INT | CHAR
tuple      = "(" [ operand { "," operand } ] ")"
sigres     = type | "(" [ type { "," type } ] ")"

cexpr      = operand-literal-or-name | expression         (* section 18 *)
```

`cexpr` replaces `operand` ONLY in: `memory`, `const`, `global` initialisers, `db/dw/dd/dq/resb`
values, struct field counts `[N]`, `scratch`, the offset (third operand) of a load or store, and
the value (second operand) of `const.i32/i64/f32/f64`. Instruction source operands stay plain
operands because a `(` there starts a tuple (`ret (a, b)`).

Disambiguation, in the parser:
- `NAME`, `INT` or `CHAR` directly followed by `(` (whitespace allowed) is call sugar. The
  parser accepts it anywhere an operand is; only `call` and `call_indirect` give it a meaning.
- A `(`...`)` group at the start of an operand is a **signature** when the token after the
  `)` is `->`, otherwise a **tuple**. Signature elements must be type names.
- `{` ends its line (a Newline must follow, after an optional comment) and `}` stands alone on
  its line (the next token after it must be a Newline or Eof). Everything else on a statement
  line ends at the Newline.
- A label may be followed by a statement on the same line (`done: ret acc`), by another label,
  or by nothing.
- Statement-level keywords `reg` and `nregs` are recognised when they are the first name of a
  statement (after labels); they are reserved so they never clash with mnemonics.
- A top-level line starting with a NAME that is not a keyword must be a data item: `NAME` then
  one of `db dw dd dq resb`.
- Parse errors recover at the next Newline; inside a function body, a statement that fails to
  parse is skipped and parsing resumes with the next line; a missing `}` is reported at Eof.

## 3. Top-level items

Items may appear in any order, each on its own line (except function bodies). Order matters
only among items of the same kind (function index, global index, data layout, import index).

| Item | Syntax | Notes |
|---|---|---|
| import | `import ns.name(i64, i64) -> i64` | Split at the LAST dot into namespace and name (builder `import(ns, name, ty)`); a dot is required. Results `-> t` or `-> (t, t)`; none = no results. Imports are function indices `0..` in source order; called with the qualified name `ns.name`. |
| memory | `memory MIN [MAX]` | Pages of 64 KiB. Integer constant expressions (18), non-negative, fit `u32`. At most one. Default: the pages the data needs, at least 1, no max. Explicit MIN smaller than the data end is an error; MAX < MIN is an error. |
| const | `const NAME = 123`, `const AREA = W * H` | A literal (integer, float or character) or a constant expression (18): arithmetic over literals and other consts, data lengths and struct constants, in any declaration order. A character is an integer constant. A const that uses another file's `pub const` works too (18.4). |
| global | `global NAME: i64 = 0`, `global mut NAME: f64 = 1.5`, `export global [mut] NAME: T = v` | Init is a literal, a `const`, a data symbol or `NAME.len`, or a constant expression (18), typed as the global's type (6). `export` exports it under NAME. |
| table | `table [f1, null, f2]` | Entries are functions/imports (`std.println`) or `null` (unset). At most one. Index 0.. in order. |
| data | `NAME db "text", 10, 0xff`; `NAME dw 1, 2`; `NAME dd 1, 2.5`; `NAME dq 1, -0.5`; `NAME resb 64` | See 8. Needs at least one value (`resb` exactly one count). |
| fn | `fn NAME(a: i64, b: f64) -> i64 { ... }`, `export fn ...` | `export` exports it under NAME. Parameters are `name: type`. |

Name spaces (module level), each rejects redeclaration:
1. **functions** (declared `fn` names; imports live in the same index space but are written
   `ns.name`, so they never collide with a plain name);
2. **globals**;
3. **values**: `const` names and data symbols share one space (both are usable as
   immediates). `NAME.len` is derived from the data symbol, not separately declared.
Within a function: **registers** (parameters + `reg` names) and **labels** are separate local
spaces. Declared names (fn, param, reg, label, global, const, data) have no dots and are not
reserved words and not `r<digits>`. Export names must be unique across functions and globals.

## 4. Functions and registers

```
export fn sum(n: i64) -> i64 {
    reg acc, t
    const.i64 acc, 0
loop:
    i64.eqz t, n
    br_if t, done
    i64.add acc, acc, n
    i64.sub n, n, 1
    jmp loop
done:
    ret acc
}
```
- Parameters are registers `0..nparams` in order. `reg a, b` declares named registers that
  take the next indices (`nparams`, `nparams+1`, ...) in declaration order; `reg` may appear
  several times and anywhere in the body, and the names are usable anywhere in it (the
  lowering collects them first).
- A parameter written `rK: T` where `K` is its own position is **unnamed** (it has no `names`
  entry); `r5: T` at position 2 is an error. This is how the disassembler prints parameters
  that have no name.
- Raw registers `r0`, `r1`, ... may be used anywhere; `rN` is register N whether or not it has
  a name too.
- Labels are `name:` and local to the function; a label names the index of the next
  instruction. A label with no instruction after it (end of the body) is an error, a function
  with no instruction is an error, and a function whose last instruction is not a terminator
  (`ret`, `jmp`, `br_table`, `unreachable`) is an error ("function `f` can fall off its end";
  the assembler never inserts one).
- `nregs N` (optional, at most once, anywhere in the body) sets the function's register count
  exactly. It is an error if N is below the computed requirement (7) or above 65535. Without
  it, nregs is the computed value.
- Types on parameters and results: `i32 i64 f32 f64`.

## 5. Operand forms per instruction shape

Mnemonics are exactly `efevm::Op::name()` (`crates/efevm/OPCODES.md`), operand order as in
the spec: destination first, then sources. `d` is a destination register, `a b c` source
operands, `S` below means "source operand": a register, or an immediate (6) that the assembler
materialises into a hidden temporary. A destination can never be an immediate. Register
operands are `rN`, parameter names or `reg` names. Operand count is checked exactly.

Resolution of a `Name` operand depends on the position: register position -> register first,
then (for a source) const / data symbol / `NAME.len`; label position -> label; function
position -> function or import; global position -> global. A register name shadows a
module-level const or data symbol of the same name. Dotted names never resolve to registers.

| `efevm::Shape` | Mnemonics | Textual forms |
|---|---|---|
| `None` | `nop` `unreachable` `yield` | `nop` |
| `D` | `memory.size` | `memory.size d` |
| `DA` | `mov`, unary int/float ops, `T.eqz`, conversions, `memory.grow` | `OP d, S` (`memory.grow d, S`: pages) |
| `DAB` | binary int/float arithmetic and comparisons | `OP d, S, S` |
| `ConstI32` | `const.i32` | `const.i32 d, imm` |
| `ConstI64` | `const.i64` | `const.i64 d, imm` |
| `ConstF32` | `const.f32` | `const.f32 d, imm` (float or integer literal, `nan`, `inf`, `nan:0x...`) |
| `ConstF64` | `const.f64` | `const.f64 d, imm` |
| `Select` | `select` | `select d, S, S, S` (cond, a, b) |
| `Load` | `T.load*`, `F.load` | `OP d, S [, off]` (base, offset) |
| `Store` | `T.store*`, `F.store` | `OP S, S [, off]` (src, base, offset) |
| `ABC` | `memory.copy`, `memory.fill` | `OP S, S, S` (copy: dst, src, len; fill: dst, value, len) |
| `GlobalGet` | `global.get` | `global.get d, G` (G = global name) |
| `GlobalSet` | `global.set` | `global.set G, S` |
| `Jmp` | `jmp` | `jmp L` |
| `Br` | `br_if` `br_ifz` | `OP S, L` |
| `BrTable` | `br_table` | `br_table S, [L, L, ...], Ldefault` (list may be empty `[]`) |
| `Call` | `call` | sugar `call f(args) -> results`; raw `call f, BASE` |
| `CallIndirect` | `call_indirect` | sugar `call_indirect SIG, idx(args) -> results`; raw `call_indirect SIG, idx, BASE` |
| `Ret` | `ret` | `ret`, `ret S`, `ret (S, S, ...)`, raw `ret BASE` |

Details:
- **const.\***: `imm` is an integer literal, character, `const` name, data symbol or
  `NAME.len` (int forms), or a float/integer literal (float forms). No temporary: the value is
  emitted directly. Range and kind rules are those of the slot type (6).
- **Load/Store offset** `off`: omitted = 0; an integer, character, `const` or data symbol that
  must fit `u32` (an address constant is fine). It is never a register. The **base** may be a
  register or an immediate/data symbol (`i64.load d, msg` loads from the address of `msg`:
  `const.i64 tmp, ADDR` then the load with base `tmp`).
- **Globals** by name: the global position resolves in the global name space.
- **Call, sugar**: `call f(a, b) -> x`, `call f(a, b) -> (x, y)`, `call f(a, b)`, `call f()`.
  `f` is a function name or an import `ns.name`. Arguments: registers or immediates. The
  argument count must equal the callee's parameter count P. The `->` part is either absent
  (results dropped) or lists EXACTLY the callee's R results, all registers (a single result may
  be written without parentheses). See 7 for the expansion.
- **Call, raw**: `call f, BASE`, with BASE a register (named or `rN`); the window is the
  consecutive registers from BASE. No temporaries, no moves.
- **call_indirect**: `SIG` is `(i64, i64) -> i64`, `(i64)` is NOT valid without `->`: write
  `() -> ...` forms with the arrow; a function with no results is `(i64) -> ()`. Sugar:
  `call_indirect (i64, i64) -> i64, idx(a, b) -> x` where `idx` is a register or an integer /
  character / const immediate (type i32). Raw: `call_indirect (i64, i64) -> i64, idx, rBASE`.
  Signature results in a signature are `T`, `(T, T)` or `()`.
- **ret**: `ret` means `ret r0`. `ret X` with `X` a register is the raw `ret X` (return the
  registers `X ... X+R-1`; for R = 1 it returns X). `ret IMM` (function has exactly one result)
  loads the immediate into a window. `ret (a, b)` (arity must equal R; entries are registers
  or immediates) copies into a window. A name that resolves to a const/data symbol is an
  immediate.
- `br_if`, `br_ifz`, `br_table` test/use the low 32 bits of the register (spec 8).

## 6. Immediates and their types

An immediate (integer, float, character, `const` name, data symbol, `NAME.len`; in the
positions of 18.1 also a constant expression) in a source position is loaded into a hidden temporary with a constant of the type the position expects.
`const.T d, imm` loads directly. Never in a destination position (error).

Slot type per mnemonic (the type of the SOURCE operands):

| Mnemonic | Source slot type |
|---|---|
| `i32.*` arithmetic, bitwise, shifts, `clz ctz popcnt eqz`, comparisons | `i32` |
| `i64.*` the same | `i64` |
| `f32.*` / `f64.*` arithmetic, unary, comparisons | `f32` / `f64` |
| `i32.wrap_i64` | `i64` |
| `i64.extend_i32_s`, `i64.extend_i32_u` | `i32` |
| `i32.extend8_s`, `i32.extend16_s` | `i32` |
| `i64.extend8_s`, `i64.extend16_s`, `i64.extend32_s` | `i64` |
| `T.trunc_F_s/u`, `T.trunc_sat_F_s/u` | `F` (the type named in the suffix) |
| `F.convert_T_s/u` | `T` (named in the suffix) |
| `f32.demote_f64` | `f64`; `f64.promote_f32` | `f32` |
| `i32.reinterpret_f32` | `f32`; `i64.reinterpret_f64` | `f64`; `f32.reinterpret_i32` | `i32`; `f64.reinterpret_i64` | `i64` |
| `mov d, S` | integer/char/const/data -> `i64`; float literal -> `f64` |
| `select d, cond, a, b` | cond `i32`; `a`, `b`: integer-like -> `i64`, float literal -> `f64` |
| `br_if`, `br_ifz`, `br_table` index | `i32` |
| `call_indirect` table index | `i32` |
| Loads `*.load*` | base `i64` |
| Stores | `src` by the mnemonic prefix (`i32.store`, `i32.store8`, `i32.store16` -> `i32`; `i64.store*` -> `i64`; `f32.store` -> `f32`; `f64.store` -> `f64`); base `i64` |
| `memory.grow` pages; `memory.copy`, `memory.fill` all three operands | `i64` |
| `global.set g, S` | the global's type |
| call arguments (sugar) | the callee's parameter types (for `call_indirect`: the signature's) |
| `ret` sugar | the function's result types |
| `global` init, `dd`/`dq` values | the global's type; `dd` f32 bits, `dq` f64 bits (4.. data section 8) |

Value rules for a slot of type:
- **i32**: an integer-like value in `-2^31 ..= 2^32-1` (kept as the 32-bit pattern;
  `-1` and `4294967295` are the same constant); a float literal is an error.
- **i64**: `-2^63 ..= 2^64-1` (kept as the 64-bit pattern); a float literal is an error.
- **f32 / f64**: a float literal (parsed directly as `f32` / `f64` from its source text,
  correctly rounded; `inf`, `-inf`, `nan` = positive quiet NaN `0x7fc00000` /
  `0x7ff8000000000000`, `-nan` = the same with the sign bit set, `nan:0xHEX` = the exact bits,
  at most 8 hex digits for f32) or an integer-like value, converted (`1` becomes `1.0`).
- Out of range is an error ("integer 4294967296 does not fit in i32").
- Data symbols and `NAME.len` are integer constants (addresses/sizes); characters are integers.

## 7. Lowering (exact, deterministic)

The disassembler round trip depends on it. `T` below are slot types from 6.

**Pre-pass per function.**
- `B = max(nparams + number of reg names, highest raw register mentioned + 1)` where a raw
  register is any `rN` token in a register position of the body (call bases included).
- Hidden temporaries are `B, B+1, ...` within one statement and are reused by the next
  statement. They are allocated in operand order, left to right, one per immediate in a source
  position; their `const.T tmp, v` instructions are emitted before the statement's
  instruction in the same order.

**Expansion.**
- Immediate source: `const.T tmp, v` then the instruction using `tmp`.
- Load/store with an immediate base: `const.i64 tmp, ADDR` then the load with base `tmp`.
- `call f(a, b) -> (x, y)`: window `W = B`. Argument `i` goes to `W+i`: an immediate becomes
  `const.T W+i, v` (T = parameter type), a register becomes `mov W+i, reg`. Then `call f, W`,
  then `mov x, W`, `mov y, W+1`, ... for the listed results. **No moves are elided.**
  Arguments and results are all processed even if registers coincide.
- `call_indirect SIG, idx(a, b) -> x`: same window `W = B`; argument moves as above; if `idx` is
  a register it is used directly; if it is an immediate it is loaded AFTER the arguments with
  `const.i32 W + max(P, R), v` and that register is the index; then `call_indirect T, idxreg, W`
  (T = the interned signature), then the result moves. `P`, `R` are the signature's counts.
- `ret` -> `ret r0`. `ret X` (register) -> `ret X`. `ret IMM` -> `const.T W, v; ret W` with
  `W = B`. `ret (a, b)` -> `mov W, a; mov W+1, b; ret W` (immediates as `const.T`).
- Raw `call f, BASE`, raw `call_indirect SIG, idx, BASE`, and everything else map one to one.

**nregs** = the maximum of:
- `nparams + number of reg names`;
- every register operand + 1 (temporaries and window registers included);
- `call` base + `max(P, R)` and `call_indirect` base + `max(P, R)`;
- `ret` base + R (so `ret r0` in a function with no results needs nothing);
- for `call_indirect` with an immediate index: `W + max(P,R) + 1`.
`nregs N` (4) overrides it; error when `N` is below this value.

**Builder call order** (`efevm::ModuleBuilder` / `FunctionBuilder`):
1. `import(ns, name, FuncType)` for every import in source order (this interns its type).
2. `declare_func(FuncType)` for every function in source order (interns its type).
3. `global(...)` for every global in source order (`global_i32/i64/f32/f64`).
4. `table(entries)` if present (`Some(index)` / `None` for `null`).
5. `memory(min, max)` (always called; the default applies when there is no `memory` item).
6. `data(offset, bytes)` for every non-`resb` data item in source order; `resb` creates no
   segment.
7. `file(file)` once (the name as passed to `assemble`), then per function in source order: a
   `FunctionBuilder::new(nregs)`, one `new_label` per label in source order (so forward jumps
   work), then the statements: for every emitted instruction a `loc(file_id, line, col)`
   immediately before it, `bind(label)` for each label before the instruction it precedes,
   then the instruction; `define(func, body)`.
   `call_indirect` signature types are interned (`add_type`) when first encountered in a body.
8. Exports: `export_func(name, index)` for exported functions by function index, then
   `export_global(name, index)` for exported globals by global index.
9. Names: `func_name(index, "ns.name")` for every import and `func_name(index, name)` for every
   function by index; then for each function (by index) `reg_name(func, reg, name)` for each
   named register in index order (named parameters, skipping unnamed `rK` ones, then the `reg`
   names).
10. `finish()` verifies. A `LoadError::Verify { func, inst, error }` is mapped to the statement
    that produced instruction `inst` of function `func` (the lowering keeps a per-instruction
    table of statement spans) and reported at that line/column with the verifier's message; other
    load errors are reported without a location.

The position recorded by `loc` for every instruction a statement expands to (temporaries,
window moves, the instruction itself, result moves) is the statement's position: the line
and column of its mnemonic (for `ret`, `call`, ...).

## 8. Data layout (the string system)

- Every data item defines `NAME` (its address) and `NAME.len` (its size in bytes); both are
  integer constants usable wherever an immediate is accepted, forward references included
  (sizes depend only on the values' count and kind, so the layout is a pre-pass).
- Items are laid out in definition order from address **1024**, each aligned to 8. The
  address of the first item is 1024; of the next, `align8(previous end)`.
- Sizes: `db` 1 byte per value (a string contributes one byte per byte), `dw` 2, `dd` 4,
  `dq` 8 bytes per value, little-endian; `resb N` is N zero bytes (N a non-negative integer
  or const; it advances the cursor but creates no segment).
- Values: `db` accepts integers `-128 ..= 255`, characters, `const` names, data symbols
  (must fit) and strings; `dw` `-32768 ..= 65535`; `dd` `-2^31 ..= 2^32-1` or a float
  (stored as `f32` bits); `dq` `-2^63 ..= 2^64-1` or a float (`f64` bits). A string is only
  valid in `db`. Negative values are two's complement.
- Default memory: `ceil(end / 65536)` pages where `end` is the cursor after the last item,
  at least 1.
- Example: `msg db "hi", 10` -> address 1024, len 3; `words dw 1, 2` -> 1032, len 4;
  `buf resb 64` -> 1040, len 64; end 1104 -> 1 page.

## 9. Names and lines (debug info)

- `names`: function names for every import (`ns.name`) and function, register names for the named
  registers (parameters named `name`, and `reg` names). There is no names entry for globals
  or data.
- `lines`: file name list `[file]`; every emitted instruction has an entry
  (`loc` before each instruction, section 7 step 7) mapping it to file id 0, the statement's
  line and column.

## 10. Extras (needed for round trips)

- `nregs N` statement (4).
- `table [f, null, g]`: `null` leaves an entry unset.
- `export global [mut] NAME: T = v`.
- `dd` / `dq` accept float literals (f32 / f64 bits); `dw`/`dd`/`dq`/`db` values accept
  integer literals, characters, consts and data symbols, or constant expressions (18). Instruction
  source operands take no expressions.
- `nan:0x...` exact-bits float literal.

## 11. Round trip

`assemble -> encode -> decode -> disassemble -> assemble` yields identical bytes when compared
as `Module::encode()` with the `lines` section removed from both sides (the disassembled text
has different line numbers by nature). Names are kept and must match. This is guaranteed for
every module this assembler produces; for foreign modules it is best effort, and anything that
`.efa` cannot express is written as a `; warning:` comment (12).

## 12. Disassembler output (everything a `dis` implementation needs)

Input: a decoded `efevm::Module`. Output: valid `.efa` text, raw forms only (no call sugar,
no immediates in source positions, no temporaries) so that reassembly gives the same bytes.
All lines end with `\n`.

**Naming.**
- Imports: `ns.name` as stored.
- Functions: the `names` function name if present, else (when exported) the export name, else
  `f{N}` with `N` the absolute function index (imports occupy the first indices, so the first
  defined function of a module with 3 imports is `f3`). A name that is not a valid identifier
  (has a dot or other chars, starts with a digit, is a reserved word, matches `r<digits>`) is
  sanitised: invalid chars become `_`, a leading digit gets a `_` prefix, a reserved word or
  `r<digits>` gets a `_` prefix; a name that then collides with another gets `_{N}` appended
  (N = function index). If an exported function's export name differs from its printed name,
  the export cannot be expressed: add `; warning: export "X" of function F cannot be expressed`.
- Registers: the `names` register name if present, else `r{N}`. Parameters: `name: T` if the
  parameter has a name, else `r{K}: T` (K = its position).
- Registers beyond the parameters: if the named ones are exactly `nparams, nparams+1, ...`
  (contiguous from `nparams`), print `    reg a, b, c`; if they are not contiguous, print the
  registers that fit and `; warning: register names not contiguous` (the rest print as `rN`).
- Globals: `g{N}` (N = global index), except an exported global which uses its export name (first export
  wins; further exports of the same global give a warning comment).
- Data items: `data{N}` (N = segment index among segments).
- Labels: `L0, L1, ...`, numbered by ascending target instruction index over all targets of
  `jmp`, `br_if`, `br_ifz` and `br_table` (default included) in that function, each distinct
  index once.

**Layout.** Order: imports, `memory` line, globals, `table`, data items, functions. Groups are
separated by one blank line; functions are separated by one blank line. Nothing is printed for
an absent table or an empty group. No header comment.

```
import std.println(i64, i64)

memory 1

global g0: i64 = 0
export global mut ticks: i32 = -1

table [f3, null]

data0 db "hello world!", 0x0a

export fn main() -> i32 {
    reg i, acc
    nregs 12
    const.i64 r3, 1024
L0:
    br_if r2, L1
    ret r0
}
```

- `memory MIN` or `memory MIN MAX` is always printed (even `memory 0`; the assembler's default
  would otherwise add a page).
- Global init: `i32`/`i64` signed decimal; `f32`/`f64` float text (below); `mut` when mutable.
- `table [a, b, null]` entry `Some(i)` -> the function's printed name (import: `ns.name`); a
  table entry is printed with 4 items per line at most is NOT required: one line.
- Function header: `fn NAME(p: T, ...) -> R {` or `-> (R, R)`; no arrow when there are no
  results; `export fn` when exported (by export of kind function pointing at it; first one).
- Body lines are indented 4 spaces; labels are printed alone on a line at column 0 as `L0:`
  immediately before the instruction they target. Then (before any instruction) the
  `    reg ...` line (if any), then `    nregs N` ONLY when the module's nregs differs from the
  value computed by the rule of section 7 over the printed raw instructions.
- Instruction text: `mnemonic` then operands joined by `, `; no operands: only the mnemonic.
  Operand formats by `Shape`: `D` `d`; `DA` `d, a`; `DAB` `d, a, b`; `ConstI32` `d, imm`
  (signed i32 decimal); `ConstI64` `d, imm` (signed i64 decimal); `ConstF32`/`ConstF64`
  `d, float`; `Select` `d, c, a, b`; `Load` `d, base, off`; `Store` `src, base, off` (the
  offset is ALWAYS printed, unsigned decimal); `ABC` `a, b, c`; `GlobalGet` `d, G`;
  `GlobalSet` `G, a`; `Jmp` `L`; `Br` `a, L`; `BrTable` `a, [L, L], Ld` (empty list `[]`);
  `Call` `f, rBASE`; `CallIndirect` `(P..) -> R, a, rBASE` with the signature printed as
  `(i64, i64) -> i64` / `(i64) -> ()` / `() -> (i64, i64)` (parentheses around the results only
  for 0 or 2+ results); `Ret` `rBASE` (`ret r0` for base 0). Register operands print their
  names (or `rN`); a function operand prints its printed name.
- Floats: `inf`, `-inf`; a NaN whose bits are exactly the canonical positive quiet NaN
  (`0x7fc00000` / `0x7ff8000000000000`) prints `nan`; any other NaN (including the negative
  canonical one) prints `nan:0x` + the raw bits as lowercase hex (8 digits for f32, 16 for
  f64); every other value prints with Rust `{:?}` (`1.5`, `-0.0`, `1e20`), which round trips.
  The text always lexes as a float (a `{:?}` result with no `.`/`e` does not occur for floats).
- Data: for each data segment in order, at cursor `c` (starts 1024; after an item it is the
  item's end): let `s = align8(c)`. If `offset == s` print the item; if `offset > s` and
  `offset % 8 == 0`, first print `padN resb {offset - s}` (`N` = a counter) and then the
  item (`s' = align8(s + gap)` equals `offset`); otherwise the segment cannot be expressed:
  `; warning: data segment at offset X cannot be expressed` and the cursor is unchanged. The
  item is `dataN db ...` with the bytes; maximal runs of at least 3 printable ASCII bytes
  (0x20-0x7e) print as `"..."` with `\"` and `\\` escaped, other bytes as `0x%02x`. Items on
  one line are joined by `, `; after 12 items a line ends with `,` and the list continues on
  the next line indented 4 spaces. An empty segment prints a warning comment instead. When a
  module has data bytes below 1024, the same warning applies.
- Warnings are comment lines `; warning: ...` placed where the unexpressible thing would be.

## 13. Diagnostics

One error type, `asm::Diagnostic { file, line, column, len, message, note, source_line }`
(1-based line/column in chars; `line == 0` = no location). `Diagnostics(Vec<Diagnostic>)` prints
all diagnostics separated by a blank line (no trailing newline). `Display` of one:

```
error: unknown register `acx`
  --> hello.efa:7:18
   |
 7 |     i64.add acc, acx, n
   |                  ^^^ did you mean `acc`?
```

Exact rules (`n` = number of digits of the line number):
- Line 1: `error: ` + message.
- Line 2: `n+1` spaces, `--> `, `file:line:column`.
- Line 3: `n+2` spaces, `|`.
- Line 4: one space, the line number, ` | `, the source line (tabs shown as 4 spaces,
  trailing whitespace removed). If the shown line is empty it is ` N |` with no trailing space.
- Line 5: `n+2` spaces, `| `, then `column-1` chars of padding (a tab counts 4), then
  `^` repeated `len` times (a tab in the span counts 4; minimum one caret), then, when a note is
  present, a space and the note.
- `line == 0`: only `error: msg` and `  --> file`, plus `   = note: NOTE` when there is a note.
- The example in `e2-design.md` shows `:7:19`; the caret there sits under column 18, which is what
  this implementation prints.
- `did_you_mean(name, candidates)`: the nearest candidate with Levenshtein distance <= 2 (and
  smaller than the length of `name`, candidates equal to `name` ignored, ties to the first).
- The assembler collects as many errors as it can; the parser recovers at the next line, the
  lowering continues after an error in one statement. Diagnostics of the lexer and the parser
  are merged and sorted by (line, column); the lowering only runs when there are none.
- Recommended messages (not byte-exact contracts): ``unknown register `acx` ``, ``unknown
  label `x` ``, ``unknown function `f` ``, ``unknown global `g` ``, ``duplicate label `x` ``
  (note: first defined at line N), ``expected a register, found 5`` for immediates in
  destination positions, ``integer 4294967296 does not fit in i32``, ``float literal in an integer
  position``, ``function `f` can fall off its end`` (note: end it with ret, jmp, br_table or
  unreachable), ``wrong number of operands for `i64.add`: expected 3, found 2``,
  ``unknown instruction `i64.ad` `` (note: did you mean `i64.add`?), ``label `x` has no
  instruction``, ```nregs 3` is too small, this function needs 7``.
- No input panics the assembler.

## 14. Decisions beyond the design

Beyond `e2-design.md` and `decisions.md` (made while writing the contracts; review these):

1. **Newlines.** The lexer collapses newline runs, drops a Newline right after a `,` (list
   continuation, used by the disassembler for long data), and always ends with `Newline, Eof`.
   `{` must end its line; `}` stands alone on its line.
2. **Float text** keeps the sign, drops `_`; `inf`/`nan` are literals only as whole names, so
   `inf`/`nan` are reserved. `-nan` is allowed (sign bit set); `-nan:0x...` is rejected.
3. **Reserved words**: `import memory const global table fn export mut reg nregs null db dw dd
   dq resb i32 i64 f32 f64 inf nan` and `r<digits>`.
4. **Declared names have no dots.** Import qualified names are split at the last dot.
5. **Name spaces**: functions; globals; consts and data symbols share one space; registers and
   labels are per-function. A register shadows a const or data symbol of the same name. Export
   names are unique across kinds.
6. **`const` values are single literals** (integer, float, character), no references.
7. **`memory`/`table` at most once**; memory operands are integer literals or consts.
8. **`reg` may appear several times and anywhere**, names are collected first; `nregs N` at most
   once, anywhere; N must be at least the computed requirement and at most 65535.
9. **Labels** may share a line with a statement or with each other (`a: b: ret`).
10. **Operand checks**: call argument count equals P; the `->` list is absent or has exactly R
    entries (no discarding placeholder); `ret (..)` arity equals R; `ret IMM` needs R = 1;
    `ret` with no operand is `ret r0` for any R.
11. **call_indirect sugar order**: arguments first, then an immediate index (`const.i32 W +
    max(P,R)`), then the call, then result moves. A tuple of one element in `ret (a)` is still a
    window copy.
12. **memory.* typing**: `memory.grow`, `memory.copy`, `memory.fill` operands (including the
    fill value) are `i64`; `br_if`/`br_ifz` immediates are `i32`. `mov` of a char is `i64`.
13. **Data**: `NAME db`/`dw`/`dd`/`dq` need at least one value; `resb` takes exactly one count;
    value ranges per directive as in section 8; strings only in `db`; no segment for empty
    data; data symbols may be forward-referenced. Item alignment is 8 even for `db`.
14. **Statement position** for `loc`/errors is the mnemonic (line, column); every instruction
    a statement expands to gets it.
15. **File name** in the `lines` section is the `file` argument verbatim.
16. **Verification errors** map to the statement; other load errors have no location.
17. **Disassembler conventions** (section 12): naming and sanitising rules, `f{abs index}`,
    `g{N}` (export name for exported globals), `data{N}`, `padN` gaps, `memory` always printed,
    `reg` line only for contiguous names, order of groups, 4-space indent, labels at column 0,
    string runs of at least 3 printable bytes, 12 items per data line.
18. **Diagnostics**: tabs rendered as 4 spaces; `line == 0` form with `  --> file`; zero-length
    spans print one caret; `did_you_mean` ignores exact matches and requires distance below the
    name's length; lexer and parser diagnostics are merged sorted by (line, column).
19. **Import namespace** must contain a dot; `-> ()` is not accepted for functions or imports.
20. **Characters** accept `\"` too; strings do not accept `\'`.

## 15. Complete example (every feature)

```
; tour.efa: exercises the whole language
import std.println(i64, i64)
import std.print_i64(i64)
import std.print_f64(f64)
import std.print_char(i32)

memory 1 4
const LIMIT = 10
const PI = 3.14159
const NEWLINE = '\n'

global mut counter: i64 = 0
global mut total: f64 = 1.5
export global mut ticks: i32 = -1

table [inc, null, twice]

msg    db "hello world!", NEWLINE, 0
words  dw 1, 2, 0xffff
floats dd 1.5, nan:0x7fc00001
reals  dq 2.5, -0.0, inf
buf    resb 64

fn inc(x: i64) -> i64 {
    i64.add x, x, 1                 ; immediate source
    ret x
}

fn twice(x: i64) -> i64 {
    i64.add x, x, x
    ret x
}

fn swap(a: i64, b: i64) -> (i64, i64) {
    ret (b, a)                      ; window copy
}

fn half(x: f64) -> f64 {
    f64.div x, x, 2                 ; integer literal in a float slot
    ret x
}

fn raw_demo(a: i64, r1: i64) -> i64 {      ; r1 is an unnamed parameter
    reg idx
    const.i32 idx, 0
    mov r4, a                       ; window for the raw calls, base r4
    call_indirect (i64) -> i64, idx, r4
    call inc, r4
    ret r4
}

fn never() {
    unreachable
}

export fn main() -> i32 {
    reg i, acc, t, f, x, y, h
    nregs 12
    const.i64 acc, 0
    const.i64 i, LIMIT
loop:
    i64.eqz t, i
    br_if t, done
    i64.add acc, acc, i
    i64.sub i, i, 1
    jmp loop
done:
    call std.print_i64(acc)
    call std.print_char(NEWLINE)
    call std.println(msg, msg.len)
    call swap(acc, 7) -> (x, y)
    call inc(x) -> x
    call_indirect (i64) -> i64, 2(acc) -> y       ; immediate index: twice
    const.i32 t, 0
    call_indirect (i64) -> i64, t(acc) -> y       ; register index: inc
    call raw_demo(acc, 0) -> x
    i64.store 42, buf, 8            ; immediate source and base symbol
    i64.load x, buf, 8
    i32.store8 'A', buf, 1
    i32.load8_u t, buf, 1
    i64.load y, buf                 ; offset omitted
    f64.load f, reals, 8
    f64.add f, f, PI
    call half(f) -> f
    call std.print_f64(f)
    const.f32 h, 1.5
    f32.add h, h, 2
    f64.promote_f32 f, h
    i64.trunc_f64_s x, f
    i32.wrap_i64 t, x
    select x, t, 1, 2
    global.get y, counter
    i64.add y, y, 1
    global.set counter, y
    global.set total, 2.5
    global.set ticks, -5
    memory.size x
    memory.grow y, 1
    memory.fill buf, 0, 16
    memory.copy buf, msg, msg.len
    nop
    yield
    i32.wrap_i64 t, acc
    br_table t, [first, second], other
first:
    call std.print_char('1')
    jmp finish
second:
    call std.print_char('2')
    jmp finish
other:
    call std.print_char('?')
finish:
    ret 0
}
```

Notes on the example: the module-level items are in several orders on purpose; `buf` is at
address `1024 + 16 + 8 + 4 + ...` aligned to 8 per item (`msg` 1024, `words` 1040, `floats`
1048, `reals` 1056, `buf` 1080); `inc`, `twice` and the unnamed-parameter function
`raw_demo` show the register forms; `raw_demo` mentions `r4`, so its hidden-temporary base `B`
is 5 and its nregs is 5; `main`'s `nregs 12` is above its computed requirement (10).

## 16. Multi-file programs and the mod interface (E8)

Design and rationale: `docs/e8-design.md` sections 3–5. Everything in sections 1–15 stays
valid, and a program of one file without the new items assembles to exactly the same bytes as
before. Entry points: `asm::assemble_program(root, &dyn FileSource, &AsmOptions)` and
`asm::assemble_programs(&[roots], ..)` read the use graph through a `FileSource` (`FsSource`
on native, `MemSource` in memory); `assemble_with(src, file, opts)` is a program of one
in-memory file (a `use` in it finds no file). `efescript build ROOT.efa [MORE.efa...] -o
OUT` follows `use` with `FsSource`. `run`, `debug`, `dis`, `bundle` and the language server
still read one file (they do not follow `use` yet).

### 16.1 `use`

```
use "items.efa"                 ; alias `items` (the file stem)
use "util/math.efa" as m        ; alias `m`
```

- `use STRING [as NAME]` is a top-level item. The string is a path **relative to the
  directory of the file that contains the `use`**, with `/` as the separator, ending in
  `.efa`. Absolute paths, `\`, and empty segments are errors. `..` is allowed, but the
  normalised path must stay inside the **root directory** (the directory of the root file
  given to `efescript build`, or `[build] entry`'s directory for `efescript pack`); leaving
  it is an error. Paths are normalised lexically; the native file source also canonicalises
  and refuses a symlink that leads outside the root. A file that cannot be read is reported
  at the `use` that first reached it. A `use` of the root file (or of the file itself) is
  allowed and refers to it.
- The alias is the `as` name, or the file stem. An alias is local to the file that declares
  it. Two `use` items with the same alias in one file are an error; one file may be used
  under two aliases. Aliases are declared names (no dots, not reserved).
- The **use graph** is every file reachable from the root file. Each file is assembled once
  however often it is used. Cycles are allowed (two files may call each other). Syntax
  errors of every file are reported together; linking only runs when no file has one.
  Diagnostics are sorted by (file order, line, column) and capped at 100 in total.
- Several roots (`assemble_programs`, `build a.efa b.efa`): the first is the root (its
  directory is the root directory; only it may hold `memory` and `scratch`); the others are
  numbered after the first one's graph as if it used them, and must lie inside the root
  directory (they get a namespace from their path).
- Order: the files are numbered depth-first in `use` order starting with the root (the first
  time a file is reached decides its place). This order fixes function, global and import
  indices and the data layout (16.5), so builds are deterministic.

### 16.2 `pub` and visibility

- Every item of a file is private to that file unless marked `pub`:
  `pub fn`, `pub const`, `pub global [mut]`, `pub NAME db ...` (any data directive),
  `pub struct`. `export fn` and `export global` are also visible to other files (an export is
  public by nature). `pub` on `import`, `use`, `memory`, `table`, `scratch` or `provide` is an
  error.
- Imports are per file: a file must declare every host function it calls. Identical imports
  (same `ns.name`) of several files become one module import; a different signature for the
  same `ns.name` in two files is an error naming both.
- Export names (`export fn`, `export global`), provide names (16.6) and exported struct
  names (16.7) are unique across the whole program; the error's note names the file and line
  of the first one (`first defined in main.efa:2`).

### 16.3 Name resolution

Within a file, in the positions of section 5:
- An **undotted** name is resolved in the file's own scope only (its own items, public or
  not, plus registers and labels as before). Another file's item is never found without an
  alias.
- **`A.x`**, where `A` is an alias of this file, is item `x` of the aliased file: a function
  (function position), a global (global position), a `const` or data symbol (immediate
  position); `A.x.len` is the length of data item `x`; `A.S.size`, `A.S.align`, `A.S.<field>`
  are the constants of struct `S` (16.7); `A.f` is also valid as a `table` entry and as
  the function of a `provide`, `&A.S` as a provide type. The item must be public, otherwise
  the error is ``` `x` is private to `util/math.efa` ``` with the note ``declared at
  util/math.efa:3; mark it `pub` to use it from other files``.
- Positions evaluated while the layout is decided (`resb` sizes, `memory`, `scratch`, struct
  array lengths) accept literals and the file's **own** consts only, not `A.X`.
- **`ns.name`** that equals a declared `import` of this file is that import, as before.
- Index struct constants (`Transform.size`, from `--api`) work as before.
- Conflicts are errors at the alias declaration: an alias may not equal a `const`, data,
  global or function name of the same file, a struct name visible in the file (local or from
  the index), or the first segment of a namespace imported by the file (`use "x.efa" as
  engine` next to `import engine.log.info` is refused). So a dotted name always has exactly
  one meaning.

### 16.4 Per-file namespaces and debug info

- Each file has a **namespace** derived from its path relative to the root directory: the
  `.efa` suffix is dropped and `/` becomes `.` (`util/math.efa` → `util.math`). Every segment
  must be an identifier (so `my-file.efa` cannot be used). The root file's namespace is empty.
- `names` section: a function of a non-root file is named `<namespace>.<name>`
  (`util.math.square`); root-file functions keep their plain names; imports keep `ns.name`.
  So backtraces and the debugger show which file a function came from.
- `lines` section: one file entry per file of the use graph, in file order. The root file is
  written as given; the others by the file source's display name of their normalised path
  (relative to the root directory for `MemSource`; with the root directory as the user typed
  it in front for `FsSource::for_file`, so `build examples/main.efa` records
  `examples/util/math.efa` and backtraces find the source). Every instruction maps to its file.
- Diagnostics name the file the error is in (by the same name), and a note names the file of a
  related declaration (`first defined in items.efa:12`).
- `efescript dis` of a multi-file module prints one file; dotted function names are
  sanitised as in section 12 (`util_math_square`). The round trip of section 11 is guaranteed
  per single-file program only.

### 16.5 Program-wide items and layout

- `memory` and `scratch` may appear only in the root file; at most one `table` in the whole
  program (in any file; its entries may be `A.f`). An explicit root `memory` must hold the
  data and the scratch region.
- Data items of all files are laid out in file order (16.1), each file's items in source
  order, from address 1024 with the alignment of section 8, as one continuous cursor. The
  scratch region (16.8) follows the last data item.
- Globals, functions and imports are numbered in file order; imports first in order of first
  appearance, as in section 7.

### 16.6 `provide`: declared mod exports

```
provide give(player: entity, item: str, count: i32) -> (status: i32) = give_impl,
    "Give `count` copies of `item` to `player`; 0 ok, 1 unknown item."
provide count(item: str) -> (n: i64),
    "How many of `item` the bag holds."
```

- `provide NAME(params) [-> (results)] [= FUNC], STRING` declares that the mod offers
  `<mod id>.NAME` (the mod id comes from the manifest when the package is built; the
  assembler does not know it, so its diagnostics say ``provide `NAME` ``). `FUNC` is a function of this file or `A.f` of another file; without `= FUNC` it is
  the function `NAME` of this file. The string is the doc (required, not blank). The line may
  break after the comma before the doc, as any list.
- Parameter and result types: `i32 i64 f32 f64 bool entity str bytes &S`, where `S` is a
  struct of the file (16.7), `A.S`, or an index struct (`&Transform`). `ptr` is not allowed.
  Results are written in parentheses, named, and scalar only (`i32 i64 f32 f64 bool entity`).
- The lowered signature (`bool` → `i32`, `entity` → `i64`, `str`/`bytes` → `i64, i64`, `&S` →
  `i64`) must equal the function's signature exactly; otherwise the error shows both:
  ``provide `p` lowers to `(i32) -> i64`, but its function `p` has `(i64) -> i64` ``. A
  cross-file function is shown with its file's namespace (`items.add`). Parameter and result
  names are unique within a provide; `ptr` is refused with its own error.
- At run time, `str` arrives as a pointer and length **into the provider's own memory** (the
  copy in its scratch region), `bytes` likewise and is copied back to the caller after the
  call, `&S` points to a copy of the struct that is copied back too (e8-design 7). The
  provider must not keep pointers into the scratch region after it returns.
- Provides go into section 13 (`SPEC.md` 9.2). They need not be `export fn`.
- `provide` may appear in any file of the program.

### 16.7 `struct`

```
export struct Item, "One stack of items." {
    id: u32
    count: i32
    pos: f32[2], "World position."
    owner: entity
}
```

- `[pub | export] struct NAME [, STRING] {` then one field per line, `NAME: PRIM [ "[" N "]" ]
  [, STRING]`, then `}` alone. `PRIM` is `u8 i8 u16 i16 u32 i32 u64 i64 f32 f64 bool entity`;
  `[N]` (N ≥ 1, an integer or `const`) makes a fixed array. These type words are contextual,
  not reserved.
- Layout is C's: each field at the next offset aligned to its primitive size, `align` the
  largest field alignment, `size` rounded up to `align`. A struct needs at least one field.
- The struct defines the constants `NAME.size`, `NAME.align` and `NAME.<field>` (byte offset)
  in the file, like index structs (e3-design "Changes recorded": `size`/`align` win over a
  field of that name). A struct name that equals an index struct is an error.
- `export struct` puts the struct in section 13 as one of the mod's own types (it appears in
  the mod's index with the mod as owner). A struct used by a `provide` must be `export` (or
  come from the index, in which case the assembler writes a copy with its owner into section
  13, keeping its component id). `pub struct` only shares the constants with other files.
  A struct name shares the value name space with consts and data (a data item `S` next to
  struct `S` is a duplicate definition).
- Section 13 lists the structs as: every `export struct` of the program in file then source
  order, then the copies of index structs in the order provides first use them. Section 13
  exists only when the program has a provide, an `export struct` or a scratch region, so a
  program without them is unchanged.

### 16.8 `scratch`

`scratch N` (root file, at most once, N an integer or `const`, `0 ..= 1048576`) sets the size in
bytes of the exchange region the engine copies `str`, `bytes` and struct arguments into. Without
it, a program with a provide that needs one gets 4096 bytes; a program without such a provide
gets none. `scratch 0` with such a provide is an error. The region is 8-aligned, placed after
the last data item (at 1024 when there is no data) and counts toward the default `memory`
size (section 8). An explicit `scratch N > 0` reserves the region even without a provide that
needs it.

### 16.9 Grammar and lexer additions

```
item     += use | provide | struct | scratch | event_import | provide_event   (section 17.4)
use       = "use" STRING [ "as" NAME ] NL
provide   = "provide" NAME "(" [ aparam { "," aparam } ] ")"
            [ "->" "(" aparam { "," aparam } ")" ] [ "=" NAME ] "," STRING NL
aparam    = NAME ":" atype
atype     = "i32" | "i64" | "f32" | "f64" | "bool" | "entity" | "str" | "bytes" | "&" NAME
struct    = [ "pub" | "export" ] "struct" NAME [ "," STRING ] "{" NL { field } "}" NL
field     = NAME ":" NAME [ "[" cexpr "]" ] [ "," STRING ] NL
scratch   = "scratch" cexpr NL
func, const, global, data: may start with "pub"
```

- New punctuation token: `&` (only valid before a struct name in an `atype`).
- New reserved words: `use as pub provide struct scratch`. A program of today that uses one of
  them as a name (a label `use:`, a register `reg as`) no longer assembles; the error says
  the word is reserved (``` `use` is a reserved word and cannot be a label name ```; a data
  item `use db 1` gets ``` `use` is a reserved word and cannot be a data name ```). `as` is a
  keyword only after the path of a `use`. Type words of provides (`bool entity str bytes`)
  and fields (`u8 ... entity`) are contextual. `export struct` is accepted; the message for a
  bad word after `export` keeps its E2 wording. `pub` on `import`, `use`, `memory`, `table`,
  `scratch`, `provide`, `export` or `pub` gives ``` `pub` is not allowed on `X` ```.

### 16.10 Example

```
; main.efa (root)
use "items.efa"
import engine.log.info(i64, i64)

provide give(item: str, count: i32) -> (status: i32) = items.add,
    "Add `count` of `item` to the bag; 0 ok, 1 bag full."

export fn init() {
    call engine.log.info(hello, hello.len)
    ret
}

hello db "inventory ready"
```

```
; items.efa
pub const CAPACITY = 64
slots resb 512

pub fn add(name: i64, len: i64, count: i32) -> i32 {
    ; `name`/`len` point into this mod's scratch region; the work is left out here
    ret 0
}
```

## 17. Events (E8.13)

Design: `docs/e8-design.md` "E8.13 Typed script events"; format and engine ABI: `efevm/SPEC.md`
9.2 rule 9 and 9.3, `efevm/API_INDEX.md` "Events". A script sends and reads events through the
engine functions `engine.events.send(ev: i32, payload: ptr)`, `count(ev: i32) -> (count: i32)` and
`read(ev: i32, index: i32, out: ptr) -> (status: i32)`. `ev` is the event's **id inside the
module**: its index in section 14 `events`. This section is how `.efa` fills section 14 and the
events of section 13. A program without these items assembles to the same bytes as before.

### 17.1 `import event`: using an event

```
import event game.score_changed            ; payload and size from the API index (--api)
import event game.score_changed(&Score)    ; explicit payload struct
import event game.reset()                  ; explicit: no payload
```

- `import event NS.NAME [ "(" [ "&" STRUCT ] ")" ]` is a top-level item and file-local, like
  `import`: a file must declare every event it uses. `NS.NAME` splits at the last dot; `NS` is a
  namespace (dot-separated identifiers), `NAME` an identifier. Two imports of the same event in
  one file are an error.
- **With `--api`** the event must exist in the index (``unknown event `game.score_changd` ``, with
  a "did you mean" note). Without a payload list the payload struct and its size come from the
  index. A written payload list must agree with the index: `&S` is the index's struct `S`, `()`
  means the index lists no payload; otherwise ``payload mismatch for event `game.x`: declared
  `&Lives`, but the API index has `&Score` ``. (Exception: the program's own provided events,
  17.3.)
- **Without `--api`** the payload form is required (``event `game.reset` has no payload: write
  `import event game.reset(&Struct)` or `import event game.reset()` ``). `&S` is a struct of the
  file (16.7, `pub`/`export` or not), or `&A.S` a public struct of a used file; its name and size
  go into section 14. There is no index, so nothing checks them here: the host's link check
  (section 14 rules) does.
- **The id** is the constant `NS.NAME.id` (for example `game.score_changed.id`), valid wherever
  an integer immediate is (call arguments, `const.i32`, a global initialiser, data values). It is
  a link-time constant: it cannot be used in layout positions (`resb` size, `memory`, `scratch`,
  array lengths), where the error says so. Reason for the form: `NS.NAME` alone could equal a
  struct constant (`S.field`) or data symbol, and `NS.NAME` is also the name of the *function*
  `import` family; `NS.NAME.id` has three or more segments and is never one of
  `S.size`/`x.len`/`S.field`; the only remaining clash is a `use` alias, which the alias rule of
  16.3 refuses (the first segment of an imported event's namespace counts like an imported
  namespace). `id` follows the `S.id` (component id) precedent of struct constants. An event
  and a function never have the same full name (the index guarantees it); without an index, an
  `import event` and an `import` with the same `ns.name` in the program is an error.
- **Merge.** Section 14 lists the events of all files: unit (file) order, source order within a
  file, first appearance decides the id; identical imports in several files become one entry.
  Different payloads for one event are an error naming both files:
  ``event `game.reset` is declared with &Score (4 bytes) here but with no payload in `main.efa` ``
  (note ``first defined in main.efa:2``). Payload equality is by struct name and size.
- Section 14 is written only when the program has an event import; section 12 (with `--api`)
  also lists the namespaces of the events with their versions.

### 17.2 `provide event`: declaring an event of the mod

```
export struct Hit { damage: i32  who: entity }      ; (one field per line, 16.7)
provide event hit(&Hit), "A hit landed; damage is in hit points."
provide event cleared(), "Everything was cleared."
```

- `provide event NAME ( [&S] ), STRING` (any file). The name is an identifier (not reserved),
  the doc is required and not blank. The payload `&S` is an `export struct` of the program or
  `&A.S` of a used file (it must be `export`), or a struct of the index (a copy with its owner
  goes into section 13, as for provide parameters, 16.6/16.7).
- Names are unique among the program's provided events, and a provided event may not have the
  name of a `provide` (SPEC.md 9.2 rule 9: a function and an event never share a full name).
- They go into `Interface.events` (section 13) in file then source order. Section 13's structs
  are: the `export struct`s, then the copied index structs in order of first use (provides
  first, then events). Section 13 also exists when the program only has provided events.
- The event is `<mod id>.NAME`; the engine registers it when the mod loads.

### 17.3 A mod using its own event

The assembler does not know the mod id, so it cannot name the event's namespace itself. A mod
sends or reads its own event by importing it by its full name with an explicit payload:

```
export struct Hit { damage: i32 }
provide event hit(&Hit), "A hit landed."
import event mymod.hit(&Hit)
...
call engine.events.send(mymod.hit.id, buf)
```

With `--api`, an event that is not in the index is accepted only when the program has a
`provide event` of the same name (its last segment) with the same payload struct name, and the
import has an explicit payload; anything else is ``unknown event``, and a different payload is
reported. The namespace is **not** checked by the assembler (it is the mod's id); the engine's
link check of section 14 (the mod's own events are registered first) and `efescript pack`, which
knows the id, are where a wrong namespace is caught.

### 17.4 Grammar and words

```
item         += event_import | provide_event
event_import  = "import" "event" NAME [ "(" [ "&" NAME ] ")" ] NL      ; NAME is ns.name
provide_event = "provide" "event" NAME "(" [ "&" NAME ] ")" "," STRING NL
```

`event` is a **contextual** word, not reserved: it is a keyword only directly after `import` or
`provide` when a name follows. `import event.ping(i32)` (a function in namespace `event`),
`provide event(...)` (a provide called `event`), and `event` as a label, register, const or data
name keep their meaning. The reserved-word list of 16.9 is unchanged.

## 18. Constant expressions

A constant expression computes a number while the file is assembled. It is a literal or a name
as before, or an arithmetic expression over them. The parser hands the tokens to the workspace
crate `calc` (`crates/calc`), which parses and evaluates them.

### 18.1 Where

Only in these positions (a lone literal or name is parsed exactly as before):

1. `const NAME = expr` and `pub const NAME = expr`;
2. the offset (third operand) of a load or store: `f32.load y, p, 4 * 2`;
3. `resb N` and the values of `db dw dd dq` (a string stays a string);
4. `memory MIN [MAX]` (the two operands are separated by whitespace: `memory 1 + 1 2 * 4`; use
   parentheses to be clear: `memory (1 + 1) (2 * 4)`);
5. `global NAME: T = expr`;
6. struct field array counts `a: u8[N * 2]`, and `scratch N`;
7. the value of `const.T d, expr`.

Not in instruction source operands, call arguments, `nregs`, `table`: a `(` there starts a tuple
or a signature, and a source is a register or one literal. The expression ends at the first
token that cannot continue it: a `,`, `]`, the end of the line, or an operand that follows an
operand (`memory 1 2` is two expressions).

### 18.2 Syntax and precedence

Operands: integers, characters (an integer), floats (`1.5`, `inf`, `nan`; not `nan:0x...`, which
is an exact bit pattern and is only allowed alone), names, and parenthesised expressions.
Unary `-` and `~` bind tightest; the binary operators, from tight to loose, all left
associative (C and Rust order):

| Level | Operators |
|---|---|
| 1 | unary `-`, `~` |
| 2 | `*` `/` `%` |
| 3 | `+` `-` |
| 4 | `<<` `>>` |
| 5 | `&` |
| 6 | `^` |
| 7 | `\|` |

`1 << 2 + 1` is `1 << 3`; `1 | 6 & 3 ^ 1` is `1 | ((6 & 3) ^ 1)`. Comparisons and `&&`/`||` are
lexed by `calc` but not by `.efa`, and a truth value is never a constant here. `5-3`, `5 -3` and
`5 - 3` are all subtractions (a negative literal right after an operand is split into `-` and
the number); `5 - -3` is 8. Because of that, a `memory MIN MAX` whose MAX starts with `-` is read
as one subtraction (a negative maximum is an error anyway). An expression has at most 1000
tokens.

### 18.3 Values and typing

Integers are computed in `i128` with checked arithmetic, floats in `f64`; an integer mixed with a
float is converted to float. `/` and `%` on two integers truncate toward zero (`7 / 2` is 3).
The result is **folded wide, then narrowed at the use site**: the same rules as for a literal
of section 6 apply to it (`i32` slot: `-2^31 ..= 2^32-1`; a float result in an integer slot is
an error; an integer result in a float slot is converted; a byte of `db` is `-128 ..= 255`;
`memory`, `resb`, counts and offsets must fit their own ranges). So `const.i32 d, 1 << 40` is
"integer 1099511627776 does not fit in i32", while `const.i64 d, 1 << 40` is fine. A float result
is handed on as its `f64` text (shortest text that round-trips) and parsed again as `f32` or
`f64`, so `const.f32 d, 0.1 + 0.2` is `0.30000000000000004` rounded to `f32`. A lone float
literal never goes through `f64`: it is parsed straight as `f32` or `f64` as before.

Errors, each at the span of the offending operation or name: `overflow` (`i128`, or
`1 << 200`), `division by zero` (also for floats), a type error (`1.5 << 2`: a float operand of a
bitwise operator), `unknown constant X`, an incomplete or unbalanced expression, a cycle
(```` `A` is defined in terms of itself ```` with the note `cycle: A -> B -> A`, reported at the
use that closes the cycle), and the usual out-of-range and float-in-integer-slot errors of the
use site, whose span is the whole expression. A const that failed does not make its users fail
again.

### 18.4 Names

An expression may use: consts of the file (declared before or after: they are evaluated on
demand, so there is no order); `NAME.len`; struct constants `S.size S.align S.<field>`
(`S.id` of an API struct) of the file or the API index; a field-array length may use a const that
uses `S.size` of another struct; and `alias.NAME` of the `pub` consts, `pub` structs and `pub`
data lengths of a used file (16.3). A non-`pub` name of another file is the usual "private"
error. Other files are declared before the files that `use` them, so their `pub const`
values are known when this file is declared (a `use` cycle is cut at the back edge).

A **data symbol** (`msg`, `alias.msg`) or an **event id** (`ns.name.id`) is a number the linker
decides. It may be used in an expression where the value is only emitted: instruction
immediates, offsets, `global` initial values, `db/dw/dd/dq` values, and `const`s used only
there (such a const is computed after the layout). It cannot be used where the layout itself
needs a number: `memory`, `scratch`, `resb`, struct array counts, and `pub const` (another file
may be declared before this one has an address). The error names the root: ``` `E` depends on
`msg` (a data address), which is only known when the program is linked; it cannot be used here ```.

### 18.5 Implementation

`asm::parser` (`const_operand`) converts the efa tokens of the expression into `calc` tokens
with synthetic byte offsets (token `k` is `k..k+1`) and drives `calc::Parser` to parse one
expression and stop. The AST keeps `Operand::Expr(Box<ConstExpr>)`: the `calc::Expr` plus a table
`spans[k]` back to the file's line/column spans, so every `calc` error span maps to a source span.
Evaluation is in `asm::lower::consts`: `Fold` evaluates consts, struct layouts and data sizes on
demand during `declare` (cycle-checked), late consts in `resolve`, and everything else
(instruction immediates, globals, data bytes) calls `Env::eval_known` with the final names.
