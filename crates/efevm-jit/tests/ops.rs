//! Per-op differential tests: every op of `Op::ALL` with edge inputs, interpreter vs JIT.
mod common;
use common::*;
use efevm::ValType::I64;
use efevm::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum K {
    I32,
    I64,
    F32,
    F64,
}

fn parse_ty(s: &str) -> Option<K> {
    Some(match s {
        "i32" => K::I32,
        "i64" => K::I64,
        "f32" => K::F32,
        "f64" => K::F64,
        _ => return None,
    })
}

/// Operand type of an arithmetic/conversion op, from its mnemonic.
fn operand_kind(name: &str) -> Option<K> {
    let (pre, rest) = name.split_once('.')?;
    let p = parse_ty(pre)?;
    for key in ["wrap_", "extend_", "trunc_sat_", "trunc_", "convert_", "demote_", "promote_", "reinterpret_"] {
        if let Some(r) = rest.strip_prefix(key) {
            let t = r.split('_').next().unwrap();
            return parse_ty(t);
        }
    }
    Some(p)
}

fn result_is_float(name: &str) -> bool {
    let (pre, rest) = name.split_once('.').unwrap();
    (pre == "f32" || pre == "f64")
        && !["eq", "ne", "lt", "gt", "le", "ge"].contains(&rest)
        && !rest.starts_with("reinterpret")
        && !rest.starts_with("load")
        && !rest.starts_with("store")
}

fn i32_edges() -> Vec<u64> {
    let mut v: Vec<u64> = vec![0, 1, 2, 3, 7, 31, 32, 33, 63, 64, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0xdead_beef, 0xffff_fffb, 0x80, 0xff, 0x8000, 0xffff, 0x7f, 0x7fff, 0x1234_5678, 0x8000_0001];
    // garbage in the upper half must be ignored by 32-bit ops
    v.extend([0x1_0000_0001, 0xffff_ffff_0000_0005, 0xabcd_0000_8000_0000]);
    v
}
fn i64_edges() -> Vec<u64> {
    vec![
        0, 1, 2, 3, 31, 32, 33, 63, 64, 65, 0xffff_ffff, 0x1_0000_0000, 0x7fff_ffff, 0x8000_0000, u64::MAX, i64::MIN as u64,
        i64::MAX as u64, 0xdead_beef_cafe_f00d, (-5i64) as u64, 0x8000_0000_0000_0001, 0x0123_4567_89ab_cdef, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff,
    ]
}
fn f32_edges() -> Vec<u64> {
    let f = |x: f32| x.to_bits() as u64;
    let mut v = vec![
        f(0.0),
        f(-0.0),
        f(1.0),
        f(-1.0),
        f(0.5),
        f(-0.5),
        f(1.5),
        f(-1.5),
        f(2.5),
        f(-2.5),
        f(3.5),
        f(f32::INFINITY),
        f(f32::NEG_INFINITY),
        f(f32::NAN),
        f(f32::MAX),
        f(f32::MIN),
        f(f32::MIN_POSITIVE),
        1,
        0x8000_0001,
        f(2147483648.0),
        f(-2147483648.0),
        f(2147483520.0),
        0xcf00_0001, // -2147483904
        f(4294967296.0),
        f(4294967040.0),
        f(9223372036854775808.0),
        f(-9223372036854775808.0),
        f(18446744073709551616.0),
        f(9223371487098961920.0),
        f(18446742974197923840.0),
        0xbf7f_ffff, // -0.99999994
        0xbf80_0000,
        f(0.99999994),
        f(16777217.0),
        f(100.7),
        f(-100.7),
        0x7fc0_0001, // another NaN
        0xffc0_0000, // negative NaN
        0x7f80_0001, // signalling NaN
    ];
    v.push(0xffff_ffff_3f80_0000); // garbage upper half
    v
}
fn f64_edges() -> Vec<u64> {
    let f = |x: f64| x.to_bits();
    vec![
        f(0.0),
        f(-0.0),
        f(1.0),
        f(-1.0),
        f(0.5),
        f(-0.5),
        f(1.5),
        f(-1.5),
        f(2.5),
        f(-2.5),
        f(f64::INFINITY),
        f(f64::NEG_INFINITY),
        f(f64::NAN),
        f(f64::MAX),
        f(f64::MIN),
        f(f64::MIN_POSITIVE),
        1,
        f(2147483648.0),
        f(-2147483648.0),
        f(-2147483649.0),
        f(2147483647.9),
        f(-2147483648.9),
        f(4294967296.0),
        f(4294967295.9),
        f(9223372036854775808.0),
        f(-9223372036854775808.0),
        f(18446744073709551616.0),
        f(9223372036854774784.0),
        f(18446744073709549568.0),
        f(-0.9999999999999999),
        f(-1.0000000000000002),
        f(100.7),
        f(-100.7),
        f(1e300),
        0x7ff8_0000_0000_0001,
        0xfff8_0000_0000_0000,
        0x7ff0_0000_0000_0001,
        f(4503599627370497.5),
    ]
}
fn edges(k: K) -> Vec<u64> {
    match k {
        K::I32 => i32_edges(),
        K::I64 => i64_edges(),
        K::F32 => f32_edges(),
        K::F64 => f64_edges(),
    }
}

fn one_op_module(op: Op, binary: bool) -> Module {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(3);
    if binary {
        f.op3(op, 2, 0, 1);
    } else {
        f.op2(op, 2, 0);
    }
    f.ret(2);
    let func = mb.add_func(ty(&[I64, I64], &[I64]), f);
    mb.export_func("f", func);
    mb.finish().unwrap()
}

fn arith_ops() -> Vec<(Op, bool)> {
    Op::ALL
        .iter()
        .copied()
        .filter_map(|op| {
            let name = op.name();
            let (pre, rest) = name.split_once('.')?;
            parse_ty(pre)?;
            if rest.starts_with("load") || rest.starts_with("store") {
                return None;
            }
            match op.shape() {
                Shape::DAB => Some((op, true)),
                Shape::DA => Some((op, false)),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn all_arith_ops_edges() {
    let ops = arith_ops();
    assert!(ops.len() > 130, "found {} arithmetic ops", ops.len());
    let before = efevm_jit::native_runs();
    for (op, binary) in ops {
        let name = op.name();
        let k = operand_kind(name).unwrap();
        let m = one_op_module(op, binary);
        let cfg = Cfg { nan: result_is_float(name), ..Cfg::default() };
        let mut pair = Pair::new(&m, &HostRegistry::new(), &cfg);
        let e = edges(k);
        let second = if binary { e.clone() } else { vec![0] };
        for &x in &e {
            for &y in &second {
                pair.run("f", &[x, y], &[u64::MAX]);
            }
        }
        // also a budgeted run (one block: whatever the budget, same exit)
        pair.run("f", &[e[1], e[2]], &[1]);
        pair.run("f", &[e[1], e[2]], &[0, 1, 5]);
    }
    assert!(efevm_jit::native_runs() > before + 10_000, "the JIT must actually run");
}

#[test]
fn trap_kinds_and_locations() {
    // div by zero, MIN / -1, rem MIN % -1 = 0, trunc traps: check kinds explicitly (the diff
    // already compares to the interpreter; this pins the expected semantics).
    let m = one_op_module(Op::I32DivS, true);
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    let s = p.run("f", &[1, 0], &[u64::MAX]);
    assert_eq!(s.exit, Exit::Trap(Trap::DivideByZero));
    assert_eq!(s.trap_loc, Some((0, 0)));
    let s = p.run("f", &[0x8000_0000, 0xffff_ffff], &[u64::MAX]);
    assert_eq!(s.exit, Exit::Trap(Trap::IntegerOverflow));
    let m = one_op_module(Op::I64RemS, true);
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    let s = p.run("f", &[i64::MIN as u64, u64::MAX], &[u64::MAX]);
    assert_eq!(s.results, Some(vec![0]));
    let m = one_op_module(Op::I32TruncF32S, false);
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    assert_eq!(p.run("f", &[f32::NAN.to_bits() as u64, 0], &[u64::MAX]).exit, Exit::Trap(Trap::InvalidConversion));
    assert_eq!(p.run("f", &[3e9f32.to_bits() as u64, 0], &[u64::MAX]).exit, Exit::Trap(Trap::IntegerOverflow));
    assert_eq!(p.run("f", &[(-2147483648.0f32).to_bits() as u64, 0], &[u64::MAX]).results, Some(vec![0x8000_0000]));
}

#[test]
fn control_and_misc_ops() {
    // select, mov, consts, br_table, br_if, br_ifz, nop, unreachable, globals
    let mut mb = ModuleBuilder::new();
    let g = mb.global(ValType::I64, true, 77);
    let g32 = mb.global(ValType::I32, true, 5);
    let mut f = FunctionBuilder::new(8);
    let (l0, l1, l2, ld, end) = (f.new_label(), f.new_label(), f.new_label(), f.new_label(), f.new_label());
    f.nop();
    f.const_i32(3, -9);
    f.const_i64(4, 0x1122_3344_5566_7788);
    f.const_f32(5, 1.5);
    f.const_f64(6, -2.25);
    f.global_get(7, g);
    f.op3(Op::I64Add, 7, 7, 0);
    f.global_set(g, 7);
    f.global_get(7, g32);
    f.global_set(g32, 3);
    f.br_table(0, &[l0, l1, l2], ld);
    f.bind(l0);
    f.select(7, 1, 3, 4);
    f.jmp(end);
    f.bind(l1);
    f.mov(7, 4);
    f.br_if(1, end);
    f.mov(7, 5);
    f.jmp(end);
    f.bind(l2);
    f.br_ifz(1, end);
    f.mov(7, 6);
    f.jmp(end);
    f.bind(ld);
    f.unreachable();
    f.bind(end);
    f.ret(7);
    let func = mb.add_func(ty(&[I64, I64], &[I64]), f);
    mb.export_func("f", func);
    let m = mb.finish().unwrap();
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    for idx in [0u64, 1, 2, 3, 4, 0x1_0000_0001, 0xffff_ffff, 0xffff_ffff_0000_0000] {
        for c in [0u64, 1, 0xffff_ffff_0000_0000, 0x1_0000_0000, 2] {
            for b in [&[u64::MAX][..], &[1], &[2], &[3, 1, 4]] {
                p.run("f", &[idx, c], b);
            }
        }
    }
}
