//! Instruction tests for every float op and conversion op (SPEC.md section 5).

mod common;
use efevm::*;
use std::collections::HashSet;

#[derive(Clone, Debug)]
enum Exp {
    Bits(u64),
    Nan32,
    Nan64,
    Trap(Trap),
}

#[derive(Clone, Debug)]
struct Case {
    op: Op,
    args: Vec<u64>,
    exp: Exp,
}

fn op(name: &str) -> Op {
    *Op::ALL.iter().find(|o| o.name() == name).unwrap_or_else(|| panic!("no op {name}"))
}

fn exec(op: Op, args: &[u64]) -> Result<u64, Trap> {
    let n = args.len() as u16;
    let mut f = FunctionBuilder::new(n + 1);
    match n {
        1 => f.op2(op, 1, 0),
        2 => f.op3(op, 2, 0, 1),
        _ => unreachable!(),
    }
    f.ret(n);
    let mut mb = ModuleBuilder::new();
    let ty = FuncType::new(vec![ValType::I64; n as usize], [ValType::I64]);
    let idx = mb.add_func(ty, f);
    mb.export_func("main", idx);
    let module = mb.finish().expect("module");
    let mut inst = common::new_instance(&module, &Imports::<()>::new(), Limits::default()).unwrap();
    let fiber = inst.spawn("main", args).unwrap();
    match inst.resume(fiber, &mut (), 1_000_000) {
        Exit::Done => Ok(inst.results(fiber).unwrap()[0]),
        Exit::Trap(t) => Err(t),
        other => panic!("unexpected exit {other:?}"),
    }
}

/// Input width in bits of the (all) operands of `op`.
fn in_width(op: Op) -> u32 {
    let n = op.name();
    let (head, rest) = n.split_once('.').unwrap();
    match head {
        "f32" => {
            if rest.contains("demote") || rest.contains("_i64") {
                64
            } else {
                32
            }
        }
        "f64" => {
            if rest.contains("promote") || rest.contains("_i32") || rest.contains("_f32") {
                32
            } else {
                64
            }
        }
        "i32" => {
            if rest.contains("wrap") || rest.contains("_f64") || rest.contains("reinterpret_f64") {
                64
            } else {
                32
            }
        }
        "i64" if rest.contains("_i32") || rest.contains("_f32") => 32,
        _ => 64,
    }
}

fn check(c: &Case, args: &[u64], fails: &mut Vec<String>) {
    let got = exec(c.op, args);
    let ok = match (&c.exp, &got) {
        (Exp::Bits(b), Ok(g)) => b == g,
        (Exp::Nan32, Ok(g)) => g >> 32 == 0 && f32::from_bits(*g as u32).is_nan(),
        (Exp::Nan64, Ok(g)) => f64::from_bits(*g).is_nan(),
        (Exp::Trap(t), Err(g)) => t == g,
        _ => false,
    };
    if !ok {
        fails.push(format!(
            "{} args={:x?} expected {:?} got {:x?}",
            c.op.name(),
            args,
            c.exp,
            got
        ));
    }
}

fn run_cases(cases: &[Case]) {
    let mut fails = Vec::new();
    for c in cases {
        check(c, &c.args, &mut fails);
        if in_width(c.op) == 32 {
            let g: Vec<u64> = c.args.iter().map(|a| a | 0xDEAD_BEEF_0000_0000).collect();
            check(c, &g, &mut fails);
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

fn c1(v: &mut Vec<Case>, name: &str, a: u64, exp: Exp) {
    v.push(Case { op: op(name), args: vec![a], exp });
}
fn c2(v: &mut Vec<Case>, name: &str, a: u64, b: u64, exp: Exp) {
    v.push(Case { op: op(name), args: vec![a, b], exp });
}

// ---------------------------------------------------------------- float ops

fn float_cases(is32: bool) -> Vec<Case> {
    let p = if is32 { "f32" } else { "f64" };
    let bits = move |x: f64| -> u64 { if is32 { (x as f32).to_bits() as u64 } else { x.to_bits() } };
    let sign: u64 = if is32 { 1 << 31 } else { 1 << 63 };
    let nan_payload: u64 = if is32 { 0x7FC0_0001 } else { 0x7FF8_0000_0000_0001 };
    let pnan = nan_payload;
    let nnan = nan_payload | sign;
    let nan = if is32 { Exp::Nan32 } else { Exp::Nan64 };
    let nm = |n: &str| format!("{p}.{n}");
    let b = |x: u64| Exp::Bits(x);
    let nz = bits(-0.0);
    let pz = bits(0.0);
    let inf = bits(f64::INFINITY);
    let ninf = bits(f64::NEG_INFINITY);
    let mut v = Vec::new();

    // binary arithmetic
    let bin = |v: &mut Vec<Case>, n: &str, a: u64, bb: u64, e: Exp| c2(v, &nm(n), a, bb, e);
    bin(&mut v, "add", bits(1.5), bits(2.25), b(bits(3.75)));
    bin(&mut v, "add", inf, ninf, nan.clone());
    bin(&mut v, "add", pnan, bits(1.0), nan.clone());
    bin(&mut v, "add", bits(1.0), nnan, nan.clone());
    bin(&mut v, "add", nz, nz, b(nz));
    bin(&mut v, "sub", bits(5.0), bits(7.5), b(bits(-2.5)));
    bin(&mut v, "sub", inf, inf, nan.clone());
    bin(&mut v, "sub", pnan, pnan, nan.clone());
    bin(&mut v, "mul", bits(3.0), bits(-0.5), b(bits(-1.5)));
    bin(&mut v, "mul", pz, inf, nan.clone());
    bin(&mut v, "mul", nz, bits(1.0), b(nz));
    bin(&mut v, "mul", nnan, bits(2.0), nan.clone());
    bin(&mut v, "div", bits(7.5), bits(2.5), b(bits(3.0)));
    bin(&mut v, "div", bits(1.0), pz, b(inf));
    bin(&mut v, "div", bits(-1.0), pz, b(ninf));
    bin(&mut v, "div", bits(1.0), nz, b(ninf));
    bin(&mut v, "div", pz, pz, nan.clone());
    bin(&mut v, "div", inf, inf, nan.clone());
    bin(&mut v, "div", pnan, bits(1.0), nan.clone());

    // min / max
    bin(&mut v, "min", bits(1.0), bits(2.0), b(bits(1.0)));
    bin(&mut v, "min", bits(2.0), bits(1.0), b(bits(1.0)));
    bin(&mut v, "min", nz, pz, b(nz));
    bin(&mut v, "min", pz, nz, b(nz));
    bin(&mut v, "min", pz, pz, b(pz));
    bin(&mut v, "min", ninf, bits(1.0), b(ninf));
    bin(&mut v, "min", pnan, bits(1.0), nan.clone());
    bin(&mut v, "min", bits(1.0), pnan, nan.clone());
    bin(&mut v, "min", nnan, ninf, nan.clone());
    bin(&mut v, "min", ninf, nnan, nan.clone());
    bin(&mut v, "max", bits(1.0), bits(2.0), b(bits(2.0)));
    bin(&mut v, "max", bits(2.0), bits(1.0), b(bits(2.0)));
    bin(&mut v, "max", nz, pz, b(pz));
    bin(&mut v, "max", pz, nz, b(pz));
    bin(&mut v, "max", nz, nz, b(nz));
    bin(&mut v, "max", inf, bits(1.0), b(inf));
    bin(&mut v, "max", pnan, bits(1.0), nan.clone());
    bin(&mut v, "max", bits(1.0), pnan, nan.clone());
    bin(&mut v, "max", nnan, inf, nan.clone());
    bin(&mut v, "max", inf, nnan, nan.clone());

    // copysign
    bin(&mut v, "copysign", bits(1.5), nz, b(bits(-1.5)));
    bin(&mut v, "copysign", bits(-1.5), pz, b(bits(1.5)));
    bin(&mut v, "copysign", bits(1.5), nnan, b(bits(-1.5)));
    bin(&mut v, "copysign", bits(-1.5), pnan, b(bits(1.5)));
    bin(&mut v, "copysign", pz, nz, b(nz));
    bin(&mut v, "copysign", nz, pz, b(pz));
    bin(&mut v, "copysign", inf, bits(-3.0), b(ninf));
    bin(&mut v, "copysign", pnan, bits(-3.0), b(nnan));
    bin(&mut v, "copysign", nnan, bits(3.0), b(pnan));

    // unary
    let un = |v: &mut Vec<Case>, n: &str, a: u64, e: Exp| c1(v, &nm(n), a, e);
    un(&mut v, "abs", bits(-1.5), b(bits(1.5)));
    un(&mut v, "abs", bits(1.5), b(bits(1.5)));
    un(&mut v, "abs", nz, b(pz));
    un(&mut v, "abs", ninf, b(inf));
    un(&mut v, "abs", nnan, b(pnan));
    un(&mut v, "abs", pnan, b(pnan));
    un(&mut v, "neg", bits(1.5), b(bits(-1.5)));
    un(&mut v, "neg", bits(-1.5), b(bits(1.5)));
    un(&mut v, "neg", pz, b(nz));
    un(&mut v, "neg", nz, b(pz));
    un(&mut v, "neg", inf, b(ninf));
    un(&mut v, "neg", pnan, b(nnan));
    un(&mut v, "neg", nnan, b(pnan));
    un(&mut v, "sqrt", bits(4.0), b(bits(2.0)));
    un(&mut v, "sqrt", bits(2.25), b(bits(1.5)));
    un(&mut v, "sqrt", bits(-1.0), nan.clone());
    un(&mut v, "sqrt", ninf, nan.clone());
    un(&mut v, "sqrt", nz, b(nz));
    un(&mut v, "sqrt", pz, b(pz));
    un(&mut v, "sqrt", inf, b(inf));
    un(&mut v, "sqrt", pnan, nan.clone());

    // rounding
    let table: &[(&str, &[(f64, f64)])] = &[
        ("ceil", &[(0.5, 1.0), (-0.5, -0.0), (1.5, 2.0), (-1.5, -1.0), (-0.0, -0.0), (0.0, 0.0), (2.0, 2.0), (-2.0, -2.0), (0.1, 1.0), (-0.9, -0.0)]),
        ("floor", &[(0.5, 0.0), (-0.5, -1.0), (1.5, 1.0), (-1.5, -2.0), (-0.0, -0.0), (0.0, 0.0), (2.0, 2.0), (0.9, 0.0), (-0.1, -1.0)]),
        ("trunc", &[(0.5, 0.0), (-0.5, -0.0), (1.5, 1.0), (-1.5, -1.0), (-0.0, -0.0), (0.0, 0.0), (2.9, 2.0), (-2.9, -2.0)]),
        ("nearest", &[(0.5, 0.0), (1.5, 2.0), (2.5, 2.0), (3.5, 4.0), (-0.5, -0.0), (-1.5, -2.0), (-2.5, -2.0), (-0.0, -0.0), (0.0, 0.0), (0.75, 1.0), (-0.25, -0.0), (2.6, 3.0), (-2.6, -3.0)]),
    ];
    for (n, rows) in table {
        for &(a, e) in rows.iter() {
            un(&mut v, n, bits(a), b(bits(e)));
        }
        un(&mut v, n, inf, b(inf));
        un(&mut v, n, ninf, b(ninf));
        un(&mut v, n, pnan, nan.clone());
        un(&mut v, n, nnan, nan.clone());
    }

    // comparisons
    let t = b(1);
    let f = b(0);
    let (one, two) = (bits(1.0), bits(2.0));
    bin(&mut v, "eq", one, one, t.clone());
    bin(&mut v, "eq", one, two, f.clone());
    bin(&mut v, "eq", nz, pz, t.clone());
    bin(&mut v, "eq", pnan, pnan, f.clone());
    bin(&mut v, "eq", pnan, one, f.clone());
    bin(&mut v, "eq", one, nnan, f.clone());
    bin(&mut v, "eq", inf, inf, t.clone());
    bin(&mut v, "ne", one, one, f.clone());
    bin(&mut v, "ne", one, two, t.clone());
    bin(&mut v, "ne", nz, pz, f.clone());
    bin(&mut v, "ne", pnan, pnan, t.clone());
    bin(&mut v, "ne", pnan, one, t.clone());
    bin(&mut v, "ne", one, nnan, t.clone());
    bin(&mut v, "lt", one, two, t.clone());
    bin(&mut v, "lt", two, one, f.clone());
    bin(&mut v, "lt", one, one, f.clone());
    bin(&mut v, "lt", nz, pz, f.clone());
    bin(&mut v, "lt", ninf, inf, t.clone());
    bin(&mut v, "lt", pnan, one, f.clone());
    bin(&mut v, "lt", one, pnan, f.clone());
    bin(&mut v, "gt", two, one, t.clone());
    bin(&mut v, "gt", one, two, f.clone());
    bin(&mut v, "gt", one, one, f.clone());
    bin(&mut v, "gt", pz, nz, f.clone());
    bin(&mut v, "gt", pnan, one, f.clone());
    bin(&mut v, "gt", one, pnan, f.clone());
    bin(&mut v, "le", one, two, t.clone());
    bin(&mut v, "le", two, one, f.clone());
    bin(&mut v, "le", one, one, t.clone());
    bin(&mut v, "le", nz, pz, t.clone());
    bin(&mut v, "le", pnan, one, f.clone());
    bin(&mut v, "le", one, pnan, f.clone());
    bin(&mut v, "le", pnan, pnan, f.clone());
    bin(&mut v, "ge", two, one, t.clone());
    bin(&mut v, "ge", one, two, f.clone());
    bin(&mut v, "ge", one, one, t.clone());
    bin(&mut v, "ge", pz, nz, t.clone());
    bin(&mut v, "ge", pnan, one, f.clone());
    bin(&mut v, "ge", one, pnan, f.clone());
    bin(&mut v, "ge", pnan, pnan, f);
    v
}

#[test]
fn f32_ops() {
    run_cases(&float_cases(true));
}

#[test]
fn f64_ops() {
    run_cases(&float_cases(false));
}

#[test]
fn f32_results_have_zero_upper_bits_and_ignore_input_upper() {
    // explicit: garbage in the upper half of both inputs must not matter
    let g = 0xFFFF_FFFF_0000_0000u64;
    let a = 1.5f32.to_bits() as u64 | g;
    let b = 2.25f32.to_bits() as u64 | g;
    assert_eq!(exec(op("f32.add"), &[a, b]), Ok(3.75f32.to_bits() as u64));
    assert_eq!(exec(op("f32.lt"), &[a, b]), Ok(1));
    assert_eq!(exec(op("f32.eq"), &[a, a ^ g]), Ok(1));
    assert_eq!(exec(op("f32.neg"), &[a]), Ok((-1.5f32).to_bits() as u64));
    assert_eq!(exec(op("f32.abs"), &[(-1.5f32).to_bits() as u64 | g]), Ok(1.5f32.to_bits() as u64));
    assert_eq!(exec(op("f32.copysign"), &[a, b | 0x8000_0000]), Ok((-1.5f32).to_bits() as u64));
}

// ------------------------------------------------------ wrap / extend

fn wrap_extend_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let big = 0x1234_5678_9ABC_DEF0u64;
    c1(&mut v, "i32.wrap_i64", big, Exp::Bits(big as u32 as u64));
    assert_eq!(big as u32 as u64, 0x9ABC_DEF0);
    c1(&mut v, "i32.wrap_i64", u64::MAX, Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i32.wrap_i64", 0x1_0000_0000, Exp::Bits(0));
    c1(&mut v, "i32.wrap_i64", 0xFFFF_FFFF_8000_0000, Exp::Bits(0x8000_0000));
    c1(&mut v, "i64.extend_i32_s", 0x8000_0000, Exp::Bits(0xFFFF_FFFF_8000_0000));
    c1(&mut v, "i64.extend_i32_s", 0x7FFF_FFFF, Exp::Bits(0x7FFF_FFFF));
    c1(&mut v, "i64.extend_i32_s", 0xFFFF_FFFF, Exp::Bits(u64::MAX));
    c1(&mut v, "i64.extend_i32_s", 0, Exp::Bits(0));
    c1(&mut v, "i64.extend_i32_u", 0x8000_0000, Exp::Bits(0x8000_0000));
    c1(&mut v, "i64.extend_i32_u", 0xFFFF_FFFF, Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i64.extend_i32_u", 1, Exp::Bits(1));
    // i32.extend8_s / 16_s: result zero-extended into the 64-bit register
    c1(&mut v, "i32.extend8_s", 0x80, Exp::Bits(0xFFFF_FF80));
    c1(&mut v, "i32.extend8_s", 0x7F, Exp::Bits(0x7F));
    c1(&mut v, "i32.extend8_s", 0xFF, Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i32.extend8_s", 0x180, Exp::Bits(0xFFFF_FF80));
    c1(&mut v, "i32.extend8_s", 0x1234_5600, Exp::Bits(0));
    c1(&mut v, "i32.extend16_s", 0x8000, Exp::Bits(0xFFFF_8000));
    c1(&mut v, "i32.extend16_s", 0x7FFF, Exp::Bits(0x7FFF));
    c1(&mut v, "i32.extend16_s", 0xFFFF, Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i32.extend16_s", 0x1_8000, Exp::Bits(0xFFFF_8000));
    c1(&mut v, "i32.extend16_s", 0x7F80_0001, Exp::Bits(1));
    c1(&mut v, "i64.extend8_s", 0x80, Exp::Bits(0xFFFF_FFFF_FFFF_FF80));
    c1(&mut v, "i64.extend8_s", 0x7F, Exp::Bits(0x7F));
    c1(&mut v, "i64.extend8_s", 0xFFFF_FF7F, Exp::Bits(0x7F));
    c1(&mut v, "i64.extend8_s", 0x1234_5678_9ABC_DE80, Exp::Bits(0xFFFF_FFFF_FFFF_FF80));
    c1(&mut v, "i64.extend16_s", 0x8000, Exp::Bits(0xFFFF_FFFF_FFFF_8000));
    c1(&mut v, "i64.extend16_s", 0x1234_7FFF, Exp::Bits(0x7FFF));
    c1(&mut v, "i64.extend16_s", 0xFFFF_FFFF_FFFF_0001, Exp::Bits(1));
    c1(&mut v, "i64.extend32_s", 0x8000_0000, Exp::Bits(0xFFFF_FFFF_8000_0000));
    c1(&mut v, "i64.extend32_s", 0x1_7FFF_FFFF, Exp::Bits(0x7FFF_FFFF));
    c1(&mut v, "i64.extend32_s", 0xFFFF_FFFF, Exp::Bits(u64::MAX));
    c1(&mut v, "i64.extend32_s", 0xABCD_0000_0000_0001, Exp::Bits(1));
    v
}

#[test]
fn wrap_and_extend() {
    run_cases(&wrap_extend_cases());
}

// ------------------------------------------------------- trapping trunc

fn t32(v: &mut Vec<Case>, name: &str, x: f32, exp: Exp) {
    c1(v, name, x.to_bits() as u64, exp);
}
fn t64(v: &mut Vec<Case>, name: &str, x: f64, exp: Exp) {
    c1(v, name, x.to_bits(), exp);
}

fn trunc_cases() -> Vec<Case> {
    let ov = || Exp::Trap(Trap::IntegerOverflow);
    let inv = || Exp::Trap(Trap::InvalidConversion);
    let i32b = |x: i32| Exp::Bits(x as u32 as u64);
    let u32b = |x: u32| Exp::Bits(x as u64);
    let mut v = Vec::new();
    let nan32s = [f32::NAN, f32::from_bits(0xFFC0_0001), f32::from_bits(0x7F80_0001)];
    let nan64s = [f64::NAN, f64::from_bits(0xFFF8_0000_0000_0001), f64::from_bits(0x7FF0_0000_0000_0001)];

    // i32.trunc_f32_s
    let n = "i32.trunc_f32_s";
    t32(&mut v, n, -2147483648.0, i32b(i32::MIN));
    t32(&mut v, n, 2147483648.0, ov());
    t32(&mut v, n, -2147483904.0, ov());
    t32(&mut v, n, 2147483520.0, i32b(2147483520));
    t32(&mut v, n, -2147483520.0, i32b(-2147483520));
    t32(&mut v, n, -1.9, i32b(-1));
    t32(&mut v, n, 1.9, i32b(1));
    t32(&mut v, n, -0.0, i32b(0));
    t32(&mut v, n, f32::INFINITY, ov());
    t32(&mut v, n, f32::NEG_INFINITY, ov());
    for x in nan32s { t32(&mut v, n, x, inv()); }
    // i32.trunc_f32_u
    let n = "i32.trunc_f32_u";
    t32(&mut v, n, -0.9, u32b(0));
    t32(&mut v, n, -1.0, ov());
    t32(&mut v, n, 4294967296.0, ov());
    t32(&mut v, n, 4294967040.0, u32b(4294967040));
    t32(&mut v, n, 0.9, u32b(0));
    t32(&mut v, n, 2147483648.0, u32b(0x8000_0000));
    t32(&mut v, n, f32::INFINITY, ov());
    t32(&mut v, n, f32::NEG_INFINITY, ov());
    for x in nan32s { t32(&mut v, n, x, inv()); }
    // i32.trunc_f64_s
    let n = "i32.trunc_f64_s";
    t64(&mut v, n, -2147483648.9, i32b(i32::MIN));
    t64(&mut v, n, -2147483648.0, i32b(i32::MIN));
    t64(&mut v, n, -2147483649.0, ov());
    t64(&mut v, n, 2147483647.9, i32b(i32::MAX));
    t64(&mut v, n, 2147483647.0, i32b(i32::MAX));
    t64(&mut v, n, 2147483648.0, ov());
    t64(&mut v, n, -1.5, i32b(-1));
    t64(&mut v, n, -0.0, i32b(0));
    t64(&mut v, n, f64::INFINITY, ov());
    t64(&mut v, n, f64::NEG_INFINITY, ov());
    for x in nan64s { t64(&mut v, n, x, inv()); }
    // i32.trunc_f64_u
    let n = "i32.trunc_f64_u";
    t64(&mut v, n, -0.9, u32b(0));
    t64(&mut v, n, -1.0, ov());
    t64(&mut v, n, 4294967295.9, u32b(u32::MAX));
    t64(&mut v, n, 4294967295.0, u32b(u32::MAX));
    t64(&mut v, n, 4294967296.0, ov());
    t64(&mut v, n, f64::INFINITY, ov());
    t64(&mut v, n, f64::NEG_INFINITY, ov());
    for x in nan64s { t64(&mut v, n, x, inv()); }
    // i64.trunc_f32_s
    let n = "i64.trunc_f32_s";
    let p63 = (1u64 << 63) as f32;
    t32(&mut v, n, -p63, Exp::Bits(i64::MIN as u64));
    t32(&mut v, n, p63, ov());
    t32(&mut v, n, ((1u64 << 63) - (1 << 39)) as f32, Exp::Bits((1u64 << 63) - (1 << 39)));
    t32(&mut v, n, -(((1u64 << 63) + (1 << 40)) as f32), ov());
    t32(&mut v, n, -1.5, Exp::Bits(u64::MAX));
    t32(&mut v, n, f32::INFINITY, ov());
    t32(&mut v, n, f32::NEG_INFINITY, ov());
    for x in nan32s { t32(&mut v, n, x, inv()); }
    // i64.trunc_f32_u
    let n = "i64.trunc_f32_u";
    let p64 = 18446744073709551616.0f32;
    t32(&mut v, n, p64, ov());
    t32(&mut v, n, ((u64::MAX >> 40) << 40) as f32, Exp::Bits((u64::MAX >> 40) << 40));
    t32(&mut v, n, -0.9, Exp::Bits(0));
    t32(&mut v, n, -1.0, ov());
    t32(&mut v, n, p63, Exp::Bits(1 << 63));
    t32(&mut v, n, f32::INFINITY, ov());
    t32(&mut v, n, f32::NEG_INFINITY, ov());
    for x in nan32s { t32(&mut v, n, x, inv()); }
    // i64.trunc_f64_s
    let n = "i64.trunc_f64_s";
    let q63 = 9223372036854775808.0f64;
    t64(&mut v, n, -q63, Exp::Bits(i64::MIN as u64));
    t64(&mut v, n, q63, ov());
    t64(&mut v, n, 9223372036854774784.0, Exp::Bits(9223372036854774784));
    t64(&mut v, n, -9223372036854777856.0, ov());
    t64(&mut v, n, -1.5, Exp::Bits(u64::MAX));
    t64(&mut v, n, f64::INFINITY, ov());
    t64(&mut v, n, f64::NEG_INFINITY, ov());
    for x in nan64s { t64(&mut v, n, x, inv()); }
    // i64.trunc_f64_u
    let n = "i64.trunc_f64_u";
    t64(&mut v, n, 18446744073709549568.0, Exp::Bits(18446744073709549568));
    t64(&mut v, n, 18446744073709551616.0, ov());
    t64(&mut v, n, -0.9, Exp::Bits(0));
    t64(&mut v, n, -1.0, ov());
    t64(&mut v, n, q63, Exp::Bits(1 << 63));
    t64(&mut v, n, f64::INFINITY, ov());
    t64(&mut v, n, f64::NEG_INFINITY, ov());
    for x in nan64s { t64(&mut v, n, x, inv()); }
    v
}

#[test]
fn trunc_trapping() {
    run_cases(&trunc_cases());
}

// ------------------------------------------------------------- trunc_sat

fn trunc_sat_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let s32: &[f32] = &[
        f32::NAN, f32::from_bits(0xFFC0_0001), f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, 0.9, -0.9, 1.9, -1.9, 3e9, -3e9,
        5e9, -5e9, 2147483648.0, -2147483648.0, -2147483904.0, 2147483520.0, 4294967296.0, 4294967040.0, 1e30, -1e30, 9.3e18,
        -9.3e18, 1.9e19, 1.8446743e19, 1.8446742e19, 1.0e10, -5.0,
    ];
    let s64: Vec<f64> = vec![
        f64::NAN, f64::from_bits(0xFFF8_0000_0000_0001), f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.0, 0.9, -0.9, 1.9, -1.9,
        3e9, -3e9, 5e9, -5e9, 2147483647.9, -2147483648.9, -2147483649.0, 2147483648.0, 4294967295.9, 4294967296.0, 1e30,
        -1e30, 9223372036854775808.0, -9223372036854775808.0, -9223372036854777856.0, 9223372036854774784.0,
        18446744073709549568.0, 18446744073709551616.0, 1e19, -5.0,
    ];
    for &x in s32 {
        let a = x.to_bits() as u64;
        c1(&mut v, "i32.trunc_sat_f32_s", a, Exp::Bits((x as i32) as u32 as u64));
        c1(&mut v, "i32.trunc_sat_f32_u", a, Exp::Bits((x as u32) as u64));
        c1(&mut v, "i64.trunc_sat_f32_s", a, Exp::Bits((x as i64) as u64));
        c1(&mut v, "i64.trunc_sat_f32_u", a, Exp::Bits(x as u64));
    }
    for &x in &s64 {
        let a = x.to_bits();
        c1(&mut v, "i32.trunc_sat_f64_s", a, Exp::Bits((x as i32) as u32 as u64));
        c1(&mut v, "i32.trunc_sat_f64_u", a, Exp::Bits((x as u32) as u64));
        c1(&mut v, "i64.trunc_sat_f64_s", a, Exp::Bits((x as i64) as u64));
        c1(&mut v, "i64.trunc_sat_f64_u", a, Exp::Bits(x as u64));
    }
    // literal spot checks (independent of Rust's cast semantics)
    let inf32 = f32::INFINITY.to_bits() as u64;
    let ninf32 = f32::NEG_INFINITY.to_bits() as u64;
    let nan32 = f32::NAN.to_bits() as u64;
    c1(&mut v, "i32.trunc_sat_f32_s", nan32, Exp::Bits(0));
    c1(&mut v, "i32.trunc_sat_f32_s", inf32, Exp::Bits(0x7FFF_FFFF));
    c1(&mut v, "i32.trunc_sat_f32_s", ninf32, Exp::Bits(0x8000_0000));
    c1(&mut v, "i32.trunc_sat_f32_u", nan32, Exp::Bits(0));
    c1(&mut v, "i32.trunc_sat_f32_u", inf32, Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i32.trunc_sat_f32_u", ninf32, Exp::Bits(0));
    c1(&mut v, "i64.trunc_sat_f32_s", inf32, Exp::Bits(0x7FFF_FFFF_FFFF_FFFF));
    c1(&mut v, "i64.trunc_sat_f32_s", ninf32, Exp::Bits(0x8000_0000_0000_0000));
    c1(&mut v, "i64.trunc_sat_f32_u", inf32, Exp::Bits(u64::MAX));
    c1(&mut v, "i64.trunc_sat_f32_u", ninf32, Exp::Bits(0));
    let inf = f64::INFINITY.to_bits();
    let ninf = f64::NEG_INFINITY.to_bits();
    c1(&mut v, "i32.trunc_sat_f64_s", (-3e9f64).to_bits(), Exp::Bits(0x8000_0000));
    c1(&mut v, "i32.trunc_sat_f64_s", 3e9f64.to_bits(), Exp::Bits(0x7FFF_FFFF));
    c1(&mut v, "i32.trunc_sat_f64_u", (-1.0f64).to_bits(), Exp::Bits(0));
    c1(&mut v, "i32.trunc_sat_f64_u", 5e9f64.to_bits(), Exp::Bits(0xFFFF_FFFF));
    c1(&mut v, "i64.trunc_sat_f64_s", inf, Exp::Bits(0x7FFF_FFFF_FFFF_FFFF));
    c1(&mut v, "i64.trunc_sat_f64_s", ninf, Exp::Bits(0x8000_0000_0000_0000));
    c1(&mut v, "i64.trunc_sat_f64_u", inf, Exp::Bits(u64::MAX));
    c1(&mut v, "i64.trunc_sat_f64_u", (-1.0f64).to_bits(), Exp::Bits(0));
    c1(&mut v, "i64.trunc_sat_f64_u", 1e19f64.to_bits(), Exp::Bits(10_000_000_000_000_000_000));
    c1(&mut v, "i64.trunc_sat_f64_s", f64::NAN.to_bits(), Exp::Bits(0));
    v
}

#[test]
fn trunc_saturating() {
    run_cases(&trunc_sat_cases());
}

// --------------------------------------------------------------- convert

fn convert_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let i32s: &[u32] = &[0, 1, 0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF, 16777217, 16777219, 0x8000_0001, 0xFFFF_FF01, 123456789];
    let i64s: &[u64] = &[
        0, 1, u64::MAX, 1 << 63, (1 << 63) - 1, (1 << 24) + 1, (1 << 24) + 3, (1 << 53) + 1, (1 << 53) + 3, 0xFFFF_FFFF,
        0x1_0000_0000, 0x7FFF_FF80_0000_0001, 0xFFFF_FF7F_FFFF_FFFF, 0x1234_5678_9ABC_DEF0, 0x8000_0000_0000_0401,
        u64::MAX - 1023, u64::MAX - 1024,
    ];
    for &x in i32s {
        c1(&mut v, "f32.convert_i32_s", x as u64, Exp::Bits((x as i32 as f32).to_bits() as u64));
        c1(&mut v, "f32.convert_i32_u", x as u64, Exp::Bits((x as f32).to_bits() as u64));
        c1(&mut v, "f64.convert_i32_s", x as u64, Exp::Bits((x as i32 as f64).to_bits()));
        c1(&mut v, "f64.convert_i32_u", x as u64, Exp::Bits((x as f64).to_bits()));
    }
    for &x in i64s {
        c1(&mut v, "f32.convert_i64_s", x, Exp::Bits((x as i64 as f32).to_bits() as u64));
        c1(&mut v, "f32.convert_i64_u", x, Exp::Bits((x as f32).to_bits() as u64));
        c1(&mut v, "f64.convert_i64_s", x, Exp::Bits((x as i64 as f64).to_bits()));
        c1(&mut v, "f64.convert_i64_u", x, Exp::Bits((x as f64).to_bits()));
    }
    // literal checks
    let f = |x: f32| Exp::Bits(x.to_bits() as u64);
    let d = |x: f64| Exp::Bits(x.to_bits());
    c1(&mut v, "f32.convert_i32_s", 0xFFFF_FFFF, f(-1.0));
    c1(&mut v, "f32.convert_i32_u", 0xFFFF_FFFF, f(4294967296.0));
    c1(&mut v, "f64.convert_i32_s", 0xFFFF_FFFF, d(-1.0));
    c1(&mut v, "f64.convert_i32_u", 0xFFFF_FFFF, d(4294967295.0));
    c1(&mut v, "f32.convert_i64_s", u64::MAX, f(-1.0));
    c1(&mut v, "f32.convert_i64_u", u64::MAX, f(18446744073709551616.0));
    c1(&mut v, "f64.convert_i64_s", u64::MAX, d(-1.0));
    c1(&mut v, "f64.convert_i64_u", u64::MAX, d(18446744073709551616.0));
    c1(&mut v, "f32.convert_i64_s", 1 << 63, f(-9223372036854775808.0));
    c1(&mut v, "f64.convert_i64_s", 1 << 63, d(-9223372036854775808.0));
    c1(&mut v, "f64.convert_i64_u", 1 << 63, d(9223372036854775808.0));
    c1(&mut v, "f32.convert_i64_s", (1 << 24) + 1, f(16777216.0));
    c1(&mut v, "f32.convert_i64_s", (1 << 24) + 3, f(16777220.0));
    c1(&mut v, "f64.convert_i64_s", (1 << 53) + 1, d(9007199254740992.0));
    v
}

#[test]
fn convert_int_to_float() {
    run_cases(&convert_cases());
}

// -------------------------------------------------- demote / promote / reinterpret

fn demote_promote_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let f = |x: f32| Exp::Bits(x.to_bits() as u64);
    let d = |x: f64| Exp::Bits(x.to_bits());
    let dem: &[f64] = &[
        1.5, -1.5, 0.0, -0.0, 1e300, -1e300, 1e-50, -1e-50, f64::INFINITY, f64::NEG_INFINITY, f32::MAX as f64,
        f32::MIN as f64, 3.4028235677973366e38, 3.4028235677973362e38, 3.5e38, 16777217.0, 1e-45, 0.1,
    ];
    for &x in dem {
        c2dem(&mut v, x, f(x as f32));
    }
    c1(&mut v, "f32.demote_f64", 1e300f64.to_bits(), f(f32::INFINITY));
    c1(&mut v, "f32.demote_f64", (-1e300f64).to_bits(), f(f32::NEG_INFINITY));
    c1(&mut v, "f32.demote_f64", 16777217.0f64.to_bits(), f(16777216.0));
    c1(&mut v, "f32.demote_f64", (-0.0f64).to_bits(), Exp::Bits(0x8000_0000));
    c1(&mut v, "f32.demote_f64", f64::NAN.to_bits(), Exp::Nan32);
    c1(&mut v, "f32.demote_f64", 0xFFF8_0000_0000_0001, Exp::Nan32);
    c1(&mut v, "f32.demote_f64", 0x7FF0_0000_0000_0001, Exp::Nan32);
    let pro: &[f32] = &[1.5, -1.5, 0.0, -0.0, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 1e-45, f32::MIN_POSITIVE, 0.1, 16777216.0];
    for &x in pro {
        c1(&mut v, "f64.promote_f32", x.to_bits() as u64, d(x as f64));
    }
    c1(&mut v, "f64.promote_f32", 0.1f32.to_bits() as u64, d(0.10000000149011612));
    c1(&mut v, "f64.promote_f32", (-0.0f32).to_bits() as u64, Exp::Bits(1 << 63));
    c1(&mut v, "f64.promote_f32", f32::NAN.to_bits() as u64, Exp::Nan64);
    c1(&mut v, "f64.promote_f32", 0xFFC0_0001, Exp::Nan64);
    c1(&mut v, "f64.promote_f32", 0x7F80_0001, Exp::Nan64);

    // reinterpret: bit identity
    let bits32: &[u32] = &[0, 0x8000_0000, 0x7FC0_0001, 0xFFC0_0000, 0x7F80_0000, 0x3F80_0000, 0xFFFF_FFFF, 0x1234_5678, 1];
    for &b in bits32 {
        c1(&mut v, "i32.reinterpret_f32", b as u64, Exp::Bits(b as u64));
        c1(&mut v, "f32.reinterpret_i32", b as u64, Exp::Bits(b as u64));
    }
    let bits64: &[u64] = &[
        0, 1 << 63, 0x7FF8_0000_0000_0001, 0xFFF8_0000_0000_0001, 0x7FF0_0000_0000_0000, 0x3FF0_0000_0000_0000, u64::MAX,
        0x0123_4567_89AB_CDEF, 1,
    ];
    for &b in bits64 {
        c1(&mut v, "i64.reinterpret_f64", b, Exp::Bits(b));
        c1(&mut v, "f64.reinterpret_i64", b, Exp::Bits(b));
    }
    v
}

fn c2dem(v: &mut Vec<Case>, x: f64, exp: Exp) {
    c1(v, "f32.demote_f64", x.to_bits(), exp);
}

#[test]
fn demote_promote_reinterpret() {
    run_cases(&demote_promote_cases());
}

#[test]
fn reinterpret_upper_bits() {
    // i32.reinterpret_f32 / f32.reinterpret_i32 take the low 32 bits and zero the upper half
    let g = 0xDEAD_BEEF_0000_0000u64;
    assert_eq!(exec(op("i32.reinterpret_f32"), &[g | 0x8000_0001]), Ok(0x8000_0001));
    assert_eq!(exec(op("f32.reinterpret_i32"), &[g | 0xFFC0_0000]), Ok(0xFFC0_0000));
    assert_eq!(exec(op("i64.reinterpret_f64"), &[u64::MAX]), Ok(u64::MAX));
}

// -------------------------------------------------------------- coverage

fn is_mine(name: &str) -> bool {
    if name.ends_with(".load") || name.ends_with(".store") {
        return false;
    }
    name.starts_with("f32.")
        || name.starts_with("f64.")
        || ["wrap", "extend", "trunc", "convert", "demote", "promote", "reinterpret"]
            .iter()
            .any(|k| name.contains(k))
}

#[test]
fn coverage_every_float_and_conversion_op_is_tested() {
    let mut all = Vec::new();
    all.extend(float_cases(true));
    all.extend(float_cases(false));
    all.extend(wrap_extend_cases());
    all.extend(trunc_cases());
    all.extend(trunc_sat_cases());
    all.extend(convert_cases());
    all.extend(demote_promote_cases());
    let covered: HashSet<Op> = all.iter().map(|c| c.op).collect();
    let missing: Vec<&str> = Op::ALL
        .iter()
        .filter(|o| is_mine(o.name()) && !covered.contains(o))
        .map(|o| o.name())
        .collect();
    assert!(missing.is_empty(), "uncovered ops: {missing:?}");
    // sanity: we really are checking a large set
    assert!(Op::ALL.iter().filter(|o| is_mine(o.name())).count() == 78);
}
