//! Instruction tests for the integer ops of SPEC.md section 5 ("Integer (for each T)"),
//! for both i32 and i64.
//!
//! Every op runs through a tiny function: two (or one) i64-typed params in r0/r1,
//! `op3`/`op2` into r2, `ret 2`. i64 params and results pass raw register bits through
//! unchanged, so the tests see exactly what the VM computes.

mod common;
use std::collections::BTreeSet;

use efevm::*;

type Case = (Op, u64, u64, Result<u64, Trap>);
type UnCase = (Op, u64, Result<u64, Trap>);

/// The mnemonics of section 5 that are integer ops for each T.
const MNEMONICS: &[&str] = &[
    "add", "sub", "mul", "and", "or", "xor", "shl", "shr_s", "shr_u", "rotl", "rotr", "div_s",
    "div_u", "rem_s", "rem_u", "clz", "ctz", "popcnt", "eqz", "eq", "ne", "lt_s", "lt_u", "gt_s",
    "gt_u", "le_s", "le_u", "ge_s", "ge_u",
];

const BIN_MNEMONICS: &[&str] = &[
    "add", "sub", "mul", "and", "or", "xor", "shl", "shr_s", "shr_u", "rotl", "rotr", "div_s",
    "div_u", "rem_s", "rem_u", "eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s",
    "ge_u",
];

const UN_MNEMONICS: &[&str] = &["clz", "ctz", "popcnt", "eqz"];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The op with this exact mnemonic, e.g. `op("i32.add")`.
fn op(name: &str) -> Op {
    *Op::ALL
        .iter()
        .find(|o| o.name() == name)
        .unwrap_or_else(|| panic!("no op named {name}"))
}

/// Builds `main(args...) -> i64` whose body is `emit` followed by `ret 2`, runs it on a
/// fresh instance, and returns the result or the trap.
fn run(emit: impl FnOnce(&mut FunctionBuilder), args: &[u64]) -> Result<u64, Trap> {
    let mut mb = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(3);
    emit(&mut fb);
    fb.ret(2);
    let ty = FuncType::new(vec![ValType::I64; args.len()], [ValType::I64]);
    let f = mb.add_func(ty, fb);
    mb.export_func("main", f);
    let module = mb.finish().expect("module should verify");
    let mut inst =
        common::new_instance(&module, &Imports::<()>::new(), Limits::default()).expect("instance");
    let fiber = inst.spawn("main", args).expect("spawn");
    match inst.resume(fiber, &mut (), 1_000_000) {
        Exit::Done => Ok(inst.results(fiber).expect("done fiber has results")[0]),
        Exit::Trap(t) => Err(t),
        other => panic!("unexpected exit {other:?}"),
    }
}

/// Runs a binary op: `r2 = op(r0, r1)`.
fn bin(op: Op, a: u64, b: u64) -> Result<u64, Trap> {
    run(|f| f.op3(op, 2, 0, 1), &[a, b])
}

/// Runs a unary op: `r2 = op(r0)`.
fn un(op: Op, a: u64) -> Result<u64, Trap> {
    run(|f| f.op2(op, 2, 0), &[a])
}

fn check_bin(cases: Vec<Case>) {
    for (op, a, b, want) in cases {
        let got = bin(op, a, b);
        assert_eq!(got, want, "{} a={a:#x} b={b:#x}", op.name());
    }
}

fn check_un(cases: Vec<UnCase>) {
    for (op, a, want) in cases {
        let got = un(op, a);
        assert_eq!(got, want, "{} a={a:#x}", op.name());
    }
}

// ---------------------------------------------------------------------------
// Reference implementation (Rust's own integer semantics)
// ---------------------------------------------------------------------------

macro_rules! reference_impl {
    ($bin:ident, $un:ident, $u:ty, $s:ty, $bits:expr) => {
        /// Reference result of a binary op on the low bits of the inputs.
        fn $bin(mnem: &str, a: u64, b: u64) -> Result<u64, Trap> {
            let x = a as $u;
            let y = b as $u;
            let sx = x as $s;
            let sy = y as $s;
            let sh = (y as u64 % $bits) as u32;
            let v: u64 = match mnem {
                "add" => x.wrapping_add(y) as u64,
                "sub" => x.wrapping_sub(y) as u64,
                "mul" => x.wrapping_mul(y) as u64,
                "and" => (x & y) as u64,
                "or" => (x | y) as u64,
                "xor" => (x ^ y) as u64,
                "shl" => x.wrapping_shl(sh) as u64,
                "shr_u" => x.wrapping_shr(sh) as u64,
                "shr_s" => sx.wrapping_shr(sh) as $u as u64,
                "rotl" => x.rotate_left(sh) as u64,
                "rotr" => x.rotate_right(sh) as u64,
                "div_u" => {
                    if y == 0 {
                        return Err(Trap::DivideByZero);
                    }
                    (x / y) as u64
                }
                "rem_u" => {
                    if y == 0 {
                        return Err(Trap::DivideByZero);
                    }
                    (x % y) as u64
                }
                "div_s" => {
                    if sy == 0 {
                        return Err(Trap::DivideByZero);
                    }
                    if sx == <$s>::MIN && sy == -1 {
                        return Err(Trap::IntegerOverflow);
                    }
                    (sx / sy) as $u as u64
                }
                "rem_s" => {
                    if sy == 0 {
                        return Err(Trap::DivideByZero);
                    }
                    sx.wrapping_rem(sy) as $u as u64
                }
                "eq" => (x == y) as u64,
                "ne" => (x != y) as u64,
                "lt_s" => (sx < sy) as u64,
                "lt_u" => (x < y) as u64,
                "gt_s" => (sx > sy) as u64,
                "gt_u" => (x > y) as u64,
                "le_s" => (sx <= sy) as u64,
                "le_u" => (x <= y) as u64,
                "ge_s" => (sx >= sy) as u64,
                "ge_u" => (x >= y) as u64,
                other => panic!("no reference for binary mnemonic {other}"),
            };
            Ok(v)
        }

        /// Reference result of a unary op on the low bits of the input.
        fn $un(mnem: &str, a: u64) -> u64 {
            let x = a as $u;
            match mnem {
                "clz" => x.leading_zeros() as u64,
                "ctz" => x.trailing_zeros() as u64,
                "popcnt" => x.count_ones() as u64,
                "eqz" => (x == 0) as u64,
                other => panic!("no reference for unary mnemonic {other}"),
            }
        }
    };
}

reference_impl!(ref_i32_bin, ref_i32_un, u32, i32, 32u64);
reference_impl!(ref_i64_bin, ref_i64_un, u64, i64, 64u64);

/// Reference for a full op name such as "i64.rotl".
fn reference_bin(name: &str, a: u64, b: u64) -> Result<u64, Trap> {
    match name.split_once('.') {
        Some(("i32", m)) => ref_i32_bin(m, a, b),
        Some(("i64", m)) => ref_i64_bin(m, a, b),
        _ => panic!("not an integer op: {name}"),
    }
}

fn reference_un(name: &str, a: u64) -> u64 {
    match name.split_once('.') {
        Some(("i32", m)) => ref_i32_un(m, a),
        Some(("i64", m)) => ref_i64_un(m, a),
        _ => panic!("not an integer op: {name}"),
    }
}

// ---------------------------------------------------------------------------
// Deterministic pseudo-random inputs
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Full-width value, or (every third call) a small value that hits zero, equal and
    /// near-width counts.
    fn operand(&mut self, i: usize) -> u64 {
        let v = self.next();
        if i % 3 == 0 { v % 70 } else { v }
    }
}

// ---------------------------------------------------------------------------
// Case tables
// ---------------------------------------------------------------------------

fn wrapping_cases() -> Vec<Case> {
    vec![
        (op("i32.add"), 0xFFFF_FFFF, 1, Ok(0)),
        (op("i64.add"), u64::MAX, 1, Ok(0)),
        (op("i32.add"), 0xFFFF_FFFF, 0xFFFF_FFFF, Ok(0xFFFF_FFFE)),
        (op("i32.add"), 0x7FFF_FFFF, 1, Ok(0x8000_0000)),
        (op("i32.mul"), 0x8000_0000, 2, Ok(0)),
        (op("i32.mul"), 0x7FFF_FFFF, 0x7FFF_FFFF, Ok(1)),
        (op("i64.mul"), 0x8000_0000_0000_0000, 3, Ok(0x8000_0000_0000_0000)),
        (op("i64.mul"), 0x7FFF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF, Ok(1)),
        // 0 - 1 is 0xFFFF_FFFF zero-extended for i32, never u64::MAX.
        (op("i32.sub"), 0, 1, Ok(0xFFFF_FFFF)),
        (op("i64.sub"), 0, 1, Ok(u64::MAX)),
        (op("i32.sub"), 0x8000_0000, 1, Ok(0x7FFF_FFFF)),
        (op("i64.sub"), 0x8000_0000_0000_0000, 1, Ok(0x7FFF_FFFF_FFFF_FFFF)),
    ]
}

fn bitwise_cases() -> Vec<Case> {
    vec![
        (op("i32.and"), 0xF0F0_F0F0, 0xFF00_FF00, Ok(0xF000_F000)),
        (op("i32.or"), 0xF0F0_F0F0, 0xFF00_FF00, Ok(0xFFF0_FFF0)),
        (op("i32.xor"), 0xF0F0_F0F0, 0xFF00_FF00, Ok(0x0FF0_0FF0)),
        (op("i64.and"), 0xF0F0_F0F0_F0F0_F0F0, 0xFF00_FF00_FF00_FF00, Ok(0xF000_F000_F000_F000)),
        (op("i64.or"), 0xF0F0_F0F0_F0F0_F0F0, 0xFF00_FF00_FF00_FF00, Ok(0xFFF0_FFF0_FFF0_FFF0)),
        (op("i64.xor"), 0xF0F0_F0F0_F0F0_F0F0, 0xFF00_FF00_FF00_FF00, Ok(0x0FF0_0FF0_0FF0_0FF0)),
    ]
}

fn shift_cases() -> Vec<Case> {
    vec![
        // shl: count modulo width
        (op("i32.shl"), 1, 32, Ok(1)),
        (op("i32.shl"), 1, 33, Ok(2)),
        (op("i32.shl"), 1, 31, Ok(0x8000_0000)),
        (op("i32.shl"), 1, 0xFFFF_FFFF, Ok(0x8000_0000)),
        (op("i32.shl"), 0xFFFF_FFFF, 1, Ok(0xFFFF_FFFE)),
        (op("i64.shl"), 1, 64, Ok(1)),
        (op("i64.shl"), 1, 65, Ok(2)),
        (op("i64.shl"), 1, 63, Ok(0x8000_0000_0000_0000)),
        (op("i64.shl"), 1, 0xFFFF_FFFF_FFFF_FFC0, Ok(1)),
        // shr_s: arithmetic
        (op("i32.shr_s"), 0x8000_0000, 1, Ok(0xC000_0000)),
        (op("i32.shr_s"), 0xFFFF_FFFF, 31, Ok(0xFFFF_FFFF)),
        (op("i32.shr_s"), 0x8000_0000, 32, Ok(0x8000_0000)),
        (op("i32.shr_s"), 0x8000_0000, 33, Ok(0xC000_0000)),
        (op("i64.shr_s"), 0x8000_0000_0000_0000, 1, Ok(0xC000_0000_0000_0000)),
        (op("i64.shr_s"), u64::MAX, 63, Ok(u64::MAX)),
        (op("i64.shr_s"), u64::MAX, 64, Ok(u64::MAX)),
        (op("i64.shr_s"), 0x8000_0000_0000_0000, u64::MAX, Ok(u64::MAX)),
        // shr_u: logical
        (op("i32.shr_u"), 0x8000_0000, 1, Ok(0x4000_0000)),
        (op("i32.shr_u"), 0xFFFF_FFFF, 1, Ok(0x7FFF_FFFF)),
        (op("i32.shr_u"), 0xFFFF_FFFF, 33, Ok(0x7FFF_FFFF)),
        (op("i32.shr_u"), 0x8000_0000, 32, Ok(0x8000_0000)),
        (op("i64.shr_u"), u64::MAX, 1, Ok(0x7FFF_FFFF_FFFF_FFFF)),
        (op("i64.shr_u"), u64::MAX, 65, Ok(0x7FFF_FFFF_FFFF_FFFF)),
        (op("i64.shr_u"), 0x8000_0000_0000_0000, 63, Ok(1)),
    ]
}

fn rotate_cases() -> Vec<Case> {
    vec![
        (op("i32.rotl"), 0x8000_0001, 1, Ok(3)),
        (op("i32.rotl"), 0x1234_5678, 0, Ok(0x1234_5678)),
        (op("i32.rotl"), 0x1234_5678, 32, Ok(0x1234_5678)),
        (op("i32.rotl"), 0x1234_5678, 33, Ok(0x1234_5678u32.rotate_left(1) as u64)),
        (op("i32.rotl"), 0x1234_5678, 0xFFFF_FFFF_FFFF_FFE1, Ok(0x1234_5678u32.rotate_left(1) as u64)),
        (op("i32.rotr"), 0x8000_0001, 1, Ok(0xC000_0000)),
        (op("i32.rotr"), 0x1234_5678, 0, Ok(0x1234_5678)),
        (op("i32.rotr"), 0x1234_5678, 32, Ok(0x1234_5678)),
        (op("i32.rotr"), 0x1234_5678, 33, Ok(0x1234_5678u32.rotate_right(1) as u64)),
        (op("i32.rotr"), 0x1234_5678, 0xFFFF_FFFF_FFFF_FFE1, Ok(0x1234_5678u32.rotate_right(1) as u64)),
        (op("i64.rotl"), 0x8000_0000_0000_0001, 1, Ok(3)),
        (op("i64.rotl"), 0x0123_4567_89AB_CDEF, 0, Ok(0x0123_4567_89AB_CDEF)),
        (op("i64.rotl"), 0x0123_4567_89AB_CDEF, 64, Ok(0x0123_4567_89AB_CDEF)),
        (op("i64.rotl"), 0x0123_4567_89AB_CDEF, 65, Ok(0x0123_4567_89AB_CDEFu64.rotate_left(1))),
        (op("i64.rotl"), 0x0123_4567_89AB_CDEF, 0xFFFF_FFFF_FFFF_FFE1, Ok(0x0123_4567_89AB_CDEFu64.rotate_left(33))),
        (op("i64.rotr"), 0x8000_0000_0000_0001, 1, Ok(0xC000_0000_0000_0000)),
        (op("i64.rotr"), 0x0123_4567_89AB_CDEF, 0, Ok(0x0123_4567_89AB_CDEF)),
        (op("i64.rotr"), 0x0123_4567_89AB_CDEF, 64, Ok(0x0123_4567_89AB_CDEF)),
        (op("i64.rotr"), 0x0123_4567_89AB_CDEF, 65, Ok(0x0123_4567_89AB_CDEFu64.rotate_right(1))),
        (op("i64.rotr"), 0x0123_4567_89AB_CDEF, 0xFFFF_FFFF_FFFF_FFE1, Ok(0x0123_4567_89AB_CDEFu64.rotate_right(33))),
    ]
}

fn div_rem_cases() -> Vec<Case> {
    let min32 = 0x8000_0000u64;
    let min64 = 0x8000_0000_0000_0000u64;
    let neg7_32 = (-7i32 as u32) as u64;
    let neg7_64 = (-7i64) as u64;
    vec![
        // divisor 0 traps for all eight
        (op("i32.div_s"), 5, 0, Err(Trap::DivideByZero)),
        (op("i32.div_u"), 5, 0, Err(Trap::DivideByZero)),
        (op("i32.rem_s"), 5, 0, Err(Trap::DivideByZero)),
        (op("i32.rem_u"), 5, 0, Err(Trap::DivideByZero)),
        // only the low 32 bits of the divisor count for i32
        (op("i32.div_u"), 7, 0x1_0000_0000, Err(Trap::DivideByZero)),
        (op("i64.div_s"), 5, 0, Err(Trap::DivideByZero)),
        (op("i64.div_u"), 5, 0, Err(Trap::DivideByZero)),
        (op("i64.rem_s"), 5, 0, Err(Trap::DivideByZero)),
        (op("i64.rem_u"), 5, 0, Err(Trap::DivideByZero)),
        // MIN / -1 overflows for div_s
        (op("i32.div_s"), min32, 0xFFFF_FFFF, Err(Trap::IntegerOverflow)),
        (op("i64.div_s"), min64, u64::MAX, Err(Trap::IntegerOverflow)),
        // MIN % -1 = 0 for rem_s
        (op("i32.rem_s"), min32, 0xFFFF_FFFF, Ok(0)),
        (op("i64.rem_s"), min64, u64::MAX, Ok(0)),
        // unsigned: MIN / -1 is 0 / MIN % -1 is MIN, since -1 is the max value
        (op("i32.div_u"), min32, 0xFFFF_FFFF, Ok(0)),
        (op("i32.rem_u"), min32, 0xFFFF_FFFF, Ok(min32)),
        (op("i64.div_u"), min64, u64::MAX, Ok(0)),
        (op("i64.rem_u"), min64, u64::MAX, Ok(min64)),
        // signed division truncates toward zero
        (op("i32.div_s"), neg7_32, 2, Ok((-3i32 as u32) as u64)),
        (op("i32.rem_s"), neg7_32, 2, Ok((-1i32 as u32) as u64)),
        (op("i64.div_s"), neg7_64, 2, Ok((-3i64) as u64)),
        (op("i64.rem_s"), neg7_64, 2, Ok((-1i64) as u64)),
        (op("i32.div_s"), 0xFFFF_FFFF, 2, Ok(0)),
        (op("i32.rem_s"), 0xFFFF_FFFF, 2, Ok(0xFFFF_FFFF)),
        // unsigned treats -1 as the maximum value
        (op("i32.div_u"), 0xFFFF_FFFF, 2, Ok(0x7FFF_FFFF)),
        (op("i32.rem_u"), 0xFFFF_FFFF, 2, Ok(1)),
        (op("i64.div_u"), u64::MAX, 2, Ok(0x7FFF_FFFF_FFFF_FFFF)),
        (op("i64.rem_u"), u64::MAX, 2, Ok(1)),
    ]
}

fn comparison_cases() -> Vec<Case> {
    vec![
        (op("i32.eq"), 5, 5, Ok(1)),
        (op("i32.eq"), 5, 6, Ok(0)),
        (op("i32.ne"), 5, 5, Ok(0)),
        (op("i32.ne"), 5, 6, Ok(1)),
        // -1 vs 1: signed says less, unsigned says greater
        (op("i32.lt_s"), 0xFFFF_FFFF, 1, Ok(1)),
        (op("i32.lt_u"), 0xFFFF_FFFF, 1, Ok(0)),
        (op("i32.gt_s"), 1, 0xFFFF_FFFF, Ok(1)),
        (op("i32.gt_u"), 1, 0xFFFF_FFFF, Ok(0)),
        // boundaries
        (op("i32.lt_s"), 0x8000_0000, 0x7FFF_FFFF, Ok(1)),
        (op("i32.lt_u"), 0x8000_0000, 0x7FFF_FFFF, Ok(0)),
        (op("i32.le_s"), 7, 7, Ok(1)),
        (op("i32.le_u"), 0xFFFF_FFFF, 0xFFFF_FFFF, Ok(1)),
        (op("i32.le_s"), 0x8000_0000, 0x8000_0000, Ok(1)),
        (op("i32.le_u"), 0xFFFF_FFFF, 0, Ok(0)),
        (op("i32.ge_s"), 0, 0xFFFF_FFFF, Ok(1)),
        (op("i32.ge_u"), 0, 0xFFFF_FFFF, Ok(0)),
        (op("i32.ge_s"), 0x8000_0000, 0x7FFF_FFFF, Ok(0)),
        (op("i32.ge_u"), 0x8000_0000, 0x7FFF_FFFF, Ok(1)),
        (op("i64.eq"), 5, 5, Ok(1)),
        (op("i64.eq"), 5, 6, Ok(0)),
        (op("i64.ne"), 5, 5, Ok(0)),
        (op("i64.ne"), 5, 6, Ok(1)),
        (op("i64.lt_s"), u64::MAX, 1, Ok(1)),
        (op("i64.lt_u"), u64::MAX, 1, Ok(0)),
        (op("i64.gt_s"), 1, u64::MAX, Ok(1)),
        (op("i64.gt_u"), 1, u64::MAX, Ok(0)),
        (op("i64.lt_s"), 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, Ok(1)),
        (op("i64.lt_u"), 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, Ok(0)),
        (op("i64.le_s"), u64::MAX, u64::MAX, Ok(1)),
        (op("i64.le_u"), u64::MAX, u64::MAX, Ok(1)),
        (op("i64.le_u"), u64::MAX, 0, Ok(0)),
        (op("i64.ge_s"), 0, u64::MAX, Ok(1)),
        (op("i64.ge_u"), 0, u64::MAX, Ok(0)),
        (op("i64.ge_s"), 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, Ok(0)),
        (op("i64.ge_u"), 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, Ok(1)),
    ]
}

fn unary_cases() -> Vec<UnCase> {
    vec![
        (op("i32.clz"), 0, Ok(32)),
        (op("i64.clz"), 0, Ok(64)),
        (op("i32.ctz"), 0, Ok(32)),
        (op("i64.ctz"), 0, Ok(64)),
        (op("i32.popcnt"), 0, Ok(0)),
        (op("i64.popcnt"), 0, Ok(0)),
        (op("i32.popcnt"), 0xFFFF_FFFF, Ok(32)),
        (op("i64.popcnt"), u64::MAX, Ok(64)),
        (op("i32.clz"), 1, Ok(31)),
        (op("i64.clz"), 1, Ok(63)),
        (op("i64.clz"), u64::MAX, Ok(0)),
        (op("i32.ctz"), 0x8000_0000, Ok(31)),
        (op("i64.ctz"), 1 << 40, Ok(40)),
        (op("i64.popcnt"), 1, Ok(1)),
        // i32 only looks at the low 32 bits
        (op("i32.clz"), 0x1_0000_0000, Ok(32)),
        (op("i32.ctz"), 0x1_0000_0000, Ok(32)),
        (op("i32.popcnt"), 0x1_0000_0001, Ok(1)),
        (op("i32.clz"), 0xDEAD_BEEF_0000_1000, Ok(19)),
        (op("i32.ctz"), 0xDEAD_BEEF_0000_1000, Ok(12)),
        (op("i32.eqz"), 0, Ok(1)),
        (op("i32.eqz"), 1, Ok(0)),
        (op("i32.eqz"), 0xFFFF_FFFF, Ok(0)),
        (op("i32.eqz"), 0x1_0000_0000, Ok(1)),
        (op("i64.eqz"), 0, Ok(1)),
        (op("i64.eqz"), 1, Ok(0)),
        (op("i64.eqz"), u64::MAX, Ok(0)),
        (op("i64.eqz"), 0x1_0000_0000, Ok(0)),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn wrapping_arithmetic() {
    check_bin(wrapping_cases());
}

#[test]
fn bitwise_ops() {
    check_bin(bitwise_cases());
}

#[test]
fn shifts_use_count_modulo_width() {
    check_bin(shift_cases());
}

#[test]
fn rotates_use_count_modulo_width() {
    check_bin(rotate_cases());
}

#[test]
fn div_rem_traps_and_sign() {
    check_bin(div_rem_cases());
}

#[test]
fn comparisons_are_zero_or_one() {
    check_bin(comparison_cases());
}

#[test]
fn clz_ctz_popcnt_eqz() {
    check_un(unary_cases());
}

#[test]
fn i32_ops_ignore_upper_bits_of_inputs() {
    assert_eq!(bin(op("i32.add"), 0xDEAD_BEEF_0000_0001, 2), Ok(3));
    for name in BIN_MNEMONICS.iter().map(|m| format!("i32.{m}")) {
        let o = op(&name);
        let clean = bin(o, 1, 2);
        let dirty_a = bin(o, 0xDEAD_BEEF_0000_0001, 2);
        let dirty_b = bin(o, 1, 0xCAFE_0000_0000_0002);
        let dirty_both = bin(o, 0xDEAD_BEEF_0000_0001, 0xCAFE_0000_0000_0002);
        assert_eq!(dirty_a, clean, "{name} with dirty a");
        assert_eq!(dirty_b, clean, "{name} with dirty b");
        assert_eq!(dirty_both, clean, "{name} with dirty a and b");
    }
    for name in UN_MNEMONICS.iter().map(|m| format!("i32.{m}")) {
        let o = op(&name);
        assert_eq!(un(o, 0x1_0000_0000 | 0x1234), un(o, 0x1234), "{name} with dirty a");
    }
}

#[test]
fn i32_results_have_zero_upper_bits() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for name in BIN_MNEMONICS.iter().map(|m| format!("i32.{m}")) {
        let o = op(&name);
        for i in 0..50 {
            let (a, b) = (rng.operand(i), rng.operand(i + 1));
            if let Ok(v) = bin(o, a, b) {
                assert_eq!(v >> 32, 0, "{name} a={a:#x} b={b:#x} gave {v:#x}");
            }
        }
    }
    for name in UN_MNEMONICS.iter().map(|m| format!("i32.{m}")) {
        let o = op(&name);
        for i in 0..50 {
            let a = rng.operand(i);
            let v = un(o, a).expect("unary ops never trap");
            assert_eq!(v >> 32, 0, "{name} a={a:#x} gave {v:#x}");
        }
    }
}

#[test]
fn random_binary_ops_match_reference() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for m in BIN_MNEMONICS {
        for ty in ["i32", "i64"] {
            let name = format!("{ty}.{m}");
            let o = op(&name);
            for i in 0..200 {
                let (a, b) = (rng.operand(i), rng.operand(i + 1));
                let got = bin(o, a, b);
                let want = reference_bin(&name, a, b);
                assert_eq!(got, want, "{name} a={a:#x} b={b:#x}");
            }
        }
    }
}

#[test]
fn random_unary_ops_match_reference() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for m in UN_MNEMONICS {
        for ty in ["i32", "i64"] {
            let name = format!("{ty}.{m}");
            let o = op(&name);
            for i in 0..200 {
                let a = rng.operand(i);
                let got = un(o, a);
                let want = Ok(reference_un(&name, a));
                assert_eq!(got, want, "{name} a={a:#x}");
            }
        }
    }
}

/// Every i32./i64. op whose mnemonic is in section 5's integer list appears in a case table.
#[test]
fn every_integer_op_is_covered() {
    let expected: BTreeSet<&str> = Op::ALL
        .iter()
        .map(|o| o.name())
        .filter(|name| {
            name.split_once('.')
                .is_some_and(|(ty, m)| (ty == "i32" || ty == "i64") && MNEMONICS.contains(&m))
        })
        .collect();
    assert_eq!(expected.len(), 58, "expected 29 mnemonics for each of i32 and i64");

    let mut covered: BTreeSet<&str> = BTreeSet::new();
    for (o, ..) in wrapping_cases()
        .into_iter()
        .chain(bitwise_cases())
        .chain(shift_cases())
        .chain(rotate_cases())
        .chain(div_rem_cases())
        .chain(comparison_cases())
    {
        covered.insert(o.name());
    }
    for (o, ..) in unary_cases() {
        covered.insert(o.name());
    }
    assert_eq!(covered, expected);
}
