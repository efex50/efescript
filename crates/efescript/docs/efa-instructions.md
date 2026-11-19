# efescript assembly: the instructions, explained

This is a friendly tour of every instruction you can write in a `.efa` file. It tells you
what each one does and where it bites. It is not the formal contract. For the exact textual
rules see [efa-syntax.md](efa-syntax.md); for opcode numbers see
[OPCODES.md](../../efevm/OPCODES.md); for the machine's precise semantics see
[SPEC.md](../../efevm/SPEC.md).

## How to read this

**Registers.** A function works on registers, which are numbered slots named `r0`, `r1`, and
so on. Every register is 64 bits wide. A register does not remember whether it holds an
integer or a float: the instruction decides how to read it. You can give registers names
(`reg total, i`) and function parameters are registers too: the first parameter is `r0`, the
next `r1`, and so on. Every register except the parameters starts at 0.

**Order of operands.** The destination comes first, then the sources:
`i64.add d, a, b` means `d = a + b`. Stores are the exception that proves the rule: they have
no destination, so the value being stored comes first.

**Types.** There are four value types: `i32` and `i64` (integers, signed or unsigned depending
on the instruction) and `f32` and `f64` (IEEE 754 floats). An instruction name starts with
the type it works on: `i32.add` adds 32-bit integers, `f64.add` adds doubles.

**i32 in a 64-bit slot.** An `i32` instruction reads only the low 32 bits of its inputs and
writes a result whose upper 32 bits are zero. So `-1` as an `i32` sits in the slot as
`4294967295`. An `f32` is kept as its 32 bits in the low half.

**Immediates.** Wherever an instruction wants a source, you may write a number, a character
(`'a'`), a `const` name or a data symbol instead of a register. The assembler quietly loads it
into a hidden register first. A destination can never be a number.

**Labels and blocks.** A label is `name:` and marks "the next instruction". Jumps refer to
labels. Every function must end with `ret`, `jmp`, `br_table` or `unreachable`; there is no
falling off the end.

**Traps.** Some instructions can fail: dividing by zero, touching memory that does not exist,
executing `unreachable`. That failure is called a trap. A trap stops the running fiber
immediately; it never crashes the host. When you run a program with the `efescript` command, a
trap prints an error with the source line and the exit code is 3.

**A small program.** It adds 5 + 4 + 3 + 2 + 1 and prints `15`.

```
import std.print_i64(i64)
import std.print_char(i32)

export fn main() {
    reg n, acc, t
    const.i64 n, 5          ; n = 5
    const.i64 acc, 0        ; acc = 0
loop:
    i64.eqz t, n            ; t = (n == 0)
    br_if t, done           ; leave when n is 0
    i64.add acc, acc, n     ; acc += n
    i64.sub n, n, 1         ; n -= 1
    jmp loop
done:
    call std.print_i64(acc) ; prints 15
    call std.print_char('\n')
    ret
}
```

In the tables below, `d` is a destination register, `a b c` are sources (a register or an
immediate), `L` is a label. A name like `i32.add / i64.add` means the instruction exists once
for each type.

## 1. Constants and moves

| mnemonic | operands | what it does |
|---|---|---|
| `const.i32` | `d, imm` | Put a 32-bit integer in `d` (upper bits zero). |
| `const.i64` | `d, imm` | Put a 64-bit integer in `d`. |
| `const.f32` | `d, imm` | Put a single-precision float in `d`. |
| `const.f64` | `d, imm` | Put a double-precision float in `d`. |
| `mov` | `d, a` | Copy all 64 bits of `a` into `d`. |

Notes:

- `const.i32 d, -1` and `const.i32 d, 4294967295` are the same constant. A number that does
  not fit the type is an assembler error, not a silent wrap.
- The float constants accept `1.5`, `1e9`, an integer like `2`, `inf`, `-inf`, `nan`, and the
  exact-bits form `nan:0x7fc00001`.
- A `const` is the way to put a value into a register. Most other instructions can take the
  number directly: `i64.add x, x, 1` works without a separate `const.i64`.
- `mov` does not care about types. `mov d, 5` loads the integer 5; `mov d, 1.5` loads the
  double 1.5.

```
const.i64 big, 0x1_0000_0000   ; underscores are allowed in numbers
const.f32 half, 0.5
mov copy, big                  ; copy == big
```

## 2. Integer arithmetic

| mnemonic | operands | what it does |
|---|---|---|
| `i32.add / i64.add` | `d, a, b` | `d = a + b`, wrapping on overflow. |
| `i32.sub / i64.sub` | `d, a, b` | `d = a - b`, wrapping. |
| `i32.mul / i64.mul` | `d, a, b` | `d = a * b`, keeping the low bits. |
| `i32.div_s / i64.div_s` | `d, a, b` | Signed division, rounds toward zero. |
| `i32.div_u / i64.div_u` | `d, a, b` | Unsigned division. |
| `i32.rem_s / i64.rem_s` | `d, a, b` | Signed remainder (sign follows `a`). |
| `i32.rem_u / i64.rem_u` | `d, a, b` | Unsigned remainder. |
| `i32.clz / i64.clz` | `d, a` | Count leading zero bits. |
| `i32.ctz / i64.ctz` | `d, a` | Count trailing zero bits. |
| `i32.popcnt / i64.popcnt` | `d, a` | Count the one bits. |
| `i32.extend8_s` `i32.extend16_s` | `d, a` | Sign-extend the low 8 or 16 bits to 32 bits. |
| `i64.extend8_s` `i64.extend16_s` `i64.extend32_s` | `d, a` | Sign-extend the low 8, 16 or 32 bits to 64 bits. |

Notes:

- Add, sub and mul never trap. They wrap around, like a clock.
- **Division by zero traps** (`div_s`, `div_u`, `rem_s`, `rem_u`): "integer divide by zero".
- **MIN / -1 traps** for `div_s` only: the result would not fit, so you get "integer
  overflow". `rem_s` of `MIN % -1` is simply 0 and does not trap.
- There is no `neg` or `not`. Use `sub 0, a` and `xor a, -1`.
- `clz` of 0 is the full width (32 or 64); `ctz` of 0 is too.
- The `extend` instructions are for when you have a small signed value in the low bits of a
  register and want it to be a proper full-width signed number.

```
const.i64 x, 17
i64.rem_s r, x, 5          ; r = 2
i64.div_s q, x, 5          ; q = 3
i64.div_s z, x, 0          ; TRAP: integer divide by zero
```

## 3. Bitwise and shifts

| mnemonic | operands | what it does |
|---|---|---|
| `i32.and / i64.and` | `d, a, b` | Bitwise and. |
| `i32.or / i64.or` | `d, a, b` | Bitwise or. |
| `i32.xor / i64.xor` | `d, a, b` | Bitwise exclusive or. |
| `i32.shl / i64.shl` | `d, a, b` | Shift `a` left by `b` bits. |
| `i32.shr_s / i64.shr_s` | `d, a, b` | Shift right, copying the sign bit in (arithmetic). |
| `i32.shr_u / i64.shr_u` | `d, a, b` | Shift right, filling with zeros (logical). |
| `i32.rotl / i64.rotl` | `d, a, b` | Rotate left: bits that fall off the top come back at the bottom. |
| `i32.rotr / i64.rotr` | `d, a, b` | Rotate right. |

Notes:

- **The shift count is taken modulo the width.** `i32.shl d, 1, 33` shifts by 1, not 33.
  `i64.shl d, 1, 64` shifts by 0. A shift never traps.
- `shr_s` keeps the sign: `-8 >> 1` is `-4`. `shr_u` treats the number as unsigned.
- To test a single bit, `and` with a mask, then use the result in `br_if`.

```
i32.shl r, 1, 33           ; r = 2 (33 mod 32 = 1)
i32.and r, 0b1100, 0b0110  ; r = 4
i32.shr_s r, -8, 1         ; low 32 bits are 0xFFFFFFFC, i.e. 4294967292 in the slot
```

## 4. Comparisons and tests

Every comparison writes an `i32` that is exactly `1` (true) or `0` (false), even when it
compares 64-bit values or floats.

| mnemonic | operands | what it does |
|---|---|---|
| `i32.eqz / i64.eqz` | `d, a` | 1 if `a` is zero. |
| `i32.eq / i64.eq` | `d, a, b` | 1 if `a == b`. |
| `i32.ne / i64.ne` | `d, a, b` | 1 if `a != b`. |
| `i32.lt_s / i64.lt_s` | `d, a, b` | Signed `a < b`. |
| `i32.lt_u / i64.lt_u` | `d, a, b` | Unsigned `a < b`. |
| `i32.gt_s / i64.gt_s` | `d, a, b` | Signed `a > b`. |
| `i32.gt_u / i64.gt_u` | `d, a, b` | Unsigned `a > b`. |
| `i32.le_s / i64.le_s` | `d, a, b` | Signed `a <= b`. |
| `i32.le_u / i64.le_u` | `d, a, b` | Unsigned `a <= b`. |
| `i32.ge_s / i64.ge_s` | `d, a, b` | Signed `a >= b`. |
| `i32.ge_u / i64.ge_u` | `d, a, b` | Unsigned `a >= b`. |

Notes:

- The machine does not know whether a number is signed. You choose with `_s` or `_u`. The
  same bits `0xFFFFFFFF` are `-1` to `lt_s` and 4294967295 to `lt_u`.
- Float comparisons are in the next section.
- Equal and not-equal need no signed/unsigned variants.

```
const.i32 m1, -1
i32.lt_s r, m1, 0          ; r = 1  (-1 < 0)
i32.lt_u r, m1, 0          ; r = 0  (4294967295 < 0 is false)
```

## 5. Floating point

Written once for `f32` and once for `f64`; they follow IEEE 754.

| mnemonic | operands | what it does |
|---|---|---|
| `f32.add / f64.add` | `d, a, b` | Add. |
| `f32.sub / f64.sub` | `d, a, b` | Subtract. |
| `f32.mul / f64.mul` | `d, a, b` | Multiply. |
| `f32.div / f64.div` | `d, a, b` | Divide. Division by zero gives infinity or NaN, no trap. |
| `f32.min / f64.min` | `d, a, b` | The smaller of the two. |
| `f32.max / f64.max` | `d, a, b` | The larger of the two. |
| `f32.copysign / f64.copysign` | `d, a, b` | Magnitude of `a` with the sign of `b`. |
| `f32.abs / f64.abs` | `d, a` | Absolute value. |
| `f32.neg / f64.neg` | `d, a` | Flip the sign. |
| `f32.sqrt / f64.sqrt` | `d, a` | Square root (NaN for negative input). |
| `f32.ceil / f64.ceil` | `d, a` | Round up to a whole number. |
| `f32.floor / f64.floor` | `d, a` | Round down. |
| `f32.trunc / f64.trunc` | `d, a` | Round toward zero. |
| `f32.nearest / f64.nearest` | `d, a` | Round to nearest, ties to even (2.5 gives 2, 3.5 gives 4). |
| `f32.eq / f64.eq` | `d, a, b` | 1 if equal. |
| `f32.ne / f64.ne` | `d, a, b` | 1 if not equal. |
| `f32.lt / f64.lt` | `d, a, b` | 1 if `a < b`. |
| `f32.gt / f64.gt` | `d, a, b` | 1 if `a > b`. |
| `f32.le / f64.le` | `d, a, b` | 1 if `a <= b`. |
| `f32.ge / f64.ge` | `d, a, b` | 1 if `a >= b`. |

Notes:

- Floating point never traps. Overflow gives infinity, `0/0` gives NaN.
- **NaN:** NaN is "not a number". Any comparison with a NaN is false, except `ne`, which is
  true. NaN is not equal to itself. The exact bits of a NaN produced by arithmetic are not
  specified, so do not rely on them.
- A plain integer literal is fine in a float slot: `f64.div x, x, 2` divides by 2.0.
- Float comparisons return an `i32` 0 or 1, ready for `br_if`.
- `f32` and `f64` are separate: you must convert between them (section 6). Mixing them in one
  instruction reads garbage bits, not a converted value.

```
const.f64 x, 2.5
f64.nearest y, x           ; y = 2.0
f64.sqrt y, 16             ; y = 4.0
f64.lt c, x, 3             ; c = 1
```

## 6. Conversions

| mnemonic | operands | what it does |
|---|---|---|
| `i32.wrap_i64` | `d, a` | Keep the low 32 bits of an `i64`. |
| `i64.extend_i32_s` | `d, a` | Widen an `i32` to `i64`, copying the sign. |
| `i64.extend_i32_u` | `d, a` | Widen an `i32` to `i64`, filling with zeros. |
| `i32.trunc_f32_s` `i32.trunc_f32_u` `i32.trunc_f64_s` `i32.trunc_f64_u` | `d, a` | Float to `i32`, toward zero. Traps on NaN or out of range. |
| `i64.trunc_f32_s` `i64.trunc_f32_u` `i64.trunc_f64_s` `i64.trunc_f64_u` | `d, a` | Float to `i64`, toward zero. Traps on NaN or out of range. |
| `i32.trunc_sat_f32_s` `i32.trunc_sat_f32_u` `i32.trunc_sat_f64_s` `i32.trunc_sat_f64_u` | `d, a` | Float to `i32`, saturating, never traps. |
| `i64.trunc_sat_f32_s` `i64.trunc_sat_f32_u` `i64.trunc_sat_f64_s` `i64.trunc_sat_f64_u` | `d, a` | Float to `i64`, saturating, never traps. |
| `f32.convert_i32_s` `f32.convert_i32_u` `f32.convert_i64_s` `f32.convert_i64_u` | `d, a` | Integer to `f32`. |
| `f64.convert_i32_s` `f64.convert_i32_u` `f64.convert_i64_s` `f64.convert_i64_u` | `d, a` | Integer to `f64`. |
| `f32.demote_f64` | `d, a` | `f64` to `f32` (may lose precision). |
| `f64.promote_f32` | `d, a` | `f32` to `f64` (exact). |
| `i32.reinterpret_f32` `i64.reinterpret_f64` | `d, a` | Same bits, read as an integer. |
| `f32.reinterpret_i32` `f64.reinterpret_i64` | `d, a` | Same bits, read as a float. |

Notes:

- Read the name from right to left: `i64.trunc_f64_s d, a` makes an `i64` (`d`) from an `f64`
  (`a`), treating the result as signed (`_s`) or unsigned (`_u`).
- `trunc` throws the fraction away (`-2.7` becomes `-2`). It **traps** on NaN ("invalid
  conversion to integer") and when the number is too big for the target ("integer overflow").
- `trunc_sat` never traps. NaN becomes 0; too-large values clamp to the maximum, too-small to
  the minimum (0 for the unsigned forms when negative).
- `reinterpret` does not convert the number; it only changes how the same bits are read. It is
  how you look at the raw bits of a float.
- The `i32` results of `trunc` and `wrap` have zero upper bits. To get a proper signed 64-bit
  number out of a signed `i32`, use `i64.extend_i32_s`.

```
export fn main() {
    reg f, i
    const.f64 f, 1e20
    i64.trunc_sat_f64_s i, f   ; i = 9223372036854775807 (clamped)
    const.f64 f, -2.7
    i64.trunc_f64_s i, f       ; i = -2
    const.f64 f, nan
    i64.trunc_sat_f64_s i, f   ; i = 0
    i64.trunc_f64_s i, f       ; TRAP: invalid conversion to integer
    ret
}
```

## 7. Select

| mnemonic | operands | what it does |
|---|---|---|
| `select` | `d, cond, a, b` | `d = a` if the low 32 bits of `cond` are non-zero, otherwise `d = b`. |

Notes:

- It is a branch-free "if". Both `a` and `b` are read; only one is copied.
- It copies all 64 bits, so it works for integers and floats alike.
- A typical use is the maximum: `i64.gt_s c, x, y` then `select m, c, x, y`.

```
i64.lt_s c, x, y
select small, c, x, y      ; small = the lesser of x and y
```

## 8. Memory loads and stores

Memory is one array of bytes, numbered from 0. It is little-endian: the lowest byte of a
multi-byte value is stored first. The first data item you declare lives at address 1024. The
address of an access is `base + off`, where `base` is a 64-bit register (or a data symbol,
which is an address) and `off` is an optional constant that is never a register. Leave `off`
out for 0. There is no alignment requirement.

If any byte of the access lies outside memory, the program traps with "out of bounds memory
access". An address that wraps around also traps.

**Loads** read memory into a register: `OP d, base [, off]`.

| mnemonic | what it does |
|---|---|
| `i32.load` | Read 4 bytes. |
| `i64.load` | Read 8 bytes. |
| `f32.load` | Read 4 bytes as a float. |
| `f64.load` | Read 8 bytes as a double. |
| `i32.load8_s` / `i32.load8_u` | Read 1 byte, sign- or zero-extend to 32 bits. |
| `i32.load16_s` / `i32.load16_u` | Read 2 bytes, sign- or zero-extend to 32 bits. |
| `i64.load8_s` / `i64.load8_u` | Read 1 byte, sign- or zero-extend to 64 bits. |
| `i64.load16_s` / `i64.load16_u` | Read 2 bytes, sign- or zero-extend to 64 bits. |
| `i64.load32_s` / `i64.load32_u` | Read 4 bytes, sign- or zero-extend to 64 bits. |

**Stores** write a register to memory: `OP src, base [, off]` (value first, then address).

| mnemonic | what it does |
|---|---|
| `i32.store` | Write the low 4 bytes of `src`. |
| `i64.store` | Write all 8 bytes. |
| `f32.store` | Write 4 bytes of a float. |
| `f64.store` | Write 8 bytes of a double. |
| `i32.store8` / `i32.store16` | Write only the low 1 or 2 bytes of an `i32`. |
| `i64.store8` / `i64.store16` / `i64.store32` | Write only the low 1, 2 or 4 bytes of an `i64`. |

Notes:

- The `_s` and `_u` suffix says how a narrow value fills the wider register. Byte `0xF0` loaded
  with `load8_u` is 240; with `load8_s` it is -16 (shown as 4294967280 in an `i32` slot, so
  follow it with `i64.extend_i32_s` to see -16 in 64 bits).
- Narrow stores simply drop the high bits. There is no signed or unsigned store.
- You can write the base as a data symbol: `i64.load v, table_data` reads from its address.
- Addresses are 64-bit, offsets are 32-bit.

A complete program (it prints -16, 240, -16, 65520, 136, then traps on the last load):

```
import std.print_i64(i64)
import std.print_char(i32)
buf resb 16                    ; 16 zero bytes at address 1024
fn show(v: i64) {
    call std.print_i64(v)
    call std.print_char('\n')
    ret
}
export fn main() {
    reg v
    i32.store 0xfff0, buf       ; bytes F0 FF 00 00
    i32.load8_s v, buf
    i64.extend_i32_s v, v       ; make the sign visible in 64 bits
    call show(v)                ; -16
    i32.load8_u v, buf
    call show(v)                ; 240
    i32.load16_s v, buf, 0
    i64.extend_i32_s v, v
    call show(v)                ; -16
    i64.load32_u v, buf
    call show(v)                ; 65520
    i64.store 0x1122334455667788, buf, 8
    i32.load8_u v, buf, 8
    call show(v)                ; 136 (0x88: little-endian puts it first)
    i64.load v, buf, 65535      ; TRAP: out of bounds (8 bytes past the end of page 1)
    ret
}
```

## 9. Memory size, grow, copy and fill

| mnemonic | operands | what it does |
|---|---|---|
| `memory.size` | `d` | `d` = current memory size in 64 KiB pages. |
| `memory.grow` | `d, a` | Try to add `a` pages. `d` = the old size in pages, or -1 if it failed. |
| `memory.copy` | `dst, src, len` | Copy `len` bytes from `src` to `dst`. Overlap is handled correctly. |
| `memory.fill` | `dst, value, len` | Set `len` bytes starting at `dst` to the low byte of `value`. |

Notes:

- All the operands are 64-bit values. Growing can fail (the limit is set by the program that
  runs you); failure is not a trap, you just get -1 in `d`. The new pages are zero.
- `copy` and `fill` trap with out of bounds if either range is not fully inside memory.
- Cost: normally every instruction costs 1 unit of the run budget. `memory.copy` and
  `memory.fill` cost `1 + len / 256`, and `memory.grow` costs `1 + pages * 256` (charged even
  when it fails). A big copy under a small budget still completes and the fiber then pays the
  excess back out of its next resumes. A copy that traps costs only 1.

A complete program (prints 15, then 1, 1, 3, -1, 506381209866536711, 117901063 on separate
lines; the `show` helper is as in section 8):

```
import std.print_i64(i64)
import std.print_char(i32)
global mut counter: i64 = 10
buf resb 8
fn show(v: i64) {
    call std.print_i64(v)
    call std.print_char('\n')
    ret
}
export fn main() {
    reg v, old
    global.get v, counter
    i64.add v, v, 5
    global.set counter, v
    global.get v, counter
    call show(v)                ; 15
    memory.size v
    call show(v)                ; 1
    memory.grow old, 2
    call show(old)              ; 1 (the old size)
    memory.size v
    call show(v)                ; 3
    memory.grow old, 100000
    call show(old)              ; -1 (refused: over the limit)
    memory.fill buf, 7, 8       ; eight bytes of 0x07
    i64.load v, buf
    call show(v)                ; 0x0707070707070707 = 506381209866536711
    memory.copy 2000, buf, 4
    i32.load v, 2000
    call show(v)                ; 117901063 = 0x07070707
    ret
}
```

## 10. Globals

A global is a named value that lives outside any function and is shared by all of them. You
declare it at the top of the file: `global counter: i64 = 0` (read-only) or
`global mut counter: i64 = 0` (can be changed).

| mnemonic | operands | what it does |
|---|---|---|
| `global.get` | `d, G` | Copy global `G` into `d`. |
| `global.set` | `G, a` | Store `a` into global `G`. |

Notes:

- `G` is the global's name, not a register.
- Setting a global that is not `mut` is rejected when the program is assembled, not at run
  time.
- The global has a type (`i32`, `i64`, `f32`, `f64`) and an immediate value you give to
  `global.set` is converted to that type.
- `export global` makes it visible to the program that embeds the VM.

```
global mut ticks: i32 = 0
...
global.get t, ticks
i32.add t, t, 1
global.set ticks, t
```

(The complete program in section 9 shows a global in use.)

## 11. Control flow

| mnemonic | operands | what it does |
|---|---|---|
| `jmp` | `L` | Go to label `L`. |
| `br_if` | `a, L` | Go to `L` if the low 32 bits of `a` are not zero. |
| `br_ifz` | `a, L` | Go to `L` if the low 32 bits of `a` are zero. |
| `br_table` | `a, [L0, L1, ...], Ldefault` | Go to the label at position `a`; if `a` is too big, go to `Ldefault`. |

Notes:

- A label is `name:` on its own line or before an instruction. It is local to its function.
- `br_if` and `br_ifz` look only at the **low 32 bits**. If you have an `i64` that you want to
  test, use `i64.eqz` or `i64.ne` first (which produce an `i32`), or you may test the wrong
  half of the number.
- `br_table` reads the index as an unsigned 32-bit number, so a negative index goes to the
  default. The default label is required. The list may be empty: `br_table a, [], L`.
- If a branch is not taken, the next line runs. A function must end with a terminator, so the
  last line is often `ret`, `jmp` or `unreachable`.
- A loop is a label at the top, a test, and a `jmp` back (see the first example).

A complete program using `br_table` (prints `01?`):

```
import std.print_char(i32)
fn pick(i: i32) {
    br_table i, [zero, one], other
zero:
    call std.print_char('0')
    ret
one:
    call std.print_char('1')
    ret
other:
    call std.print_char('?')
    ret
}
export fn main() {
    call pick(0)
    call pick(1)
    call pick(7)
    call std.print_char('\n')
    ret
}
```

## 12. Calls and returns

**The calling convention, simply.** When a function calls another, the arguments sit in
consecutive registers starting at some base register `B` of the caller: the first argument in
`B`, the second in `B+1`, and so on. The call runs the callee with those values as its
parameters `r0, r1, ...`. When the callee returns, its results are written back into the
caller's registers starting at `B`. No other register of the caller changes. The caller's
register count must be big enough to hold the window (the assembler works that out for you).
Each call gets its own private registers, so recursion just works. Going too deep traps with
stack overflow.

| mnemonic | operands | what it does |
|---|---|---|
| `call` | `f(args) -> results` | Call function (or import) `f`. Sugar form, see below. |
| `call` | `f, BASE` | Raw form: the window starts at register `BASE`. |
| `call_indirect` | `SIG, idx(args) -> results` | Call the function at table position `idx`, which must have signature `SIG`. |
| `call_indirect` | `SIG, idx, BASE` | Raw form. |
| `ret` | (see below) | Return from the function. |

**The easy way (sugar).** You write the call like a function call in a normal language, and
the assembler does the moving for you:

```
call add(3, 4) -> x             ; one result
call divmod(17, 5) -> (q, r)    ; two results
call std.println(msg, msg.len)  ; no results wanted
```

Arguments may be registers or numbers. The count must match the function's parameters, and
the `->` list, if present, must list exactly as many registers as the function returns. If you
do not want the results, leave out the `->` part. (There is no "ignore this one" placeholder.)

**The raw way.** `call f, r4` means "my arguments are already in `r4`, `r5`, ...; put the
results there". The assembler adds no moves. Use it when you manage the window yourself, for
example to pass a result straight into the next call.

**Returning.** A function declares its results in its header: `fn divmod(a: i64, b: i64) ->
(i64, i64)`.

| form | meaning |
|---|---|
| `ret` | Same as `ret r0`. Fine for a function with no results, or whose result is already in `r0`. |
| `ret x` | Return the register `x` (for several results, `x`, `x+1`, ...). |
| `ret 0` | Return the number 0 (only when the function has exactly one result). |
| `ret (a, b)` | Return several values, which are copied into a window for you. |

The raw `ret BASE` returns the registers `BASE ... BASE+R-1`.

**Indirect calls.** `table [add, mul]` at the top of the file makes a numbered list of
functions (index 0 is `add`). `call_indirect` picks one by number at run time. You give the
signature you expect: `(i64, i64) -> i64`. A function with no results is `(i64) -> ()`. If the
index is outside the table or the entry is `null` the program traps (undefined element); if the
function there has a different signature it traps (indirect call type mismatch).

A complete program (prints `7 3 2 42`):

```
import std.print_i64(i64)
import std.print_char(i32)
table [add, mul]
fn add(a: i64, b: i64) -> i64 {
    i64.add a, a, b
    ret a
}
fn mul(a: i64, b: i64) -> i64 {
    i64.mul a, a, b
    ret a
}
fn divmod(a: i64, b: i64) -> (i64, i64) {
    reg q, r
    i64.div_s q, a, b
    i64.rem_s r, a, b
    ret (q, r)
}
export fn main() {
    reg x, y, k
    call add(3, 4) -> x
    call std.print_i64(x)       ; 7
    call std.print_char(' ')
    call divmod(17, 5) -> (x, y)
    call std.print_i64(x)       ; 3
    call std.print_char(' ')
    call std.print_i64(y)       ; 2
    call std.print_char(' ')
    const.i32 k, 1              ; table index 1 is mul
    call_indirect (i64, i64) -> i64, k(6, 7) -> x
    call std.print_i64(x)       ; 42
    call std.print_char('\n')
    ret
}
```

The entry point the command-line tool runs is the exported function `main`. It may return
nothing, or one `i32` which becomes the process exit code.

## 13. Fibers, yield, unreachable, nop

| mnemonic | operands | what it does |
|---|---|---|
| `yield` | | Pause this fiber and give control back to whoever is running it. Resuming continues after the `yield`. |
| `unreachable` | | Always traps ("unreachable"). Marks code that must never run. |
| `nop` | | Does nothing (but still costs 1 unit of budget). |

Notes:

- A fiber is a coroutine: its own registers and call stack, but the same memory and globals as
  other fibers. The VM never switches fibers by itself; the program that embeds it decides who
  runs next. So `yield` is a polite "someone else can go now".
- The `efescript run` command has no way to start a second fiber yet, so `yield` is mostly of
  interest to embedders. The VM also pauses a fiber when it runs out of its instruction
  budget; that is invisible to your code.
- `unreachable` is a terminator, so it is a good way to end a function whose last branch
  cannot happen. It is also the usual way to finish a function that has no sensible result
  (`fn never() { unreachable }`).

## 14. std host functions

Functions that live outside the VM are imported by name. Declare each one with an `import`
line and call it like any function. The ones the `efescript` tool provides are in the `std`
namespace:

| import | what it does |
|---|---|
| `import std.print(i64, i64)` | Write `len` bytes starting at address `ptr`. Call as `call std.print(ptr, len)`. |
| `import std.println(i64, i64)` | The same, then a newline. |
| `import std.print_i64(i64)` | Print a signed 64-bit integer. |
| `import std.print_f64(f64)` | Print a double. |
| `import std.print_char(i32)` | Print one character (a code point). |
| `import std.read_line(i64, i64) -> i64` | Read a line into memory at `ptr`, at most `cap` bytes. Returns the number of bytes. |
| `import std.play_sound(i64, i64) -> i32` | Play the sound file whose path is at `ptr`, `len` bytes. Returns 0 for ok. |
| `import std.exit(i32)` | Stop the program with this exit code. |

A bad pointer or a path that is not valid text makes the call trap or fail; it never crashes the
host. Imports use the same window as ordinary calls, so the sugar works: `call std.println(msg,
msg.len)` where `msg db "hello"` declares the bytes and `msg.len` is their size.

## 15. Common mistakes

- **Forgetting the terminator.** A function must end with `ret`, `jmp`, `br_table` or
  `unreachable`. Otherwise you get "function does not end in a terminator". Even a function that
  returns nothing needs `ret`.
- **An immediate as a destination.** `i64.add 5, a, b` is an error: the first operand is where
  the result goes, so it must be a register.
- **Signed versus unsigned.** `lt_s` and `lt_u` give different answers for the same bits. If a
  loop "never ends" or a comparison with a negative number goes wrong, check the suffix.
- **i32 results are zero-extended.** `i32.add` of `-1` and `0` leaves `4294967295` in the slot,
  not `-1`. If you then pass it to an `i64` instruction or print it as `i64`, you get the big
  positive number. Use `i64.extend_i32_s` first.
- **Mixing widths.** `i32.add` ignores the top half of its inputs; `i64.add` of an `i32` result
  works, but of a value you only meant as 32 bits may not.
- **Branching on a 64-bit value.** `br_if` looks at the low 32 bits only. A value such as
  `0x1_0000_0000` is "false". Compare with `i64.eqz` or `i64.ne` first.
- **Argument and result counts.** `call f(a)` when `f` has two parameters is an assembler
  error, as is a `->` list with the wrong number of names.
- **Division and conversion traps.** `div` and `rem` by zero trap; `div_s` of the minimum value
  by -1 traps; `trunc` of NaN or a huge float traps. Use `trunc_sat` if you want clamping, and
  test the divisor first if it can be zero.
- **Shift counts.** Shifting by the width or more wraps (`i32.shl x, 1, 32` is shift by 0).
- **Float and integer in the same register.** The machine will not stop you from adding a float
  with `i64.add`; you will just get nonsense. Use `reinterpret` when you mean to look at bits.
- **Memory is not where you think.** The first data item is at address 1024. Addresses
  below that are valid memory too, but nothing is there. Going past the end of the memory
  traps, so a loop that walks too far ends with "out of bounds memory access".
- **Constant expressions are computed by the assembler, not the machine.** `const N = 4 * 8`
  works, and so do `memory`, `resb`, `global` initial values, `db`/`dd` values, load and store
  offsets and `const.i32 d, 1 << 4`. An instruction source operand does not: `i64.add d, a, 2 + 3`
  is an error (write `5`, or a const). Integer division in an expression truncates (`7 / 2` is 3);
  see efa-syntax section 18.
- **Calls and `r` registers.** If you mention a high raw register like `r9` anywhere, the
  assembler's hidden temporaries start above it. That is fine, but avoid reusing the window
  registers of a raw `call` for something else while a call is pending.

## Where to go next

- [efa-syntax.md](efa-syntax.md): the formal syntax, lowering rules and error messages.
- [OPCODES.md](../../efevm/OPCODES.md): every opcode number.
- [SPEC.md](../../efevm/SPEC.md): the VM contract (traps, limits, fibers, binary format).
- `crates/efescript/examples/*.efa`: small programs to read and run.
