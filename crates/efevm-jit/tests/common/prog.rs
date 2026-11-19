//! A recursive test program with yields, host calls, host yields, traps, memory and indirect calls.
#![allow(dead_code)]
use super::*;
use efevm::ValType::I64;

pub fn registry() -> HostRegistry<Hs> {
    let mut reg = HostRegistry::new();
    reg_host(&mut reg, "ping", ty64(1, 1), |c, a, r| {
        c.host().log.push(a[0]);
        r[0] = a[0] + 1;
        Ok(HostOk::Continue)
    });
    reg_host(&mut reg, "hy", ty64(1, 1), |c, a, r| {
        c.host().log.push(1000 + a[0]);
        r[0] = a[0] * 2;
        Ok(if a[0] % 3 == 0 { HostOk::Yield } else { HostOk::Continue })
    });
    reg_host(&mut reg, "memw", ty64(2, 0), |c, a, _| {
        let (addr, v) = (a[0], a[1]);
        if addr == 56 {
            c.memory_mut().grow(1);
        }
        c.write(addr, &v.to_le_bytes())?;
        Ok(HostOk::Continue)
    });
    reg_host(&mut reg, "setg", ty64(1, 1), |c, a, r| {
        r[0] = c.global(0).unwrap_or(0);
        c.set_global(0, a[0])?;
        Ok(HostOk::Continue)
    });
    reg_host(&mut reg, "bad", ty64(1, 1), |_, a, r| {
        if a[0] == 13 {
            return Err(Trap::Host("bad".into()));
        }
        r[0] = a[0];
        Ok(HostOk::Continue)
    });
    reg
}

/// Exports: `main(n, sel) -> i64`, `rec(n) -> i64`, global `count`.
pub fn module() -> Module {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, Some(3));
    let ping = mb.import("env", "ping", ty64(1, 1));
    let hy = mb.import("env", "hy", ty64(1, 1));
    let memw = mb.import("env", "memw", ty64(2, 0));
    let setg = mb.import("env", "setg", ty64(1, 1));
    let bad = mb.import("env", "bad", ty64(1, 1));
    let g = mb.global(I64, true, 0);
    mb.export_global("count", g);
    let t11 = mb.add_type(ty64(1, 1));
    let t21 = mb.add_type(ty64(2, 1));
    let rec = mb.declare_func(ty64(1, 1));
    let main = mb.declare_func(ty64(2, 1));
    let helper = mb.declare_func(ty64(1, 1));
    mb.table(vec![Some(rec), Some(ping), None, Some(helper), Some(memw), Some(main), Some(setg)]);

    // helper(x) = x * 3 + 1
    let mut h = FunctionBuilder::new(3);
    h.const_i64(1, 3);
    h.op3(Op::I64Mul, 0, 0, 1);
    h.const_i64(2, 1);
    h.op3(Op::I64Add, 0, 0, 2);
    h.ret(0);
    mb.define(helper, h);

    let mut f = FunctionBuilder::new(12);
    let (base, skip, ok13) = (f.new_label(), f.new_label(), f.new_label());
    f.const_i64(2, 0xAAAA);
    f.br_ifz(0, base);
    f.const_i64(1, 1);
    f.op3(Op::I64And, 3, 0, 1);
    f.br_ifz(3, skip);
    f.yield_();
    f.bind(skip);
    f.mov(4, 0);
    f.call(ping, 4);
    f.mov(5, 0);
    f.call(hy, 5);
    f.mov(9, 0);
    f.call(bad, 9);
    f.const_i64(1, 1);
    f.op3(Op::I64Sub, 6, 0, 1);
    f.call(rec, 6);
    f.op3(Op::I64Add, 6, 6, 4);
    f.op3(Op::I64Add, 6, 6, 5);
    f.op3(Op::I64Xor, 6, 6, 2);
    f.const_i64(1, 8);
    f.op3(Op::I64Mul, 7, 0, 1);
    f.store(Op::I64Store, 6, 7, 0);
    f.global_get(8, g);
    f.const_i64(1, 1);
    f.op3(Op::I64Add, 8, 8, 1);
    f.global_set(g, 8);
    f.mov(10, 7);
    f.mov(11, 6);
    f.call(memw, 10);
    f.mov(9, 6);
    f.call_indirect(t11, 1, 9); // table[1] = ping, host call through the table
    f.br_ifz(0, ok13);
    f.bind(ok13);
    f.op3(Op::I64Add, 6, 6, 9);
    f.ret(6);
    f.bind(base);
    f.const_i64(4, 1);
    f.ret(4);
    mb.define(rec, f);

    // main(n, sel)
    let mut m = FunctionBuilder::new(8);
    m.const_i64(5, 0x77);
    m.mov(3, 0);
    m.call(rec, 3);
    m.mov(4, 0);
    m.call_indirect(t11, 1, 4);
    m.op3(Op::I64Add, 2, 3, 4);
    m.op3(Op::I64Add, 2, 2, 5);
    m.mov(6, 0);
    m.mov(7, 1);
    m.call_indirect(t21, 1, 6);
    m.op3(Op::I64Xor, 2, 2, 6);
    m.ret(2);
    mb.define(main, m);
    mb.export_func("main", main);
    mb.export_func("rec", rec);
    let _ = setg;
    mb.finish().unwrap()
}
