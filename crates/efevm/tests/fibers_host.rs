//! Budget, yield, fiber lifecycle, host calls and linking (SPEC sections 7, 10).
mod common;
use common::reg_host;
use efevm::ValType::I64;
use efevm::*;

fn ty(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![I64; np], vec![I64; nr])
}

fn one(np: usize, nr: usize, nregs: u16, body: impl FnOnce(&mut FunctionBuilder)) -> Module {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(nregs);
    body(&mut f);
    let id = mb.add_func(ty(np, nr), f);
    mb.export_func("main", id);
    mb.finish().expect("module verifies")
}

fn plain(m: &Module) -> Instance<()> {
    Instance::new(m, &Imports::<()>::new(), Limits::default()).unwrap()
}

// ---------------------------------------------------------------- budget

/// main(n): acc = n + (n-1) + ... + 1
fn sum_loop() -> Module {
    one(1, 1, 3, |f| {
        let top = f.new_label();
        f.const_i64(1, 0);
        f.const_i64(2, 1);
        f.bind(top);
        f.op3(Op::I64Add, 1, 1, 0);
        f.op3(Op::I64Sub, 0, 0, 2);
        f.br_if(0, top);
        f.ret(1);
    })
}

#[test]
fn endless_loop_out_of_budget_and_resumable() {
    let m = one(0, 0, 0, |f| {
        let top = f.new_label();
        f.bind(top);
        f.jmp(top);
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    for _ in 0..5 {
        assert_eq!(inst.resume(f, &mut (), 1000), Exit::OutOfBudget);
        assert_eq!(inst.status(f), Some(FiberStatus::Ready));
        assert_eq!(inst.results(f), None);
    }
    let (e, n) = inst.resume_counted(f, &mut (), 12345);
    assert_eq!((e, n), (Exit::OutOfBudget, 12345));
}

#[test]
fn counting_loop_same_result_with_any_budget() {
    let m = sum_loop();
    let expected: u64 = (1..=100).sum();
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[100]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 1_000_000), Exit::Done);
    assert_eq!(inst.results(f), Some(&[expected][..]));
    for budget in [1u64, 2, 3, 4, 5, 7, 11, 64, 100, 299] {
        let f = inst.spawn("main", &[100]).unwrap();
        let mut resumes = 0u64;
        let mut total = 0u64;
        loop {
            let (e, n) = inst.resume_counted(f, &mut (), budget);
            assert!(n <= budget);
            total += n;
            resumes += 1;
            match e {
                Exit::Done => break,
                Exit::OutOfBudget => assert_eq!(n, budget, "budget {budget}"),
                other => panic!("unexpected {other:?}"),
            }
            assert!(resumes < 10_000);
        }
        assert_eq!(inst.results(f), Some(&[expected][..]), "budget {budget}");
        // 2 setup + 3 per iteration * 100 + ret = 303 instructions
        assert_eq!(total, 303, "budget {budget}");
        inst.remove_fiber(f);
    }
}

fn straight(n: usize) -> Module {
    // n instructions in total: (n-2) nops, const, ret
    one(0, 1, 1, |f| {
        for _ in 0..n - 2 {
            f.nop();
        }
        f.const_i64(0, 5);
        f.ret(0);
    })
}

#[test]
fn resume_counted_budget_zero_executes_nothing() {
    let m = straight(4);
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume_counted(f, &mut (), 0), (Exit::OutOfBudget, 0));
    assert_eq!(inst.resume_counted(f, &mut (), 0), (Exit::OutOfBudget, 0));
    assert_eq!(inst.resume_counted(f, &mut (), 4), (Exit::Done, 4));
}

#[test]
fn resume_counted_exact_budget_boundaries() {
    for n in [2usize, 3, 4, 10, 50] {
        let m = straight(n);
        let mut inst = plain(&m);
        // budget N -> Done in one go
        let f = inst.spawn("main", &[]).unwrap();
        assert_eq!(inst.resume_counted(f, &mut (), n as u64), (Exit::Done, n as u64));
        assert_eq!(inst.results(f), Some(&[5u64][..]));
        // budget N + 100 still counts only N
        let f = inst.spawn("main", &[]).unwrap();
        assert_eq!(inst.resume_counted(f, &mut (), n as u64 + 100), (Exit::Done, n as u64));
        // budget N-1 -> OutOfBudget, then 1 more -> Done
        let f = inst.spawn("main", &[]).unwrap();
        assert_eq!(
            inst.resume_counted(f, &mut (), n as u64 - 1),
            (Exit::OutOfBudget, n as u64 - 1)
        );
        assert_eq!(inst.results(f), None);
        assert_eq!(inst.resume_counted(f, &mut (), 1), (Exit::Done, 1));
        assert_eq!(inst.results(f), Some(&[5u64][..]));
    }
}

#[test]
fn trap_instruction_counts_toward_budget() {
    // nop, nop, unreachable: traps on the third instruction
    let m = one(0, 0, 0, |f| {
        f.nop();
        f.nop();
        f.unreachable();
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume_counted(f, &mut (), 2), (Exit::OutOfBudget, 2));
    let (e, n) = inst.resume_counted(f, &mut (), 5);
    assert_eq!(e, Exit::Trap(Trap::Unreachable));
    assert!(n <= 1, "executed {n}");
}

#[test]
fn calls_cost_one_each() {
    // callee: ret (1); main: call, ret (2) => 3 total
    let mut mb = ModuleBuilder::new();
    let mut c = FunctionBuilder::new(0);
    c.ret(0);
    let cid = mb.add_func(ty(0, 0), c);
    let mut m = FunctionBuilder::new(0);
    m.call(cid, 0);
    m.ret(0);
    let id = mb.add_func(ty(0, 0), m);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    let mut inst = plain(&module);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume_counted(f, &mut (), 100), (Exit::Done, 3));
}

// ---------------------------------------------------------------- yield

#[test]
fn yield_suspends_and_resume_continues() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(I64, true, 0);
    let mut f = FunctionBuilder::new(2);
    f.const_i64(0, 1);
    f.global_set(g, 0);
    f.yield_();
    f.const_i64(0, 2);
    f.global_set(g, 0);
    f.yield_();
    f.const_i64(0, 3);
    f.global_set(g, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    let mut inst = plain(&module);
    let fb = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.status(fb), Some(FiberStatus::Ready));
    assert_eq!(inst.resume(fb, &mut (), 100), Exit::Yield);
    assert_eq!(inst.status(fb), Some(FiberStatus::Suspended));
    assert_eq!(inst.global(g), Some(1));
    assert_eq!(inst.results(fb), None);
    assert_eq!(inst.resume(fb, &mut (), 100), Exit::Yield);
    assert_eq!(inst.global(g), Some(2));
    assert_eq!(inst.resume(fb, &mut (), 100), Exit::Done);
    assert_eq!(inst.global(g), Some(3));
    assert_eq!(inst.status(fb), Some(FiberStatus::Done));
}

#[test]
fn yield_costs_one_and_zero_budget_does_not_yield() {
    let m = one(0, 0, 0, |f| {
        f.yield_();
        f.ret(0);
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume_counted(f, &mut (), 0), (Exit::OutOfBudget, 0));
    assert_eq!(inst.resume_counted(f, &mut (), 10), (Exit::Yield, 1));
    assert_eq!(inst.resume_counted(f, &mut (), 10), (Exit::Done, 1));
}

#[test]
fn yield_from_nested_call_resumes_in_callee() {
    let mut mb = ModuleBuilder::new();
    let mut c = FunctionBuilder::new(2);
    c.const_i64(1, 1);
    c.yield_();
    c.op3(Op::I64Add, 0, 0, 1);
    c.ret(0);
    let cid = mb.add_func(ty(1, 1), c);
    let mut m = FunctionBuilder::new(2);
    m.const_i64(1, 100);
    m.call(cid, 1);
    m.ret(1);
    let id = mb.add_func(ty(0, 1), m);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    let mut inst = plain(&module);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Yield);
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(f), Some(&[101u64][..]));
}

/// step(id): 3 times { append id to the memory log; yield }. Log counter at address 0 (i64),
/// entries are bytes at 8 + index.
fn logger_module() -> Module {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let mut f = FunctionBuilder::new(5);
    let top = f.new_label();
    f.const_i64(1, 3);
    f.const_i64(3, 1);
    f.const_i64(4, 0);
    f.bind(top);
    f.load(Op::I64Load, 2, 4, 0);
    f.store(Op::I64Store8, 0, 2, 8);
    f.op3(Op::I64Add, 2, 2, 3);
    f.store(Op::I64Store, 2, 4, 0);
    f.yield_();
    f.op3(Op::I64Sub, 1, 1, 3);
    f.br_if(1, top);
    f.ret(0);
    let id = mb.add_func(ty(1, 0), f);
    mb.export_func("step", id);
    mb.finish().unwrap()
}

fn log_of(inst: &Instance<()>) -> Vec<u8> {
    let mem = inst.memory().as_slice();
    let n = u64::from_le_bytes(mem[0..8].try_into().unwrap()) as usize;
    mem[8..8 + n].to_vec()
}

#[test]
fn two_fibers_interleave_through_yield() {
    let m = logger_module();
    let mut inst = plain(&m);
    let a = inst.spawn("step", &[1]).unwrap();
    let b = inst.spawn("step", &[2]).unwrap();
    let mut order = Vec::new();
    for _ in 0..3 {
        assert_eq!(inst.resume(a, &mut (), 1000), Exit::Yield);
        assert_eq!(inst.resume(b, &mut (), 1000), Exit::Yield);
        order.push(log_of(&inst));
    }
    assert_eq!(order[0], vec![1, 2]);
    assert_eq!(order[1], vec![1, 2, 1, 2]);
    assert_eq!(order[2], vec![1, 2, 1, 2, 1, 2]);
    assert_eq!(inst.resume(a, &mut (), 1000), Exit::Done);
    assert_eq!(inst.resume(b, &mut (), 1000), Exit::Done);
    assert_eq!(log_of(&inst), vec![1, 2, 1, 2, 1, 2]);
}

#[test]
fn interleaving_follows_resume_order() {
    let m = logger_module();
    let mut inst = plain(&m);
    let a = inst.spawn("step", &[1]).unwrap();
    let b = inst.spawn("step", &[2]).unwrap();
    let c = inst.spawn("step", &[3]).unwrap();
    // a a b c b a c c b  (each fiber yields exactly 3 times)
    let order = [a, a, b, c, b, a, c, c, b];
    for f in order {
        assert_eq!(inst.resume(f, &mut (), 1000), Exit::Yield);
    }
    assert_eq!(log_of(&inst), vec![1, 1, 2, 3, 2, 1, 3, 3, 2]);
    for f in [a, b, c] {
        assert_eq!(inst.resume(f, &mut (), 1000), Exit::Done);
    }
}

#[test]
fn interleaving_pseudo_random_schedule() {
    let m = logger_module();
    let mut inst = plain(&m);
    let ids: Vec<FiberId> = (1..=4u64).map(|i| inst.spawn("step", &[i]).unwrap()).collect();
    let mut left = [3u32; 4];
    let mut expect = Vec::new();
    let mut x = 0x2545F4914F6CDD1Du64;
    while left.iter().any(|&l| l > 0) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let k = (x % 4) as usize;
        if left[k] == 0 {
            continue;
        }
        left[k] -= 1;
        assert_eq!(inst.resume(ids[k], &mut (), 1000), Exit::Yield);
        expect.push(k as u8 + 1);
    }
    assert_eq!(log_of(&inst), expect);
    for f in ids {
        assert_eq!(inst.resume(f, &mut (), 1000), Exit::Done);
    }
}

#[test]
fn spawn_same_export_twice_with_different_args() {
    let m = one(2, 1, 2, |f| {
        f.op3(Op::I64Sub, 0, 0, 1);
        f.ret(0);
    });
    let mut inst = plain(&m);
    let a = inst.spawn("main", &[10, 3]).unwrap();
    let b = inst.spawn("main", &[100, 1]).unwrap();
    assert_ne!(a, b);
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Done);
    assert_eq!(inst.resume(a, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(a), Some(&[7u64][..]));
    assert_eq!(inst.results(b), Some(&[99u64][..]));
}

#[test]
fn fibers_have_separate_registers_but_share_globals() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(I64, true, 0);
    let mut f = FunctionBuilder::new(3);
    // r1 = param + 1000; yield; global += r1; return r1
    f.const_i64(2, 1000);
    f.op3(Op::I64Add, 1, 0, 2);
    f.yield_();
    f.global_get(2, g);
    f.op3(Op::I64Add, 2, 2, 1);
    f.global_set(g, 2);
    f.ret(1);
    let id = mb.add_func(ty(1, 1), f);
    mb.export_func("main", id);
    let module = mb.finish().unwrap();
    let mut inst = plain(&module);
    let a = inst.spawn("main", &[1]).unwrap();
    let b = inst.spawn("main", &[2]).unwrap();
    assert_eq!(inst.resume(a, &mut (), 100), Exit::Yield);
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Yield);
    assert_eq!(inst.resume(b, &mut (), 100), Exit::Done);
    assert_eq!(inst.resume(a, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(a), Some(&[1001u64][..]));
    assert_eq!(inst.results(b), Some(&[1002u64][..]));
    assert_eq!(inst.global(g), Some(2003));
}

// ---------------------------------------------------------------- lifecycle

#[test]
fn status_transitions_and_results() {
    let m = one(0, 1, 1, |f| {
        f.yield_();
        f.const_i64(0, 8);
        f.ret(0);
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.status(f), Some(FiberStatus::Ready));
    assert_eq!(inst.results(f), None);
    assert_eq!(inst.resume(f, &mut (), 0), Exit::OutOfBudget);
    assert_eq!(inst.status(f), Some(FiberStatus::Ready));
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Yield);
    assert_eq!(inst.status(f), Some(FiberStatus::Suspended));
    assert_eq!(inst.results(f), None);
    assert_eq!(inst.resume(f, &mut (), 1), Exit::OutOfBudget);
    assert_eq!(inst.status(f), Some(FiberStatus::Ready));
    assert_eq!(inst.results(f), None);
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.status(f), Some(FiberStatus::Done));
    assert_eq!(inst.results(f), Some(&[8u64][..]));
}

#[test]
fn done_fiber_resumed_again_stays_done() {
    let m = one(0, 1, 1, |f| {
        f.const_i64(0, 8);
        f.ret(0);
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    for budget in [0u64, 1, 100] {
        assert_eq!(inst.resume_counted(f, &mut (), budget), (Exit::Done, 0));
        assert_eq!(inst.status(f), Some(FiberStatus::Done));
        assert_eq!(inst.results(f), Some(&[8u64][..]));
    }
}

#[test]
fn trapped_fiber_resumed_again_gives_same_trap() {
    let m = one(0, 0, 1, |f| {
        f.const_i64(0, 1);
        f.op3(Op::I64DivU, 0, 0, 0);
        f.const_i64(0, 0);
        f.op3(Op::I64DivU, 0, 0, 0);
        f.ret(0);
    });
    // first div is 1/1 fine, second 0/0 traps
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Trap(Trap::DivideByZero));
    assert_eq!(inst.status(f), Some(FiberStatus::Trapped(Trap::DivideByZero)));
    assert_eq!(inst.results(f), None);
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Trap(Trap::DivideByZero));
    assert_eq!(inst.resume_counted(f, &mut (), 0), (Exit::Trap(Trap::DivideByZero), 0));
}

#[test]
fn remove_fiber_then_resume() {
    let m = one(0, 1, 1, |f| {
        f.const_i64(0, 8);
        f.ret(0);
    });
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert!(inst.remove_fiber(f));
    assert_eq!(inst.status(f), None);
    assert_eq!(inst.results(f), None);
    assert!(matches!(inst.resume(f, &mut (), 100), Exit::Trap(Trap::Host(_))));
    assert!(!inst.remove_fiber(f));
    // a new fiber reusing the slot does not revive the stale id
    let g = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.status(f), None);
    assert!(matches!(inst.resume(f, &mut (), 100), Exit::Trap(Trap::Host(_))));
    assert_eq!(inst.resume(g, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(g), Some(&[8u64][..]));
}

#[test]
fn remove_done_fiber() {
    let m = one(0, 0, 0, |f| f.ret(0));
    let mut inst = plain(&m);
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 10), Exit::Done);
    assert!(inst.remove_fiber(f));
    assert_eq!(inst.results(f), None);
}

#[test]
fn max_fibers_limit() {
    let m = one(0, 0, 0, |f| f.ret(0));
    let lim = Limits { max_fibers: 2, ..Limits::default() };
    let mut inst = Instance::<()>::new(&m, &Imports::new(), lim).unwrap();
    let a = inst.spawn("main", &[]).unwrap();
    let b = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.spawn("main", &[]).unwrap_err(), SpawnError::TooManyFibers);
    // done fibers still count until removed
    assert_eq!(inst.resume(a, &mut (), 10), Exit::Done);
    assert_eq!(inst.spawn("main", &[]).unwrap_err(), SpawnError::TooManyFibers);
    assert!(inst.remove_fiber(a));
    let c = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.spawn("main", &[]).unwrap_err(), SpawnError::TooManyFibers);
    assert!(inst.remove_fiber(b));
    assert!(inst.remove_fiber(c));
    inst.spawn("main", &[]).unwrap();
    inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.spawn("main", &[]).unwrap_err(), SpawnError::TooManyFibers);
}

#[test]
fn max_fibers_zero() {
    let m = one(0, 0, 0, |f| f.ret(0));
    let lim = Limits { max_fibers: 0, ..Limits::default() };
    let mut inst = Instance::<()>::new(&m, &Imports::new(), lim).unwrap();
    assert_eq!(inst.spawn("main", &[]).unwrap_err(), SpawnError::TooManyFibers);
}

#[test]
fn spawn_errors() {
    let mut mb = ModuleBuilder::new();
    let host = mb.import("env", "h", ty(0, 0));
    let g = mb.global(I64, false, 1);
    let mut f = FunctionBuilder::new(2);
    f.ret(0);
    let id = mb.add_func(ty(2, 0), f);
    mb.export_func("main", id);
    mb.export_global("g", g);
    mb.export_func("imp", host);
    let module = mb.finish().unwrap();
    let mut imports = HostRegistry::<()>::new();
    reg_host(&mut imports, "env", "h", ty(0, 0), |_c, _a, _r| Ok(HostOk::Continue));
    let mut inst = Instance::new(&module, &imports.imports(), Limits::default()).unwrap();
    assert_eq!(inst.spawn("nope", &[]).unwrap_err(), SpawnError::NoSuchExport);
    assert_eq!(inst.spawn("g", &[]).unwrap_err(), SpawnError::NotAFunction);
    assert_eq!(inst.spawn("imp", &[]).unwrap_err(), SpawnError::ImportEntry);
    assert_eq!(inst.spawn_func(host, &[]).unwrap_err(), SpawnError::ImportEntry);
    assert_eq!(
        inst.spawn("main", &[]).unwrap_err(),
        SpawnError::ArgCount { expected: 2, got: 0 }
    );
    assert_eq!(
        inst.spawn("main", &[1, 2, 3]).unwrap_err(),
        SpawnError::ArgCount { expected: 2, got: 3 }
    );
    // failed spawns do not consume fiber slots
    for _ in 0..3 {
        inst.spawn("main", &[1, 2]).unwrap();
    }
    assert_eq!(inst.export_global("g"), Some(1));
}

// ---------------------------------------------------------------- host calls

#[derive(Default)]
struct St {
    calls: Vec<Vec<u64>>,
    text: String,
    errs: Vec<Trap>,
    count: u32,
}

fn run_st(m: &Module, imports: &Imports<St>, st: &mut St, args: &[u64]) -> (Exit, Vec<u64>) {
    let mut inst = Instance::new(m, imports, Limits::default()).unwrap();
    let f = inst.spawn("main", args).unwrap();
    let e = inst.resume(f, st, 100_000);
    (e, inst.results(f).map(|r| r.to_vec()).unwrap_or_default())
}

#[test]
fn host_args_results_and_state() {
    let mut mb = ModuleBuilder::new();
    let mix = mb.import("env", "mix", ty(3, 2));
    let mut f = FunctionBuilder::new(6);
    f.const_i64(0, 55); // below the window
    f.const_i64(1, 2);
    f.const_i64(2, 3);
    f.const_i64(3, 4);
    f.const_i64(4, 66); // above the window
    f.call(mix, 1);
    f.call(mix, 1); // args are now (9, 24, 4)
    f.mov(5, 4);
    f.ret(0);
    let id = mb.add_func(ty(0, 6), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "mix", ty(3, 2), |c, a, r| {
        c.host().calls.push(a.to_vec());
        r[0] = a[0] + a[1] + a[2];
        r[1] = a[0] * a[1] * a[2];
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    let (e, res) = run_st(&m, &imports.imports(), &mut st, &[]);
    assert_eq!(e, Exit::Done);
    // after 1st call: r1=9 r2=24 r3=4; 2nd: r1=37 r2=864 r3=4
    assert_eq!(res, vec![55, 37, 864, 4, 66, 66]);
    assert_eq!(st.calls, vec![vec![2, 3, 4], vec![9, 24, 4]]);
}

#[test]
fn host_zero_args_zero_results() {
    let mut mb = ModuleBuilder::new();
    let tick = mb.import("env", "tick", ty(0, 0));
    let mut f = FunctionBuilder::new(1);
    f.const_i64(0, 7);
    f.call(tick, 0);
    f.call(tick, 0);
    f.call(tick, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "tick", ty(0, 0), |c, a, r| {
        assert!(a.is_empty() && r.is_empty());
        c.host().count += 1;
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    assert_eq!(run_st(&m, &imports.imports(), &mut st, &[]), (Exit::Done, vec![7]));
    assert_eq!(st.count, 3);
}

#[test]
fn host_results_start_zeroed_and_unset_results_are_zero() {
    // the host writes nothing; the result register must read as 0 (not the old argument)
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "noop", ty(1, 1));
    let mut f = FunctionBuilder::new(1);
    f.const_i64(0, 99);
    f.call(h, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "noop", ty(1, 1), |_c, _a, _r| Ok(HostOk::Continue));
    let mut st = St::default();
    let (e, res) = run_st(&m, &imports.imports(), &mut st, &[]);
    assert_eq!(e, Exit::Done);
    assert_eq!(res, vec![0]);
}

#[test]
fn host_reads_string_written_by_guest() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let print = mb.import("env", "print", ty(2, 0));
    let mut f = FunctionBuilder::new(4);
    f.const_i64(0, 100);
    f.const_i64(1, u64::from_le_bytes(*b"hello wo") as i64);
    f.store(Op::I64Store, 1, 0, 0);
    f.const_i64(1, u32::from_le_bytes(*b"rld!") as i64);
    f.store(Op::I32Store, 1, 0, 8);
    f.const_i64(2, 100);
    f.const_i64(3, 12);
    f.call(print, 2);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "print", ty(2, 0), |c, a, _r| {
        let mut buf = vec![0u8; a[1] as usize];
        c.read(a[0], &mut buf)?;
        c.host().text.push_str(std::str::from_utf8(&buf).unwrap());
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    assert_eq!(run_st(&m, &imports.imports(), &mut st, &[]).0, Exit::Done);
    assert_eq!(st.text, "hello world!");
}

#[test]
fn host_writes_memory_guest_loads() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let fill = mb.import("env", "fill", ty(1, 0));
    let mut f = FunctionBuilder::new(3);
    f.const_i64(0, 200);
    f.call(fill, 0);
    f.load(Op::I64Load, 1, 0, 0);
    f.load(Op::I32Load8U, 2, 0, 3);
    f.ret(1);
    let id = mb.add_func(ty(0, 2), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "fill", ty(1, 0), |c, a, _r| {
        c.write(a[0], b"ABCDEFGH")?;
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    let (e, res) = run_st(&m, &imports.imports(), &mut st, &[]);
    assert_eq!(e, Exit::Done);
    assert_eq!(res, vec![u64::from_le_bytes(*b"ABCDEFGH"), b'D' as u64]);
}

#[test]
fn host_out_of_range_access_gives_out_of_bounds() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let peek = mb.import("env", "peek", ty(2, 1));
    let poke = mb.import("env", "poke", ty(2, 1));
    let mut f = FunctionBuilder::new(3);
    f.call(peek, 0);
    f.ret(0);
    let id = mb.add_func(ty(2, 1), f);
    mb.export_func("main", id);
    let mut g = FunctionBuilder::new(3);
    g.call(poke, 0);
    g.ret(0);
    let id = mb.add_func(ty(2, 1), g);
    mb.export_func("poke", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "peek", ty(2, 1), |c, a, r| {
        let mut buf = vec![0u8; a[1] as usize];
        match c.read(a[0], &mut buf) {
            Ok(()) => r[0] = 1,
            Err(e) => {
                c.host().errs.push(e);
                r[0] = 0;
            }
        }
        Ok(HostOk::Continue)
    });
    reg_host(&mut imports, "env", "poke", ty(2, 1), |c, a, r| {
        let buf = vec![9u8; a[1] as usize];
        c.write(a[0], &buf)?;
        r[0] = 1;
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    let cases: [(u64, u64, u64); 7] = [
        (0, 8, 1),
        (65528, 8, 1),
        (65536, 0, 1),
        (65528, 9, 0),
        (65536, 1, 0),
        (u64::MAX, 1, 0),
        (u64::MAX - 3, 8, 0),
    ];
    for (addr, len, ok) in cases {
        let before = st.errs.len();
        let (e, res) = run_st(&m, &imports.imports(), &mut st, &[addr, len]);
        assert_eq!((e, res), (Exit::Done, vec![ok]), "peek {addr} {len}");
        assert_eq!(st.errs.len() - before, (ok == 0) as usize);
    }
    assert!(st.errs.iter().all(|e| *e == Trap::OutOfBounds));
    // propagating the error with `?` traps the fiber with OutOfBounds
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let f = inst.spawn("poke", &[65535, 2]).unwrap();
    assert_eq!(inst.resume(f, &mut st, 100), Exit::Trap(Trap::OutOfBounds));
    // and in-range writes land in memory
    let f = inst.spawn("poke", &[65534, 2]).unwrap();
    assert_eq!(inst.resume(f, &mut st, 100), Exit::Done);
    assert_eq!(&inst.memory().as_slice()[65534..], &[9, 9]);
}

#[test]
fn host_failed_write_does_not_modify_memory() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let poke = mb.import("env", "poke", ty(2, 1));
    let mut g = FunctionBuilder::new(3);
    g.call(poke, 0);
    g.ret(0);
    let id = mb.add_func(ty(2, 1), g);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "poke", ty(2, 1), |c, a, r| {
        let buf = vec![9u8; a[1] as usize];
        c.write(a[0], &buf)?;
        r[0] = 1;
        Ok(HostOk::Continue)
    });
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[65535, 2]).unwrap();
    let mut st = St::default();
    assert_eq!(inst.resume(f, &mut st, 100), Exit::Trap(Trap::OutOfBounds));
    assert_eq!(inst.memory().as_slice()[65535], 0);
}

#[test]
fn host_trap_boom() {
    let mut mb = ModuleBuilder::new();
    let boom = mb.import("env", "boom", ty(0, 0));
    let mut f = FunctionBuilder::new(1);
    f.call(boom, 0);
    f.const_i64(0, 1);
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "boom", ty(0, 0), |_c, _a, _r| Err(Trap::Host("boom".to_string())));
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let mut st = St::default();
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut st, 100), Exit::Trap(Trap::Host("boom".into())));
    assert_eq!(inst.status(f), Some(FiberStatus::Trapped(Trap::Host("boom".into()))));
    assert_eq!(inst.resume(f, &mut st, 100), Exit::Trap(Trap::Host("boom".into())));
    // the instance is still usable
    let g = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(g, &mut st, 100), Exit::Trap(Trap::Host("boom".into())));
}

#[test]
fn host_yield_writes_results_first_and_resume_continues() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(I64, true, 0);
    let ask = mb.import("env", "ask", ty(1, 1));
    let mut f = FunctionBuilder::new(3);
    f.const_i64(1, 1);
    f.global_set(g, 1);
    f.const_i64(0, 10);
    f.call(ask, 0); // -> 11, yields
    f.mov(2, 0);
    f.call(ask, 0); // -> 12, yields
    f.const_i64(1, 2);
    f.global_set(g, 1);
    f.op3(Op::I64Add, 0, 0, 2); // 12 + 11
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "ask", ty(1, 1), |c, a, r| {
        c.host().count += 1;
        r[0] = a[0] + 1;
        Ok(HostOk::Yield)
    });
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let mut st = St::default();
    let fb = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(fb, &mut st, 100), Exit::Yield);
    assert_eq!(st.count, 1);
    assert_eq!(inst.status(fb), Some(FiberStatus::Suspended));
    assert_eq!(inst.global(g), Some(1));
    assert_eq!(inst.resume(fb, &mut st, 100), Exit::Yield);
    assert_eq!(st.count, 2);
    assert_eq!(inst.global(g), Some(1));
    assert_eq!(inst.resume(fb, &mut st, 100), Exit::Done);
    assert_eq!(inst.global(g), Some(2));
    // second call received the first result (11) as its argument => 12; sum = 12 + 11
    assert_eq!(inst.results(fb), Some(&[23u64][..]));
}

#[test]
fn host_yield_costs_one_like_any_call() {
    let mut mb = ModuleBuilder::new();
    let y = mb.import("env", "y", ty(0, 0));
    let mut f = FunctionBuilder::new(0);
    f.call(y, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "y", ty(0, 0), |_c, _a, _r| Ok(HostOk::Yield));
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let mut st = St::default();
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume_counted(f, &mut st, 100), (Exit::Yield, 1));
    assert_eq!(inst.resume_counted(f, &mut st, 100), (Exit::Done, 1));
}

#[test]
fn host_reads_and_sets_globals() {
    let mut mb = ModuleBuilder::new();
    let g0 = mb.global(I64, true, 5);
    let g1 = mb.global(I64, false, 9);
    let getg = mb.import("env", "getg", ty(1, 1));
    let setg = mb.import("env", "setg", ty(2, 1));
    let mut f = FunctionBuilder::new(8);
    // read g1 (immutable) and a missing global
    f.const_i64(0, 1);
    f.call(getg, 0);
    f.mov(4, 0);
    f.const_i64(0, 77);
    f.call(getg, 0);
    f.mov(5, 0);
    // set g0 = 123 (ok), g1 = 5 (immutable -> host reports 0), g7 = 1 (missing)
    f.const_i64(0, 0);
    f.const_i64(1, 123);
    f.call(setg, 0);
    f.mov(6, 0);
    f.const_i64(0, 1);
    f.const_i64(1, 5);
    f.call(setg, 0);
    f.mov(7, 0);
    f.global_get(0, g0);
    f.global_get(1, g1);
    // r0 = g0, r1 = g1, r2/r3 free, r4..r7 as collected
    f.ret(0);
    let id = mb.add_func(ty(0, 8), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "getg", ty(1, 1), |c, a, r| {
        r[0] = c.global(a[0] as u32).unwrap_or(u64::MAX);
        Ok(HostOk::Continue)
    });
    reg_host(&mut imports, "env", "setg", ty(2, 1), |c, a, r| {
        match c.set_global(a[0] as u32, a[1]) {
            Ok(()) => r[0] = 1,
            Err(e) => {
                c.host().errs.push(e);
                r[0] = 0;
            }
        }
        Ok(HostOk::Continue)
    });
    let mut st = St::default();
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut st, 1000), Exit::Done);
    assert_eq!(inst.results(f), Some(&[123, 9, 0, 0, 9, u64::MAX, 1, 0][..]));
    assert_eq!(st.errs.len(), 1);
    assert!(matches!(st.errs[0], Trap::Host(_)));
    assert_eq!(inst.global(g0), Some(123));
    assert_eq!(inst.global(g1), Some(9));
}

#[test]
fn host_set_immutable_or_missing_global_propagated_traps_fiber() {
    let mut mb = ModuleBuilder::new();
    let _g0 = mb.global(I64, false, 9);
    let setg = mb.import("env", "setg", ty(2, 0));
    let mut f = FunctionBuilder::new(2);
    f.call(setg, 0);
    f.ret(0);
    let id = mb.add_func(ty(2, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "setg", ty(2, 0), |c, a, _r| {
        c.set_global(a[0] as u32, a[1])?;
        Ok(HostOk::Continue)
    });
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let mut st = St::default();
    for idx in [0u64, 1, 1000] {
        let f = inst.spawn("main", &[idx, 5]).unwrap();
        assert!(matches!(inst.resume(f, &mut st, 100), Exit::Trap(Trap::Host(_))), "idx {idx}");
    }
    assert_eq!(inst.global(0), Some(9));
}

#[test]
fn host_state_persists_across_fibers_and_resumes() {
    let mut mb = ModuleBuilder::new();
    let tick = mb.import("env", "tick", ty(0, 0));
    let mut f = FunctionBuilder::new(0);
    f.call(tick, 0);
    f.yield_();
    f.call(tick, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<St>::new();
    reg_host(&mut imports, "env", "tick", ty(0, 0), |c, _a, _r| {
        c.host().count += 1;
        Ok(HostOk::Continue)
    });
    let mut inst = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let mut st = St::default();
    let a = inst.spawn("main", &[]).unwrap();
    let b = inst.spawn("main", &[]).unwrap();
    inst.resume(a, &mut st, 100);
    inst.resume(b, &mut st, 100);
    assert_eq!(st.count, 2);
    inst.resume(a, &mut st, 100);
    inst.resume(b, &mut st, 100);
    assert_eq!(st.count, 4);
}

// ---------------------------------------------------------------- linking

fn import_module() -> Module {
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "f", FuncType::new([I64], [I64]));
    let mut f = FunctionBuilder::new(1);
    f.call(h, 0);
    f.ret(0);
    let id = mb.add_func(ty(1, 1), f);
    mb.export_func("main", id);
    mb.finish().unwrap()
}

fn link_err(m: &Module, imports: &Imports<()>, limits: Limits) -> LoadError {
    match Instance::new(m, imports, limits) {
        Ok(_) => panic!("expected link failure"),
        Err(e) => e,
    }
}

#[test]
fn link_missing_import() {
    let m = import_module();
    let e = link_err(&m, &Imports::new(), Limits::default());
    assert_eq!(
        e,
        LoadError::Link(LinkError::MissingImport { namespace: "env".into(), name: "f".into() })
    );
    // right name, wrong namespace; right namespace, wrong name
    let mut imp = HostRegistry::<()>::new();
    reg_host(&mut imp, "other", "f", ty(1, 1), |_c, a, r| {
        r[0] = a[0];
        Ok(HostOk::Continue)
    });
    reg_host(&mut imp, "env", "g", ty(1, 1), |_c, a, r| {
        r[0] = a[0];
        Ok(HostOk::Continue)
    });
    assert_eq!(
        link_err(&m, &imp.imports(), Limits::default()),
        LoadError::Link(LinkError::MissingImport { namespace: "env".into(), name: "f".into() })
    );
}

#[test]
fn link_import_type_mismatch() {
    let m = import_module();
    let wrong: [(Vec<ValType>, Vec<ValType>); 4] = [
        (vec![], vec![I64]),
        (vec![I64, I64], vec![I64]),
        (vec![I64], vec![]),
        (vec![ValType::I32], vec![I64]),
    ];
    for (p, r) in wrong {
        let mut imp = HostRegistry::<()>::new();
        reg_host(&mut imp, "env", "f", FuncType::new(p.clone(), r.clone()), |_c, _a, _r| {
            Ok(HostOk::Continue)
        });
        assert_eq!(
            link_err(&m, &imp.imports(), Limits::default()),
            LoadError::Link(LinkError::ImportTypeMismatch {
                namespace: "env".into(),
                name: "f".into(),
                module: "(i64) -> i64".into(),
                host: FuncType::new(p.clone(), r.clone()).to_string(),
            }),
            "{p:?} -> {r:?}"
        );
    }
    let mut imp = HostRegistry::<()>::new();
    reg_host(&mut imp, "env", "f", FuncType::new([ValType::I32], [I64]), |_c, _a, _r| Ok(HostOk::Continue));
    reg_host(&mut imp, "env", "f", FuncType::new([I64], [I64]), |_c, a, r| {
        r[0] = a[0] * 2;
        Ok(HostOk::Continue)
    });
    // the later registration replaces the earlier one
    let mut inst = Instance::new(&m, &imp.imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[21]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(f), Some(&[42u64][..]));
}

#[test]
fn link_extra_host_functions_are_ignored() {
    let m = import_module();
    let mut imp = HostRegistry::<()>::new();
    reg_host(&mut imp, "env", "f", ty(1, 1), |_c, a, r| {
        r[0] = a[0] + 1;
        Ok(HostOk::Continue)
    });
    reg_host(&mut imp, "env", "unused", ty(5, 5), |_c, _a, _r| Ok(HostOk::Continue));
    let mut inst = Instance::new(&m, &imp.imports(), Limits::default()).unwrap();
    let f = inst.spawn("main", &[1]).unwrap();
    assert_eq!(inst.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(inst.results(f), Some(&[2u64][..]));
}

#[test]
fn link_memory_too_large() {
    let mut mb = ModuleBuilder::new();
    mb.memory(10, None);
    let mut f = FunctionBuilder::new(0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let lim = Limits { max_memory_pages: 5, ..Limits::default() };
    assert_eq!(
        link_err(&m, &Imports::new(), lim),
        LoadError::Link(LinkError::MemoryTooLarge { min_pages: 10, cap: 5 })
    );
    // exactly at the cap is fine
    let lim = Limits { max_memory_pages: 10, ..Limits::default() };
    let inst = Instance::new(&m, &Imports::<()>::new(), lim).unwrap();
    assert_eq!(inst.memory().size_pages(), 10);
    // with a declared max above the cap and min within the cap: ok
    let mut mb = ModuleBuilder::new();
    mb.memory(2, Some(100));
    let mut f = FunctionBuilder::new(0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m2 = mb.finish().unwrap();
    let lim = Limits { max_memory_pages: 5, ..Limits::default() };
    assert!(Instance::new(&m2, &Imports::<()>::new(), lim).is_ok());
}

#[test]
fn instances_of_one_module_are_independent() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    let g = mb.global(I64, true, 0);
    let mut f = FunctionBuilder::new(2);
    f.const_i64(0, 5);
    f.global_set(g, 0);
    f.const_i64(1, 0);
    f.store(Op::I64Store, 0, 1, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut a = plain(&m);
    let b = plain(&m);
    let f = a.spawn("main", &[]).unwrap();
    assert_eq!(a.resume(f, &mut (), 100), Exit::Done);
    assert_eq!(a.global(g), Some(5));
    assert_eq!(b.global(g), Some(0));
    assert_eq!(b.memory().as_slice()[0], 0);
    assert_eq!(a.memory().as_slice()[0], 5);
}
