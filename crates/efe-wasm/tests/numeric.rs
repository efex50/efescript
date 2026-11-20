//! Numeric instructions: every operator against a Rust model, on the interpreter and the JIT.
mod common;
use common::*;
use efevm::Trap;
use std::fmt::Write;

const I32S: &[u32] = &[0, 1, 2, 3, 7, 31, 32, 33, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0xffff_fffe, 0xffff_ffff, 0x1234_5678];
const I64S: &[u64] = &[
    0, 1, 2, 3, 31, 32, 63, 64, 65, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0x1_0000_0000,
    0x7fff_ffff_ffff_ffff, 0x8000_0000_0000_0000, 0x8000_0000_0000_0001, 0xffff_ffff_ffff_fffe, u64::MAX, 0x0123_4567_89ab_cdef,
];
const F64S: &[f64] = &[
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 0.1, 1e10, -1e10, 1e300, 5e-324, f64::MIN_POSITIVE, f64::MAX, f64::MIN, f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
    2147483647.0, 2147483648.0, -2147483648.0, -2147483649.0, 4294967295.0, 4294967296.0, 9223372036854775807.0, 9223372036854775808.0, -9223372036854775808.0, 18446744073709551615.0, 18446744073709551616.0, -0.9, 0.9,
];
const F32S: &[f32] = &[
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 0.1, 1e10, -1e10, 1e30, 1e-45, f32::MIN_POSITIVE, f32::MAX, f32::MIN, f32::NAN, f32::INFINITY, f32::NEG_INFINITY,
    2147483648.0, -2147483648.0, 4294967296.0, 9223372036854775808.0, -9223372036854775808.0, 18446744073709551616.0, -0.9, 0.9, 16777216.0, 16777217.0,
];

fn nan_eq32(a: u64, b: u64) -> bool {
    a == b || (f32::from_bits(a as u32).is_nan() && f32::from_bits(b as u32).is_nan())
}
fn nan_eq64(a: u64, b: u64) -> bool {
    a == b || (f64::from_bits(a).is_nan() && f64::from_bits(b).is_nan())
}

fn wasm_min<T: Copy + PartialOrd + std::ops::Neg<Output = T>>(a: T, b: T, nan: T, is_nan: impl Fn(T) -> bool, neg: impl Fn(T) -> bool) -> T {
    if is_nan(a) || is_nan(b) {
        nan
    } else if a == b {
        if neg(a) { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn wasm_max<T: Copy + PartialOrd>(a: T, b: T, nan: T, is_nan: impl Fn(T) -> bool, neg: impl Fn(T) -> bool) -> T {
    if is_nan(a) || is_nan(b) {
        nan
    } else if a == b {
        if neg(a) { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}

type Bin<T> = (&'static str, Box<dyn Fn(T, T) -> Result<T, Trap>>);

fn i32_bins() -> Vec<Bin<u32>> {
    let s = |x: u32| x as i32;
    vec![
        ("add", Box::new(|a, b| Ok(a.wrapping_add(b)))),
        ("sub", Box::new(|a, b| Ok(a.wrapping_sub(b)))),
        ("mul", Box::new(|a, b| Ok(a.wrapping_mul(b)))),
        ("div_s", Box::new(move |a, b| if b == 0 { Err(Trap::DivideByZero) } else if s(a) == i32::MIN && s(b) == -1 { Err(Trap::IntegerOverflow) } else { Ok((s(a) / s(b)) as u32) })),
        ("div_u", Box::new(|a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(a / b) })),
        ("rem_s", Box::new(move |a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(s(a).wrapping_rem(s(b)) as u32) })),
        ("rem_u", Box::new(|a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(a % b) })),
        ("and", Box::new(|a, b| Ok(a & b))),
        ("or", Box::new(|a, b| Ok(a | b))),
        ("xor", Box::new(|a, b| Ok(a ^ b))),
        ("shl", Box::new(|a, b| Ok(a.wrapping_shl(b)))),
        ("shr_s", Box::new(move |a, b| Ok(s(a).wrapping_shr(b) as u32))),
        ("shr_u", Box::new(|a, b| Ok(a.wrapping_shr(b)))),
        ("rotl", Box::new(|a, b| Ok(a.rotate_left(b % 32)))),
        ("rotr", Box::new(|a, b| Ok(a.rotate_right(b % 32)))),
        ("eq", Box::new(|a, b| Ok((a == b) as u32))),
        ("ne", Box::new(|a, b| Ok((a != b) as u32))),
        ("lt_s", Box::new(move |a, b| Ok((s(a) < s(b)) as u32))),
        ("lt_u", Box::new(|a, b| Ok((a < b) as u32))),
        ("gt_s", Box::new(move |a, b| Ok((s(a) > s(b)) as u32))),
        ("gt_u", Box::new(|a, b| Ok((a > b) as u32))),
        ("le_s", Box::new(move |a, b| Ok((s(a) <= s(b)) as u32))),
        ("le_u", Box::new(|a, b| Ok((a <= b) as u32))),
        ("ge_s", Box::new(move |a, b| Ok((s(a) >= s(b)) as u32))),
        ("ge_u", Box::new(|a, b| Ok((a >= b) as u32))),
    ]
}

fn i64_bins() -> Vec<Bin<u64>> {
    let s = |x: u64| x as i64;
    vec![
        ("add", Box::new(|a, b| Ok(a.wrapping_add(b)))),
        ("sub", Box::new(|a, b| Ok(a.wrapping_sub(b)))),
        ("mul", Box::new(|a, b| Ok(a.wrapping_mul(b)))),
        ("div_s", Box::new(move |a, b| if b == 0 { Err(Trap::DivideByZero) } else if s(a) == i64::MIN && s(b) == -1 { Err(Trap::IntegerOverflow) } else { Ok((s(a) / s(b)) as u64) })),
        ("div_u", Box::new(|a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(a / b) })),
        ("rem_s", Box::new(move |a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(s(a).wrapping_rem(s(b)) as u64) })),
        ("rem_u", Box::new(|a, b| if b == 0 { Err(Trap::DivideByZero) } else { Ok(a % b) })),
        ("and", Box::new(|a, b| Ok(a & b))),
        ("or", Box::new(|a, b| Ok(a | b))),
        ("xor", Box::new(|a, b| Ok(a ^ b))),
        ("shl", Box::new(|a, b| Ok(a.wrapping_shl(b as u32)))),
        ("shr_s", Box::new(move |a, b| Ok(s(a).wrapping_shr(b as u32) as u64))),
        ("shr_u", Box::new(|a, b| Ok(a.wrapping_shr(b as u32)))),
        ("rotl", Box::new(|a, b| Ok(a.rotate_left((b % 64) as u32)))),
        ("rotr", Box::new(|a, b| Ok(a.rotate_right((b % 64) as u32)))),
        ("eq", Box::new(|a, b| Ok((a == b) as u64))),
        ("ne", Box::new(|a, b| Ok((a != b) as u64))),
        ("lt_s", Box::new(move |a, b| Ok((s(a) < s(b)) as u64))),
        ("lt_u", Box::new(|a, b| Ok((a < b) as u64))),
        ("gt_s", Box::new(move |a, b| Ok((s(a) > s(b)) as u64))),
        ("gt_u", Box::new(|a, b| Ok((a > b) as u64))),
        ("le_s", Box::new(move |a, b| Ok((s(a) <= s(b)) as u64))),
        ("le_u", Box::new(|a, b| Ok((a <= b) as u64))),
        ("ge_s", Box::new(move |a, b| Ok((s(a) >= s(b)) as u64))),
        ("ge_u", Box::new(|a, b| Ok((a >= b) as u64))),
    ]
}

const CMP: &[&str] = &["eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u"];

#[test]
fn i32_binary() {
    let ops = i32_bins();
    let mut wat = String::from("(module\n");
    for (n, _) in &ops {
        let rt = if CMP.contains(n) { "i32" } else { "i32" };
        writeln!(wat, "(func (export \"{n}\") (param i32 i32) (result {rt}) local.get 0 local.get 1 i32.{n})").unwrap();
    }
    wat.push(')');
    let mut p = Pair::new(&tr_wat(&wat));
    for (n, f) in &ops {
        for &a in I32S {
            for &b in I32S {
                let got = p.call(n, &[a as u64, b as u64]);
                let want = f(a, b).map(|v| vec![v as u64]);
                assert_eq!(got, want, "i32.{n}({a:#x}, {b:#x})");
            }
        }
    }
}

#[test]
fn i64_binary() {
    let ops = i64_bins();
    let mut wat = String::from("(module\n");
    for (n, _) in &ops {
        let rt = if CMP.contains(n) { "i32" } else { "i64" };
        writeln!(wat, "(func (export \"{n}\") (param i64 i64) (result {rt}) local.get 0 local.get 1 i64.{n})").unwrap();
    }
    wat.push(')');
    let mut p = Pair::new(&tr_wat(&wat));
    for (n, f) in &ops {
        for &a in I64S {
            for &b in I64S {
                let got = p.call(n, &[a, b]);
                let want = f(a, b).map(|v| vec![v]);
                assert_eq!(got, want, "i64.{n}({a:#x}, {b:#x})");
            }
        }
    }
}

#[test]
fn integer_unary_and_extensions() {
    let wat = "(module
      (func (export \"i32.clz\") (param i32) (result i32) local.get 0 i32.clz)
      (func (export \"i32.ctz\") (param i32) (result i32) local.get 0 i32.ctz)
      (func (export \"i32.popcnt\") (param i32) (result i32) local.get 0 i32.popcnt)
      (func (export \"i32.eqz\") (param i32) (result i32) local.get 0 i32.eqz)
      (func (export \"i32.extend8_s\") (param i32) (result i32) local.get 0 i32.extend8_s)
      (func (export \"i32.extend16_s\") (param i32) (result i32) local.get 0 i32.extend16_s)
      (func (export \"i64.clz\") (param i64) (result i64) local.get 0 i64.clz)
      (func (export \"i64.ctz\") (param i64) (result i64) local.get 0 i64.ctz)
      (func (export \"i64.popcnt\") (param i64) (result i64) local.get 0 i64.popcnt)
      (func (export \"i64.eqz\") (param i64) (result i32) local.get 0 i64.eqz)
      (func (export \"i64.extend8_s\") (param i64) (result i64) local.get 0 i64.extend8_s)
      (func (export \"i64.extend16_s\") (param i64) (result i64) local.get 0 i64.extend16_s)
      (func (export \"i64.extend32_s\") (param i64) (result i64) local.get 0 i64.extend32_s)
      (func (export \"i32.wrap_i64\") (param i64) (result i32) local.get 0 i32.wrap_i64)
      (func (export \"i64.extend_i32_s\") (param i32) (result i64) local.get 0 i64.extend_i32_s)
      (func (export \"i64.extend_i32_u\") (param i32) (result i64) local.get 0 i64.extend_i32_u)
    )";
    let mut p = Pair::new(&tr_wat(wat));
    for &a in I32S {
        assert_eq!(p.call1("i32.clz", &[a as u64]), a.leading_zeros() as u64);
        assert_eq!(p.call1("i32.ctz", &[a as u64]), a.trailing_zeros() as u64);
        assert_eq!(p.call1("i32.popcnt", &[a as u64]), a.count_ones() as u64);
        assert_eq!(p.call1("i32.eqz", &[a as u64]), (a == 0) as u64);
        assert_eq!(p.call1("i32.extend8_s", &[a as u64]), (a as i8 as i32 as u32) as u64, "{a:#x}");
        assert_eq!(p.call1("i32.extend16_s", &[a as u64]), (a as i16 as i32 as u32) as u64);
        assert_eq!(p.call1("i64.extend_i32_s", &[a as u64]), a as i32 as i64 as u64);
        assert_eq!(p.call1("i64.extend_i32_u", &[a as u64]), a as u64);
    }
    for &a in I64S {
        assert_eq!(p.call1("i64.clz", &[a]), a.leading_zeros() as u64);
        assert_eq!(p.call1("i64.ctz", &[a]), a.trailing_zeros() as u64);
        assert_eq!(p.call1("i64.popcnt", &[a]), a.count_ones() as u64);
        assert_eq!(p.call1("i64.eqz", &[a]), (a == 0) as u64);
        assert_eq!(p.call1("i64.extend8_s", &[a]), a as i8 as i64 as u64);
        assert_eq!(p.call1("i64.extend16_s", &[a]), a as i16 as i64 as u64);
        assert_eq!(p.call1("i64.extend32_s", &[a]), a as i32 as i64 as u64);
        assert_eq!(p.call1("i32.wrap_i64", &[a]), a as u32 as u64);
    }
}

macro_rules! float_tests {
    ($name:ident, $t:ty, $bits:ty, $ty:literal, $vals:expr, $nan_eq:ident, $from:expr, $to:expr) => {
        #[test]
        fn $name() {
            let bin = ["add", "sub", "mul", "div", "min", "max", "copysign"];
            let cmp = ["eq", "ne", "lt", "gt", "le", "ge"];
            let un = ["abs", "neg", "sqrt", "ceil", "floor", "trunc", "nearest"];
            let mut wat = String::from("(module\n");
            for n in bin {
                writeln!(wat, "(func (export \"{n}\") (param {t} {t}) (result {t}) local.get 0 local.get 1 {t}.{n})", t = $ty).unwrap();
            }
            for n in cmp {
                writeln!(wat, "(func (export \"{n}\") (param {t} {t}) (result i32) local.get 0 local.get 1 {t}.{n})", t = $ty).unwrap();
            }
            for n in un {
                writeln!(wat, "(func (export \"{n}\") (param {t}) (result {t}) local.get 0 {t}.{n})", t = $ty).unwrap();
            }
            wat.push(')');
            let mut p = Pair::new(&tr_wat(&wat));
            let from = $from;
            let to = $to;
            for &a in $vals {
                for &b in $vals {
                    let (ab, bb) = (to(a), to(b));
                    let chk = |n: &str, want: $t, got: u64| {
                        assert!($nan_eq(to(want), got), "{}.{n}({a:?}, {b:?}): got {:?}, want {want:?}", $ty, from(got));
                    };
                    chk("add", a + b, p.call1("add", &[ab, bb]));
                    chk("sub", a - b, p.call1("sub", &[ab, bb]));
                    chk("mul", a * b, p.call1("mul", &[ab, bb]));
                    chk("div", a / b, p.call1("div", &[ab, bb]));
                    let nan = <$t>::NAN;
                    chk("min", wasm_min(a, b, nan, |x| x.is_nan(), |x| x.is_sign_negative()), p.call1("min", &[ab, bb]));
                    chk("max", wasm_max(a, b, nan, |x| x.is_nan(), |x| x.is_sign_negative()), p.call1("max", &[ab, bb]));
                    chk("copysign", a.copysign(b), p.call1("copysign", &[ab, bb]));
                    assert_eq!(p.call1("eq", &[ab, bb]), (a == b) as u64);
                    assert_eq!(p.call1("ne", &[ab, bb]), (a != b) as u64);
                    assert_eq!(p.call1("lt", &[ab, bb]), (a < b) as u64);
                    assert_eq!(p.call1("gt", &[ab, bb]), (a > b) as u64);
                    assert_eq!(p.call1("le", &[ab, bb]), (a <= b) as u64);
                    assert_eq!(p.call1("ge", &[ab, bb]), (a >= b) as u64);
                }
                let ab = to(a);
                let chk = |n: &str, want: $t, got: u64| {
                    assert!($nan_eq(to(want), got), "{}.{n}({a:?}): got {:?}, want {want:?}", $ty, from(got));
                };
                chk("abs", a.abs(), p.call1("abs", &[ab]));
                chk("neg", -a, p.call1("neg", &[ab]));
                chk("sqrt", a.sqrt(), p.call1("sqrt", &[ab]));
                chk("ceil", a.ceil(), p.call1("ceil", &[ab]));
                chk("floor", a.floor(), p.call1("floor", &[ab]));
                chk("trunc", a.trunc(), p.call1("trunc", &[ab]));
                chk("nearest", a.round_ties_even(), p.call1("nearest", &[ab]));
            }
        }
    };
}

float_tests!(f32_ops, f32, u32, "f32", F32S, nan_eq32, |b: u64| f32::from_bits(b as u32), |v: f32| v.to_bits() as u64);
float_tests!(f64_ops, f64, u64, "f64", F64S, nan_eq64, f64::from_bits, |v: f64| v.to_bits());

/// Expected result of `iN.trunc_f_(s|u)` (or the saturating form) of the float value `x`.
fn trunc_expect(x: f64, signed: bool, bits: u32, sat: bool) -> Result<u64, Trap> {
    let (lo, hi) = if signed { (-(2f64.powi(bits as i32 - 1)), 2f64.powi(bits as i32 - 1)) } else { (0.0, 2f64.powi(bits as i32)) };
    let mask: i128 = (1i128 << bits) - 1;
    if x.is_nan() {
        return if sat { Ok(0) } else { Err(Trap::InvalidConversion) };
    }
    let t = x.trunc();
    if t >= lo && t < hi {
        return Ok(((t as i128) & mask) as u64);
    }
    if !sat {
        return Err(Trap::IntegerOverflow);
    }
    let v: i128 = if t < lo { lo as i128 } else { hi as i128 - 1 };
    Ok((v & mask) as u64)
}

#[test]
fn conversions() {
    let mut wat = String::from("(module\n");
    for (d, s) in [("i32", "f32"), ("i32", "f64"), ("i64", "f32"), ("i64", "f64")] {
        for sign in ["s", "u"] {
            writeln!(wat, "(func (export \"{d}.trunc_{s}_{sign}\") (param {s}) (result {d}) local.get 0 {d}.trunc_{s}_{sign})").unwrap();
            writeln!(wat, "(func (export \"{d}.trunc_sat_{s}_{sign}\") (param {s}) (result {d}) local.get 0 {d}.trunc_sat_{s}_{sign})").unwrap();
        }
    }
    for (d, s) in [("f32", "i32"), ("f32", "i64"), ("f64", "i32"), ("f64", "i64")] {
        for sign in ["s", "u"] {
            writeln!(wat, "(func (export \"{d}.convert_{s}_{sign}\") (param {s}) (result {d}) local.get 0 {d}.convert_{s}_{sign})").unwrap();
        }
    }
    wat.push_str(
        "(func (export \"demote\") (param f64) (result f32) local.get 0 f32.demote_f64)
         (func (export \"promote\") (param f32) (result f64) local.get 0 f64.promote_f32)
         (func (export \"i32.reinterpret_f32\") (param f32) (result i32) local.get 0 i32.reinterpret_f32)
         (func (export \"i64.reinterpret_f64\") (param f64) (result i64) local.get 0 i64.reinterpret_f64)
         (func (export \"f32.reinterpret_i32\") (param i32) (result f32) local.get 0 f32.reinterpret_i32)
         (func (export \"f64.reinterpret_i64\") (param i64) (result f64) local.get 0 f64.reinterpret_i64))",
    );
    let mut p = Pair::new(&tr_wat(&wat));

    // Truncation, trapping and saturating, from f32 and f64 into i32 and i64, signed and unsigned.
    for &v in F64S {
        for (d, bits) in [("i32", 32u32), ("i64", 64)] {
            for (sg, signed) in [("s", true), ("u", false)] {
                let name = format!("{d}.trunc_f64_{sg}");
                assert_eq!(p.call(&name, &[v.to_bits()]), trunc_expect(v, signed, bits, false).map(|x| vec![x]), "{name}({v:?})");
                let name = format!("{d}.trunc_sat_f64_{sg}");
                assert_eq!(p.call(&name, &[v.to_bits()]), trunc_expect(v, signed, bits, true).map(|x| vec![x]), "{name}({v:?})");
            }
        }
    }
    for &v in F32S {
        for (d, bits) in [("i32", 32u32), ("i64", 64)] {
            for (sg, signed) in [("s", true), ("u", false)] {
                let name = format!("{d}.trunc_f32_{sg}");
                assert_eq!(p.call(&name, &[v.to_bits() as u64]), trunc_expect(v as f64, signed, bits, false).map(|x| vec![x]), "{name}({v:?})");
                let name = format!("{d}.trunc_sat_f32_{sg}");
                assert_eq!(p.call(&name, &[v.to_bits() as u64]), trunc_expect(v as f64, signed, bits, true).map(|x| vec![x]), "{name}({v:?})");
            }
        }
        assert!(nan_eq64(p.call1("promote", &[v.to_bits() as u64]), (v as f64).to_bits()), "promote({v:?})");
        assert_eq!(p.call1("i32.reinterpret_f32", &[v.to_bits() as u64]), v.to_bits() as u64);
    }
    for &v in F64S {
        assert!(nan_eq32(p.call1("demote", &[v.to_bits()]), (v as f32).to_bits() as u64), "demote({v:?})");
        assert_eq!(p.call1("i64.reinterpret_f64", &[v.to_bits()]), v.to_bits());
    }
    for &a in I32S {
        assert_eq!(p.call1("f32.convert_i32_s", &[a as u64]), (a as i32 as f32).to_bits() as u64, "{a:#x}");
        assert_eq!(p.call1("f32.convert_i32_u", &[a as u64]), (a as f32).to_bits() as u64);
        assert_eq!(p.call1("f64.convert_i32_s", &[a as u64]), (a as i32 as f64).to_bits());
        assert_eq!(p.call1("f64.convert_i32_u", &[a as u64]), (a as f64).to_bits());
        assert_eq!(p.call1("f32.reinterpret_i32", &[a as u64]), a as u64);
    }
    for &a in I64S {
        assert_eq!(p.call1("f32.convert_i64_s", &[a]), (a as i64 as f32).to_bits() as u64, "{a:#x}");
        assert_eq!(p.call1("f32.convert_i64_u", &[a]), (a as f32).to_bits() as u64, "{a:#x}");
        assert_eq!(p.call1("f64.convert_i64_s", &[a]), (a as i64 as f64).to_bits());
        assert_eq!(p.call1("f64.convert_i64_u", &[a]), (a as f64).to_bits());
        assert_eq!(p.call1("f64.reinterpret_i64", &[a]), a);
    }
}
