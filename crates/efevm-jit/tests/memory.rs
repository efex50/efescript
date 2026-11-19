//! Memory ops, globals and bulk operations: interpreter vs JIT.
mod common;
use common::*;
use efevm::ValType::I64;
use efevm::*;

fn pattern() -> Vec<u8> {
    (0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8 ^ 0x5a).collect()
}

fn load_ops() -> Vec<(Op, i64)> {
    Op::ALL
        .iter()
        .copied()
        .filter(|o| o.shape() == Shape::Load)
        .map(|o| {
            let n = o.name();
            let size = if n.contains("load8") {
                1
            } else if n.contains("load16") {
                2
            } else if n.contains("load32") || n.starts_with("f32") || n == "i32.load" {
                4
            } else {
                8
            };
            (o, size)
        })
        .collect()
}

fn store_ops() -> Vec<(Op, i64)> {
    Op::ALL
        .iter()
        .copied()
        .filter(|o| o.shape() == Shape::Store)
        .map(|o| {
            let n = o.name();
            let size = if n.contains("store8") {
                1
            } else if n.contains("store16") {
                2
            } else if n.contains("store32") || n == "i32.store" || n == "f32.store" {
                4
            } else {
                8
            };
            (o, size)
        })
        .collect()
}

fn bases(off: u32, size: i64) -> Vec<u64> {
    let mut v: Vec<u64> = vec![0, 1, 2, 3, 4095, 4096, 65535, 65536, 65537, u64::MAX, u64::MAX - 1, u64::MAX - 7, u32::MAX as u64, 1 << 32, 1 << 63, 0x1_0000_0001];
    for k in -10i64..=10 {
        let b = 65536i64 - off as i64 - size + k;
        if b >= 0 {
            v.push(b as u64);
        }
        v.push((u64::MAX as i128 - off as i128 + k as i128 + 1) as u64);
    }
    v
}

#[test]
fn loads() {
    for (op, size) in load_ops() {
        for off in [0u32, 1, 3, 7, 4000, 65535, 65530, u32::MAX, 0x8000_0000] {
            let mut mb = ModuleBuilder::new();
            mb.memory(1, Some(2));
            mb.data(0, &pattern());
            mb.data(65536 - 64, &pattern()[..64]);
            let mut f = FunctionBuilder::new(3);
            f.load(op, 2, 0, off);
            f.ret(2);
            let func = mb.add_func(ty(&[I64], &[I64]), f);
            mb.export_func("f", func);
            let m = mb.finish().unwrap();
            let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg { nan: false, ..Cfg::default() });
            for b in bases(off, size) {
                p.run("f", &[b], &[u64::MAX]);
            }
        }
    }
}

#[test]
fn stores() {
    for (op, size) in store_ops() {
        for off in [0u32, 1, 5, 4000, 65535, 65530, u32::MAX] {
            let mut mb = ModuleBuilder::new();
            mb.memory(1, Some(2));
            mb.data(0, &pattern());
            let mut f = FunctionBuilder::new(3);
            f.store(op, 1, 0, off);
            f.ret(1);
            let func = mb.add_func(ty(&[I64, I64], &[I64]), f);
            mb.export_func("f", func);
            let m = mb.finish().unwrap();
            let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
            for b in bases(off, size) {
                for v in [0x1122_3344_5566_7788u64, u64::MAX, 0x8000_0000_8000_0080] {
                    p.run("f", &[b, v], &[u64::MAX]);
                }
            }
        }
    }
}

#[test]
fn memory_size_grow_and_remap() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, Some(4));
    let mut f = FunctionBuilder::new(7);
    f.const_i64(5, 0x0123_4567_89ab_cdefu64 as i64);
    f.store(Op::I64Store, 5, 1, 0);
    f.memory_grow(2, 0);
    f.memory_size(3);
    f.store(Op::I64Store, 5, 1, 65536);
    f.load(Op::I64Load, 4, 1, 65536);
    f.load(Op::I64Load, 6, 1, 0);
    f.op3(Op::I64Xor, 4, 4, 6);
    f.ret(2);
    let func = mb.add_func(ty(&[I64, I64], &[I64, I64, I64]), f);
    mb.export_func("f", func);
    let m = mb.finish().unwrap();
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    for d in [0u64, 1, 2, 3, 4, u64::MAX, 1 << 32, 0x1_0000_0001] {
        for a in [0u64, 8, 65528, 65529, 3 * 65536 - 8, 3 * 65536 - 7] {
            for b in [&[u64::MAX][..], &[1], &[3, 100]] {
                p.run("f", &[d, a], b);
            }
        }
    }
}

#[test]
fn bulk_copy_fill_charge() {
    let mut mb = ModuleBuilder::new();
    mb.memory(20, Some(20));
    mb.data(0, &pattern());
    let mut f = FunctionBuilder::new(6);
    f.memory_copy(0, 1, 2);
    f.memory_fill(1, 0, 2);
    f.const_i64(5, 1);
    f.ret(5);
    let func = mb.add_func(ty(&[I64, I64, I64], &[I64]), f);
    mb.export_func("f", func);
    let m = mb.finish().unwrap();
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    let lens = [0u64, 1, 255, 256, 257, 4096, 65536, 300_000, 1_000_000, 20 * 65536, 20 * 65536 + 1, u64::MAX];
    let addrs = [0u64, 100, 19 * 65536, 20 * 65536, u64::MAX - 5];
    for &len in &lens {
        for &d in &addrs {
            for &s in &addrs {
                for b in [&[u64::MAX][..], &[1], &[5, 1000, 0, 3]] {
                    p.run("f", &[d, s, len], b);
                }
            }
        }
    }
    // overlapping copy
    for (d, s) in [(1u64, 0u64), (0, 1), (100, 90), (90, 100)] {
        p.run("f", &[d, s, 50], &[u64::MAX]);
    }
}

#[test]
fn globals_roundtrip() {
    let mut mb = ModuleBuilder::new();
    let g0 = mb.global(ValType::I64, true, 5);
    let g1 = mb.global(ValType::I32, true, 7);
    let g2 = mb.global(ValType::F64, true, 0);
    let mut f = FunctionBuilder::new(4);
    f.global_get(1, g0);
    f.op3(Op::I64Add, 1, 1, 0);
    f.global_set(g0, 1);
    f.global_set(g1, 0);
    f.global_set(g2, 0);
    f.global_get(2, g1);
    f.global_get(3, g2);
    f.ret(1);
    let func = mb.add_func(ty(&[I64], &[I64, I64, I64]), f);
    mb.export_func("f", func);
    let m = mb.finish().unwrap();
    let mut p = Pair::new(&m, &HostRegistry::new(), &Cfg::default());
    for v in [0u64, 1, u64::MAX, 0x1_0000_0005, 0xdead_beef_cafe_f00d] {
        p.run("f", &[v], &[u64::MAX]);
    }
}
