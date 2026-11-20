//! Control flow, calls, recursion, indirect calls, limits (SPEC sections 5, 6, 10).
mod common;
use common::reg_host;
use efevm::ValType::I64;
use efevm::*;

fn ty(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![I64; np], vec![I64; nr])
}

/// Module with one exported function "main".
fn one(np: usize, nr: usize, nregs: u16, body: impl FnOnce(&mut FunctionBuilder)) -> Module {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(nregs);
    body(&mut f);
    let id = mb.add_func(ty(np, nr), f);
    mb.export_func("main", id);
    mb.finish().expect("module verifies")
}

fn run_lim(m: &Module, limits: Limits, args: &[u64]) -> (Exit, Vec<u64>) {
    let mut inst = common::new_instance(m, &Imports::<()>::new(), limits).unwrap();
    let f = inst.spawn("main", args).unwrap();
    let exit = inst.resume(f, &mut (), 10_000_000);
    let res = inst.results(f).map(|r| r.to_vec()).unwrap_or_default();
    (exit, res)
}

fn run(m: &Module, args: &[u64]) -> (Exit, Vec<u64>) {
    run_lim(m, Limits::default(), args)
}

fn done(m: &Module, args: &[u64]) -> Vec<u64> {
    let (e, r) = run(m, args);
    assert_eq!(e, Exit::Done);
    r
}

fn trap_of(m: &Module, args: &[u64]) -> Trap {
    match run(m, args).0 {
        Exit::Trap(t) => t,
        e => panic!("expected trap, got {e:?}"),
    }
}

// ---------------------------------------------------------------- jumps

#[test]
fn jmp_forward() {
    let m = one(0, 1, 1, |f| {
        let l = f.new_label();
        f.const_i64(0, 1);
        f.jmp(l);
        f.const_i64(0, 2);
        f.bind(l);
        f.ret(0);
    });
    assert_eq!(done(&m, &[]), vec![1]);
}

#[test]
fn jmp_backward_loop() {
    // r0 = 0; r1 = 5; r2 = 1; loop: r0 += r2 (+1 each); r1 -= 1; br_if r1 loop
    let m = one(0, 1, 3, |f| {
        let top = f.new_label();
        f.const_i64(0, 0);
        f.const_i64(1, 5);
        f.const_i64(2, 1);
        f.bind(top);
        f.op3(Op::I64Add, 0, 0, 2);
        f.op3(Op::I64Sub, 1, 1, 2);
        f.br_if(1, top);
        f.ret(0);
    });
    assert_eq!(done(&m, &[]), vec![5]);
}

#[test]
fn jmp_backward_unconditional_with_exit() {
    // loop { r0 += 1; if r0 == 7 break }
    let m = one(0, 1, 4, |f| {
        let top = f.new_label();
        let end = f.new_label();
        f.const_i64(0, 0);
        f.const_i64(1, 1);
        f.const_i64(2, 7);
        f.bind(top);
        f.op3(Op::I64Add, 0, 0, 1);
        f.op3(Op::I64Eq, 3, 0, 2);
        f.br_if(3, end);
        f.jmp(top);
        f.bind(end);
        f.ret(0);
    });
    assert_eq!(done(&m, &[]), vec![7]);
}

fn cond_fn(op_is_ifz: bool) -> Module {
    // main(x): if cond(x) { return 20 } else { return 10 }
    one(1, 1, 2, |f| {
        let t = f.new_label();
        if op_is_ifz {
            f.br_ifz(0, t);
        } else {
            f.br_if(0, t);
        }
        f.const_i64(1, 10);
        f.ret(1);
        f.bind(t);
        f.const_i64(1, 20);
        f.ret(1);
    })
}

#[test]
fn br_if_taken_and_not_taken() {
    let m = cond_fn(false);
    assert_eq!(done(&m, &[0]), vec![10]);
    assert_eq!(done(&m, &[1]), vec![20]);
    assert_eq!(done(&m, &[0xFFFF_FFFF]), vec![20]);
    assert_eq!(done(&m, &[0x8000_0000]), vec![20]);
}

#[test]
fn br_if_uses_only_low_32_bits() {
    let m = cond_fn(false);
    assert_eq!(done(&m, &[0x1_0000_0000]), vec![10]);
    assert_eq!(done(&m, &[0xFFFF_FFFF_0000_0000]), vec![10]);
    assert_eq!(done(&m, &[0x1_0000_0001]), vec![20]);
    assert_eq!(done(&m, &[u64::MAX]), vec![20]);
}

#[test]
fn br_ifz_taken_and_not_taken() {
    let m = cond_fn(true);
    assert_eq!(done(&m, &[0]), vec![20]);
    assert_eq!(done(&m, &[1]), vec![10]);
    assert_eq!(done(&m, &[0x1_0000_0000]), vec![20]);
    assert_eq!(done(&m, &[0xFFFF_FFFF_0000_0000]), vec![20]);
    assert_eq!(done(&m, &[0x1_0000_0001]), vec![10]);
    assert_eq!(done(&m, &[u64::MAX]), vec![10]);
}

fn table_fn(n: usize) -> Module {
    // main(idx): br_table idx [100, 101, ...] default 999
    one(1, 1, 2, |f| {
        let labels: Vec<Label> = (0..n).map(|_| f.new_label()).collect();
        let def = f.new_label();
        f.br_table(0, &labels, def);
        for (i, l) in labels.iter().enumerate() {
            f.bind(*l);
            f.const_i64(1, 100 + i as i64);
            f.ret(1);
        }
        f.bind(def);
        f.const_i64(1, 999);
        f.ret(1);
    })
}

#[test]
fn br_table_in_range() {
    let m = table_fn(3);
    assert_eq!(done(&m, &[0]), vec![100]);
    assert_eq!(done(&m, &[1]), vec![101]);
    assert_eq!(done(&m, &[2]), vec![102]);
}

#[test]
fn br_table_index_equal_len_is_default() {
    let m = table_fn(3);
    assert_eq!(done(&m, &[3]), vec![999]);
    assert_eq!(done(&m, &[4]), vec![999]);
}

#[test]
fn br_table_huge_indices() {
    let m = table_fn(3);
    assert_eq!(done(&m, &[u32::MAX as u64]), vec![999]);
    // low 32 bits small, upper bits set: index is reg as u32
    assert_eq!(done(&m, &[0x1_0000_0000]), vec![100]);
    assert_eq!(done(&m, &[0x1_0000_0001]), vec![101]);
    assert_eq!(done(&m, &[0xFFFF_FFFF_0000_0002]), vec![102]);
    assert_eq!(done(&m, &[0x1_0000_0005]), vec![999]);
    assert_eq!(done(&m, &[u64::MAX]), vec![999]);
}

#[test]
fn br_table_empty_targets_always_default() {
    let m = table_fn(0);
    for v in [0u64, 1, 2, u32::MAX as u64, u64::MAX] {
        assert_eq!(done(&m, &[v]), vec![999]);
    }
}

#[test]
fn br_table_single_target_and_shared_targets() {
    // two entries pointing at the same label
    let m = one(1, 1, 2, |f| {
        let a = f.new_label();
        let d = f.new_label();
        f.br_table(0, &[a, a], d);
        f.bind(a);
        f.const_i64(1, 1);
        f.ret(1);
        f.bind(d);
        f.const_i64(1, 2);
        f.ret(1);
    });
    assert_eq!(done(&m, &[0]), vec![1]);
    assert_eq!(done(&m, &[1]), vec![1]);
    assert_eq!(done(&m, &[2]), vec![2]);
}

// ---------------------------------------------------------------- calls

#[test]
fn call_args_and_result_window_untouched_outside() {
    let mut mb = ModuleBuilder::new();
    // add(a, b) -> a + b, with scratch registers that the caller must not see
    let mut add = FunctionBuilder::new(4);
    add.op3(Op::I64Add, 0, 0, 1);
    add.const_i64(2, 0xDEAD);
    add.const_i64(3, 0xBEEF);
    add.ret(0);
    let add_id = mb.add_func(ty(2, 1), add);
    // main: r0 below base, window r3..r4 (P=2, R=1), r5/r6 above the window
    let mut m = FunctionBuilder::new(8);
    m.const_i64(0, 111);
    m.const_i64(3, 5);
    m.const_i64(4, 7);
    m.const_i64(5, 555);
    m.const_i64(6, 666);
    m.call(add_id, 3);
    m.mov(1, 3);
    m.mov(2, 4);
    m.mov(7, 5);
    // return [r0, r1(=sum), r2(=r4 unchanged = 7), r7(=r5), r6]
    m.mov(3, 7);
    m.mov(4, 6);
    // pack: r0, r1, r2, r3, r4
    m.ret(0);
    let main = mb.add_func(ty(0, 5), m);
    mb.export_func("main", main);
    let module = mb.finish().unwrap();
    assert_eq!(done(&module, &[]), vec![111, 12, 7, 555, 666]);
}

#[test]
fn call_multiple_results_swap() {
    let mut mb = ModuleBuilder::new();
    let mut swap = FunctionBuilder::new(4);
    swap.mov(2, 1);
    swap.mov(3, 0);
    swap.ret(2); // ret base > 0
    let swap_id = mb.add_func(ty(2, 2), swap);
    let mut m = FunctionBuilder::new(4);
    m.const_i64(1, 10);
    m.const_i64(2, 20);
    m.call(swap_id, 1);
    m.ret(1);
    let main = mb.add_func(ty(0, 2), m);
    mb.export_func("main", main);
    assert_eq!(done(&mb.finish().unwrap(), &[]), vec![20, 10]);
}

#[test]
fn call_more_results_than_params() {
    // P=1, R=3: window is max(P,R)=3 registers; callee returns (x, x+1, x+2)
    let mut mb = ModuleBuilder::new();
    let mut g = FunctionBuilder::new(3);
    g.const_i64(1, 1);
    g.op3(Op::I64Add, 1, 0, 1);
    g.const_i64(2, 2);
    g.op3(Op::I64Add, 2, 0, 2);
    g.ret(0);
    let gid = mb.add_func(ty(1, 3), g);
    let mut m = FunctionBuilder::new(5);
    m.const_i64(0, 99); // below the window
    m.const_i64(4, 77); // above the window (window is r1..r3)
    m.const_i64(1, 40);
    m.call(gid, 1);
    // results r1,r2,r3 then check r4 untouched -> return r1,r2,r3,r4,r0
    m.mov(4, 4);
    m.ret(1);
    let main = mb.add_func(ty(0, 4), m);
    mb.export_func("main", main);
    assert_eq!(done(&mb.finish().unwrap(), &[]), vec![40, 41, 42, 77]);
}

#[test]
fn call_zero_params_zero_results() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(I64, true, 0);
    let mut inc = FunctionBuilder::new(2);
    inc.global_get(0, g);
    inc.const_i64(1, 1);
    inc.op3(Op::I64Add, 0, 0, 1);
    inc.global_set(g, 0);
    inc.ret(0);
    let inc_id = mb.add_func(ty(0, 0), inc);
    let mut m = FunctionBuilder::new(1);
    m.const_i64(0, 123);
    m.call(inc_id, 0);
    m.call(inc_id, 0);
    m.call(inc_id, 0);
    m.global_get(0, g);
    m.ret(0);
    let main = mb.add_func(ty(0, 1), m);
    mb.export_func("main", main);
    mb.export_global("g", g);
    assert_eq!(done(&mb.finish().unwrap(), &[]), vec![3]);
}

#[test]
fn callee_registers_start_at_zero_each_activation() {
    let mut mb = ModuleBuilder::new();
    // dirty(): fills 16 registers with garbage
    let mut dirty = FunctionBuilder::new(16);
    for r in 0..16 {
        dirty.const_i64(r, 0xBEEF);
    }
    dirty.ret(0);
    let dirty_id = mb.add_func(ty(0, 0), dirty);
    // probe(p) -> r1 + r2 + r3 (never written by anyone), then scribbles on r1..r3
    let mut probe = FunctionBuilder::new(4);
    probe.op3(Op::I64Add, 1, 1, 2);
    probe.op3(Op::I64Add, 1, 1, 3);
    probe.mov(0, 1);
    probe.const_i64(1, 0xDEAD);
    probe.const_i64(2, 0xDEAD);
    probe.const_i64(3, 0xDEAD);
    probe.ret(0);
    let probe_id = mb.add_func(ty(1, 1), probe);
    let mut m = FunctionBuilder::new(8);
    for r in 0..8 {
        m.const_i64(r, 0xAAAA);
    }
    m.call(dirty_id, 4);
    m.call(probe_id, 2);
    m.mov(0, 2);
    m.const_i64(2, 1);
    m.call(probe_id, 2);
    m.mov(1, 2);
    m.call(dirty_id, 7);
    m.const_i64(2, 1);
    m.call(probe_id, 2);
    m.mov(2, 2);
    m.ret(0);
    let main = mb.add_func(ty(0, 3), m);
    mb.export_func("main", main);
    assert_eq!(done(&mb.finish().unwrap(), &[]), vec![0, 0, 0]);
}

#[test]
fn call_deep_chain_passes_values() {
    // f_k(x) = f_{k-1}(x + 1); f_0(x) = x ; chain of 50 functions
    let mut mb = ModuleBuilder::new();
    let mut prev = {
        let mut f = FunctionBuilder::new(1);
        f.ret(0);
        mb.add_func(ty(1, 1), f)
    };
    for _ in 0..50 {
        let mut f = FunctionBuilder::new(2);
        f.const_i64(1, 1);
        f.op3(Op::I64Add, 0, 0, 1);
        f.call(prev, 0);
        f.ret(0);
        prev = mb.add_func(ty(1, 1), f);
    }
    mb.export_func("main", prev);
    assert_eq!(done(&mb.finish().unwrap(), &[1000]), vec![1050]);
}

// ---------------------------------------------------------------- recursion

#[test]
fn factorial_20() {
    let mut mb = ModuleBuilder::new();
    let fact = mb.declare_func(ty(1, 1));
    let mut f = FunctionBuilder::new(5);
    let base = f.new_label();
    f.const_i64(1, 1);
    f.const_i64(2, 2);
    f.op3(Op::I64LtS, 3, 0, 2);
    f.br_if(3, base);
    f.op3(Op::I64Sub, 4, 0, 1);
    f.call(fact, 4);
    f.op3(Op::I64Mul, 0, 0, 4);
    f.ret(0);
    f.bind(base);
    f.const_i64(0, 1);
    f.ret(0);
    mb.define(fact, f);
    mb.export_func("main", fact);
    let m = mb.finish().unwrap();
    assert_eq!(done(&m, &[20]), vec![2_432_902_008_176_640_000]);
    assert_eq!(done(&m, &[0]), vec![1]);
    assert_eq!(done(&m, &[1]), vec![1]);
    assert_eq!(done(&m, &[10]), vec![3_628_800]);
}

#[test]
fn fibonacci_20_recursive() {
    let mut mb = ModuleBuilder::new();
    let fib = mb.declare_func(ty(1, 1));
    let mut f = FunctionBuilder::new(5);
    let base = f.new_label();
    f.const_i64(1, 1);
    f.const_i64(2, 2);
    f.op3(Op::I64LtS, 3, 0, 2);
    f.br_if(3, base);
    f.op3(Op::I64Sub, 4, 0, 1);
    f.call(fib, 4);
    f.mov(3, 4);
    f.op3(Op::I64Sub, 4, 0, 2);
    f.call(fib, 4);
    f.op3(Op::I64Add, 0, 3, 4);
    f.ret(0);
    f.bind(base);
    f.ret(0);
    mb.define(fib, f);
    mb.export_func("main", fib);
    let m = mb.finish().unwrap();
    assert_eq!(done(&m, &[20]), vec![6765]);
    assert_eq!(done(&m, &[0]), vec![0]);
    assert_eq!(done(&m, &[1]), vec![1]);
    assert_eq!(done(&m, &[2]), vec![1]);
    assert_eq!(done(&m, &[15]), vec![610]);
}

fn even_odd_module() -> Module {
    let mut mb = ModuleBuilder::new();
    let even = mb.declare_func(ty(1, 1));
    let odd = mb.declare_func(ty(1, 1));
    for (me, other, zero_result) in [(even, odd, 1i64), (odd, even, 0i64)] {
        let mut f = FunctionBuilder::new(3);
        let z = f.new_label();
        f.op2(Op::I64Eqz, 1, 0);
        f.br_if(1, z);
        f.const_i64(1, 1);
        f.op3(Op::I64Sub, 2, 0, 1);
        f.call(other, 2);
        f.mov(0, 2);
        f.ret(0);
        f.bind(z);
        f.const_i64(0, zero_result);
        f.ret(0);
        mb.define(me, f);
    }
    mb.export_func("main", even);
    mb.export_func("odd", odd);
    mb.finish().unwrap()
}

#[test]
fn mutual_recursion_even_odd() {
    let m = even_odd_module();
    assert_eq!(done(&m, &[1000]), vec![1]);
    assert_eq!(done(&m, &[1001]), vec![0]);
    assert_eq!(done(&m, &[0]), vec![1]);
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = inst.spawn("odd", &[1000]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 1_000_000), Exit::Done);
    assert_eq!(inst.results(f), Some(&[0u64][..]));
    let g = inst.spawn("odd", &[999]).unwrap();
    assert_eq!(inst.resume(g, &mut (), 1_000_000), Exit::Done);
    assert_eq!(inst.results(g), Some(&[1u64][..]));
}

// ---------------------------------------------------------------- call_indirect

struct IndirectModule {
    module: Module,
    unary: u32,
}

/// Table: 0 -> add1, 1 -> double, 2 -> None, 3 -> binary fn (wrong type), 4 -> host import add1000.
/// main(idx, x) = call_indirect (i64)->(i64) idx, x
fn indirect_module() -> IndirectModule {
    let mut mb = ModuleBuilder::new();
    let host = mb.import("env", "plus1000", ty(1, 1));
    let mut a = FunctionBuilder::new(2);
    a.const_i64(1, 1);
    a.op3(Op::I64Add, 0, 0, 1);
    a.ret(0);
    let add1 = mb.add_func(ty(1, 1), a);
    let mut d = FunctionBuilder::new(1);
    d.op3(Op::I64Add, 0, 0, 0);
    d.ret(0);
    let double = mb.add_func(ty(1, 1), d);
    let mut b = FunctionBuilder::new(2);
    b.op3(Op::I64Add, 0, 0, 1);
    b.ret(0);
    let binary = mb.add_func(ty(2, 1), b);
    let unary = mb.add_type(ty(1, 1));
    let mut m = FunctionBuilder::new(4);
    m.call_indirect(unary, 0, 1);
    m.ret(1);
    let main = mb.add_func(ty(2, 1), m);
    mb.table(vec![Some(add1), Some(double), None, Some(binary), Some(host)]);
    mb.export_func("main", main);
    IndirectModule { module: mb.finish().unwrap(), unary }
}

fn indirect_imports() -> Imports<()> {
    let mut imp = HostRegistry::<()>::new();
    reg_host(&mut imp, "env", "plus1000", ty(1, 1), |_c, args, res| {
        res[0] = args[0] + 1000;
        Ok(HostOk::Continue)
    });
    imp.imports()
}

fn run_ind(m: &Module, args: &[u64]) -> Exit {
    let mut inst = common::new_instance(m, &indirect_imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", args).unwrap();
    let e = inst.resume(f, &mut (), 100_000);
    if e == Exit::Done {
        // stash result in exit-independent way for callers that need it
    }
    e
}

fn ind_result(m: &Module, args: &[u64]) -> u64 {
    let mut inst = common::new_instance(m, &indirect_imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", args).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100_000), Exit::Done);
    inst.results(f).unwrap()[0]
}

#[test]
fn call_indirect_dispatches_correctly() {
    let im = indirect_module();
    assert_eq!(ind_result(&im.module, &[0, 41]), 42);
    assert_eq!(ind_result(&im.module, &[1, 21]), 42);
    assert_eq!(ind_result(&im.module, &[0, 0]), 1);
}

#[test]
fn call_indirect_index_uses_low_32_bits() {
    let im = indirect_module();
    assert_eq!(ind_result(&im.module, &[0x1_0000_0001, 21]), 42);
    assert_eq!(run_ind(&im.module, &[0x1_0000_0000 + 2, 1]), Exit::Trap(Trap::UndefinedElement));
}

#[test]
fn call_indirect_out_of_range_is_undefined_element() {
    let im = indirect_module();
    for idx in [5u64, 6, 1000, u32::MAX as u64, u64::MAX] {
        assert_eq!(run_ind(&im.module, &[idx, 1]), Exit::Trap(Trap::UndefinedElement), "idx {idx}");
    }
}

#[test]
fn call_indirect_none_entry_is_undefined_element() {
    let im = indirect_module();
    assert_eq!(run_ind(&im.module, &[2, 1]), Exit::Trap(Trap::UndefinedElement));
}

#[test]
fn call_indirect_empty_table_is_undefined_element() {
    let mut mb = ModuleBuilder::new();
    let t = mb.add_type(ty(0, 0));
    let mut m = FunctionBuilder::new(2);
    m.const_i64(0, 0);
    m.call_indirect(t, 0, 1);
    m.ret(0);
    let id = mb.add_func(ty(0, 0), m);
    mb.export_func("main", id);
    assert_eq!(trap_of(&mb.finish().unwrap(), &[]), Trap::UndefinedElement);
}

#[test]
fn call_indirect_wrong_type_mismatch() {
    let im = indirect_module();
    assert_eq!(run_ind(&im.module, &[3, 1]), Exit::Trap(Trap::IndirectCallTypeMismatch));
}

#[test]
fn call_indirect_table_entry_is_import() {
    let im = indirect_module();
    assert_eq!(ind_result(&im.module, &[4, 5]), 1005);
}

#[test]
fn call_indirect_import_wrong_type_mismatch() {
    // call the import (i64)->(i64) with a (i64,i64)->(i64) signature
    let mut mb = ModuleBuilder::new();
    let host = mb.import("env", "plus1000", ty(1, 1));
    let bin = mb.add_type(ty(2, 1));
    let mut m = FunctionBuilder::new(4);
    m.const_i64(0, 0);
    m.call_indirect(bin, 0, 1);
    m.ret(1);
    let id = mb.add_func(ty(0, 1), m);
    mb.table(vec![Some(host)]);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    assert_eq!(run_ind(&module, &[]), Exit::Trap(Trap::IndirectCallTypeMismatch));
}

#[test]
fn call_indirect_structurally_equal_duplicate_types_match() {
    let im = indirect_module();
    let mut data = im.module.data().clone();
    let dup = data.types.len() as u32;
    let t = data.types[im.unary as usize].clone();
    data.types.push(t);
    // point main's call_indirect at the duplicate type index
    let mut patched = 0;
    for f in &mut data.functions {
        for i in &mut f.code {
            if i.op == Op::CallIndirect {
                i.a = dup;
                patched += 1;
            }
        }
    }
    assert_eq!(patched, 1);
    let module = Module::new(data).expect("verifies");
    assert_eq!(ind_result(&module, &[0, 41]), 42);
    assert_eq!(ind_result(&module, &[1, 5]), 10);
    assert_eq!(ind_result(&module, &[4, 1]), 1001);
    // still a mismatch for a genuinely different type
    assert_eq!(run_ind(&module, &[3, 1]), Exit::Trap(Trap::IndirectCallTypeMismatch));
}

#[test]
fn call_indirect_callee_declared_with_duplicate_type() {
    let im = indirect_module();
    let mut data = im.module.data().clone();
    let dup = data.types.len() as u32;
    let t = data.types[im.unary as usize].clone();
    data.types.push(t);
    // function 1 in the function list is add1 (index 0 is main? order: add1, double, binary, main)
    data.functions[0].type_idx = dup;
    let module = Module::new(data).expect("verifies");
    assert_eq!(ind_result(&module, &[0, 41]), 42);
}

#[test]
fn call_indirect_window_preserved() {
    // regs below and above the window keep their values across call_indirect
    let mut mb = ModuleBuilder::new();
    let mut a = FunctionBuilder::new(3);
    a.const_i64(1, 1);
    a.op3(Op::I64Add, 0, 0, 1);
    a.ret(0);
    let add1 = mb.add_func(ty(1, 1), a);
    let t = mb.add_type(ty(1, 1));
    let mut m = FunctionBuilder::new(6);
    m.const_i64(0, 0); // table index
    m.const_i64(1, 11);
    m.const_i64(2, 100); // window = r2
    m.const_i64(3, 33);
    m.call_indirect(t, 0, 2);
    m.mov(0, 1);
    m.mov(1, 2);
    m.mov(2, 3);
    m.ret(0);
    let id = mb.add_func(ty(0, 3), m);
    mb.table(vec![Some(add1)]);
    mb.export_func("main", id);
    assert_eq!(done(&mb.finish().unwrap(), &[]), vec![11, 101, 33]);
}

// ---------------------------------------------------------------- ret

#[test]
fn ret_with_nonzero_base_from_entry() {
    let m = one(0, 2, 5, |f| {
        f.const_i64(3, 30);
        f.const_i64(4, 40);
        f.ret(3);
    });
    assert_eq!(done(&m, &[]), vec![30, 40]);
}

#[test]
fn entry_ret_gives_done_and_results_only_then() {
    let m = one(1, 1, 1, |f| f.ret(0));
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[77]).unwrap();
    assert_eq!(inst.results(f), None);
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(f), Some(&[77u64][..]));
}

#[test]
fn entry_with_zero_results() {
    let m = one(0, 0, 0, |f| f.ret(0));
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(f), Some(&[][..]));
}

// ---------------------------------------------------------------- stack overflow

fn infinite_recursion() -> Module {
    let mut mb = ModuleBuilder::new();
    let f = mb.declare_func(ty(0, 0));
    let mut b = FunctionBuilder::new(1);
    b.call(f, 0);
    b.ret(0);
    mb.define(f, b);
    mb.export_func("main", f);
    mb.finish().unwrap()
}

#[test]
fn infinite_recursion_overflows_default_limits() {
    assert_eq!(run(&infinite_recursion(), &[]).0, Exit::Trap(Trap::StackOverflow));
}

#[test]
fn infinite_recursion_overflows_small_depth() {
    for d in [1u32, 2, 3, 10] {
        let lim = Limits { max_call_depth: d, ..Limits::default() };
        assert_eq!(run_lim(&infinite_recursion(), lim, &[]).0, Exit::Trap(Trap::StackOverflow), "{d}");
    }
}

/// count(n): activations = n + 1 (entry counts as depth 1).
fn depth_module(nregs: u16) -> Module {
    let mut mb = ModuleBuilder::new();
    let f = mb.declare_func(ty(1, 1));
    let mut b = FunctionBuilder::new(nregs);
    let z = b.new_label();
    let arg = nregs - 1;
    b.op2(Op::I64Eqz, 1, 0);
    b.br_if(1, z);
    b.const_i64(1, 1);
    b.op3(Op::I64Sub, arg, 0, 1);
    b.call(f, arg);
    b.ret(arg);
    b.bind(z);
    b.const_i64(0, 0);
    b.ret(0);
    mb.define(f, b);
    mb.export_func("main", f);
    mb.finish().unwrap()
}

#[test]
fn call_depth_boundary() {
    // Rule (interp): a call traps when frames.len() + 2 > max_call_depth, i.e. the entry
    // activation is depth 1 and at most `max_call_depth` activations may be live.
    let m = depth_module(3);
    let lim = Limits { max_call_depth: 10, ..Limits::default() };
    assert_eq!(run_lim(&m, lim, &[8]).0, Exit::Done); // 9 activations
    assert_eq!(run_lim(&m, lim, &[9]).0, Exit::Done); // 10 activations
    assert_eq!(run_lim(&m, lim, &[10]).0, Exit::Trap(Trap::StackOverflow)); // 11 activations
    assert_eq!(run_lim(&m, lim, &[1000]).0, Exit::Trap(Trap::StackOverflow));
    // default limit
    assert_eq!(run(&m, &[4_094]).0, Exit::Done);
    assert_eq!(run(&m, &[4_095]).0, Exit::Done);
    assert_eq!(run(&m, &[4_096]).0, Exit::Trap(Trap::StackOverflow));
}

#[test]
fn call_depth_one_allows_only_entry() {
    let m = depth_module(3);
    let lim = Limits { max_call_depth: 1, ..Limits::default() };
    assert_eq!(run_lim(&m, lim, &[0]).0, Exit::Done);
    assert_eq!(run_lim(&m, lim, &[1]).0, Exit::Trap(Trap::StackOverflow));
}

#[test]
fn call_depth_zero_cannot_spawn() {
    let m = depth_module(3);
    let lim = Limits { max_call_depth: 0, ..Limits::default() };
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), lim).unwrap();
    assert_eq!(inst.spawn("main", &[0]).unwrap_err(), SpawnError::StackOverflow);
}

#[test]
fn max_registers_exceeded_is_stack_overflow() {
    // frames of 100 registers; budget of 1000 registers = 10 frames
    let m = depth_module(100);
    let lim = Limits { max_registers: 1000, ..Limits::default() };
    assert_eq!(run_lim(&m, lim, &[9]).0, Exit::Done); // 10 frames = 1000 regs
    assert_eq!(run_lim(&m, lim, &[10]).0, Exit::Trap(Trap::StackOverflow));
    let lim = Limits { max_registers: 999, ..Limits::default() };
    assert_eq!(run_lim(&m, lim, &[8]).0, Exit::Done);
    assert_eq!(run_lim(&m, lim, &[9]).0, Exit::Trap(Trap::StackOverflow));
}

#[test]
fn max_registers_smaller_than_entry_frame_cannot_spawn() {
    let m = depth_module(100);
    let lim = Limits { max_registers: 99, ..Limits::default() };
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), lim).unwrap();
    assert_eq!(inst.spawn("main", &[0]).unwrap_err(), SpawnError::StackOverflow);
}

// ---------------------------------------------------------------- misc

#[test]
fn unreachable_traps() {
    let m = one(0, 0, 0, |f| f.unreachable());
    assert_eq!(trap_of(&m, &[]), Trap::Unreachable);
}

#[test]
fn unreachable_in_callee_traps_and_skips_rest() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(I64, true, 0);
    let mut c = FunctionBuilder::new(0);
    c.unreachable();
    let cid = mb.add_func(ty(0, 0), c);
    let mut m = FunctionBuilder::new(1);
    m.call(cid, 0);
    m.const_i64(0, 5);
    m.global_set(g, 0);
    m.ret(0);
    let id = mb.add_func(ty(0, 0), m);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    let mut inst = common::new_instance(&module, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Trap(Trap::Unreachable));
    assert_eq!(inst.global(g), Some(0));
}

#[test]
fn nop_does_nothing_and_costs_one() {
    let m = one(0, 1, 1, |f| {
        f.nop();
        f.nop();
        f.const_i64(0, 9);
        f.nop();
        f.ret(0);
    });
    let mut inst = common::new_instance(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[]).unwrap();
    let (e, n) = inst.resume_units(f, &mut (), 100);
    assert_eq!(e, Exit::Done);
    assert_eq!(n, 5);
    assert_eq!(inst.results(f), Some(&[9u64][..]));
}

#[test]
fn instance_usable_after_trap() {
    let mut mb = ModuleBuilder::new();
    let mut bad = FunctionBuilder::new(0);
    bad.unreachable();
    let bad_id = mb.add_func(ty(0, 0), bad);
    let mut good = FunctionBuilder::new(1);
    good.const_i64(0, 42);
    good.ret(0);
    let good_id = mb.add_func(ty(0, 1), good);
    mb.export_func("bad", bad_id);
    mb.export_func("good", good_id);
    let module = mb.finish().unwrap();
    let mut inst = common::new_instance(&module, &Imports::<()>::new(), Limits::default()).unwrap();
    let b = inst.spawn("bad", &[]).unwrap();
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Trap(Trap::Unreachable));
    let g = inst.spawn("good", &[]).unwrap();
    assert_eq!(inst.resume(g, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(g), Some(&[42u64][..]));
    // the dead fiber stays dead
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Trap(Trap::Unreachable));
}

#[test]
fn instance_usable_after_stack_overflow() {
    let mut mb = ModuleBuilder::new();
    let f = mb.declare_func(ty(0, 0));
    let mut b = FunctionBuilder::new(1);
    b.call(f, 0);
    b.ret(0);
    mb.define(f, b);
    let mut ok = FunctionBuilder::new(1);
    ok.const_i64(0, 7);
    ok.ret(0);
    let ok_id = mb.add_func(ty(0, 1), ok);
    mb.export_func("rec", f);
    mb.export_func("ok", ok_id);
    let module = mb.finish().unwrap();
    let mut inst = common::new_instance(&module, &Imports::<()>::new(), Limits::default()).unwrap();
    let a = inst.spawn("rec", &[]).unwrap();
    assert_eq!(inst.resume(a, &mut (), 10_000_000), Exit::Trap(Trap::StackOverflow));
    let b = inst.spawn("ok", &[]).unwrap();
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(b), Some(&[7u64][..]));
}
