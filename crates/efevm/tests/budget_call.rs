//! Bulk memory cost and debt, default limits, fiber stack release, `Instance::call`.
use efevm::ValType::I64;
use efevm::*;

fn ty(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![I64; np], vec![I64; nr])
}

fn module(np: usize, nregs: u16, body: impl FnOnce(&mut FunctionBuilder)) -> Module {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(nregs);
    body(&mut f);
    f.ret(0);
    let id = mb.add_func(ty(np, 0), f);
    mb.export_func("main", id);
    mb.memory(1, Some(4));
    mb.finish().expect("module verifies")
}

fn inst(m: &Module) -> Instance<()> {
    Instance::new(m, &Imports::<()>::new(), Limits::default()).unwrap()
}

/// `r0 = dst, r1 = src/value, r2 = len`, then the op (3 consts + op + ret).
fn bulk(op: Op, len: u64) -> Module {
    module(0, 3, |f| {
        f.const_i64(0, 0);
        f.const_i64(1, 0);
        f.const_i64(2, len as i64);
        match op {
            Op::MemoryCopy => f.memory_copy(0, 1, 2),
            _ => f.memory_fill(0, 1, 2),
        }
    })
}

fn grow(pages: u64) -> Module {
    module(0, 2, |f| {
        f.const_i64(1, pages as i64);
        f.memory_grow(0, 1);
    })
}

/// Smallest budget with which one resume ends in `Done` (binary search; monotonic).
fn min_done(m: &Module) -> u64 {
    let (mut lo, mut hi) = (0u64, 1u64 << 60);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let mut i = inst(m);
        let f = i.spawn("main", &[]).unwrap();
        if i.resume(f, &mut (), mid) == Exit::Done {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

#[test]
fn bulk_cost_exact() {
    // other instructions: 3 consts + ret = 4
    for (len, cost) in [(0u64, 1u64), (255, 1), (256, 2), (511, 2), (512, 3), (1000, 4)] {
        assert_eq!(min_done(&bulk(Op::MemoryCopy, len)), 4 + cost, "copy {len}");
        assert_eq!(min_done(&bulk(Op::MemoryFill, len)), 4 + cost, "fill {len}");
    }
    // grow: 1 const + ret = 2 other instructions; cost 1 + pages * 256
    assert_eq!(min_done(&grow(0)), 2 + 1);
    assert_eq!(min_done(&grow(1)), 2 + 257);
    assert_eq!(min_done(&grow(3)), 2 + 1 + 3 * 256);
    // a failed grow (beyond max 4) is charged for the requested size
    assert_eq!(min_done(&grow(100)), 2 + 1 + 100 * 256);
    // saturating: an absurd request does not overflow; the fiber is left owing the excess
    let mut i = inst(&grow(u64::MAX));
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_counted(f, &mut (), u64::MAX), (Exit::OutOfBudget, 2));
}

#[test]
fn debt_is_carried_over_resumes() {
    // copy of 2560 bytes: cost 11, extra 10
    let m = bulk(Op::MemoryCopy, 2560);
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    // 3 consts + copy's own 1 = 4 instructions; 4 more units left, extra 10 -> debt 6
    assert_eq!(i.resume_counted(f, &mut (), 8), (Exit::OutOfBudget, 4));
    assert_eq!(i.status(f), Some(FiberStatus::Ready));
    // budget smaller than debt: nothing executes
    assert_eq!(i.resume_counted(f, &mut (), 2), (Exit::OutOfBudget, 0));
    assert_eq!(i.resume_counted(f, &mut (), 0), (Exit::OutOfBudget, 0));
    // budget equal to the remaining debt (4): still nothing executes
    assert_eq!(i.resume_counted(f, &mut (), 4), (Exit::OutOfBudget, 0));
    // debt is zero: ret runs
    assert_eq!(i.resume_counted(f, &mut (), 1), (Exit::Done, 1));
}

#[test]
fn debt_then_larger_budget_runs_the_rest() {
    let m = bulk(Op::MemoryFill, 2560);
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_counted(f, &mut (), 8), (Exit::OutOfBudget, 4)); // debt 6
    assert_eq!(i.resume_counted(f, &mut (), 100), (Exit::Done, 1));
}

#[test]
fn bulk_op_over_budget_still_completes() {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(3);
    f.const_i64(0, 0);
    f.const_i64(1, 0xAB);
    f.const_i64(2, 1000);
    f.memory_fill(0, 1, 2);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    mb.memory(1, Some(1));
    let m = mb.finish().unwrap();
    let mut i = inst(&m);
    let fb = i.spawn("main", &[]).unwrap();
    // fill's cost 4 > the 1 unit left after the consts
    assert_eq!(i.resume(fb, &mut (), 4), Exit::OutOfBudget);
    assert!(i.memory().as_slice()[..1000].iter().all(|&b| b == 0xAB));
}

#[test]
fn exact_budget_leaves_no_debt() {
    // cost exactly equals the budget left: no debt, OutOfBudget comes from budget 0 next
    let m = bulk(Op::MemoryCopy, 512); // cost 3
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_counted(f, &mut (), 6), (Exit::OutOfBudget, 4));
    assert_eq!(i.resume_counted(f, &mut (), 1), (Exit::Done, 1));
}

#[test]
fn trapping_bulk_op_costs_one_and_no_debt() {
    for (op, len) in [
        (Op::MemoryCopy, 70_000u64),
        (Op::MemoryFill, 70_000),
        (Op::MemoryCopy, 1 << 40),
        (Op::MemoryFill, u64::MAX),
    ] {
        let m = bulk(op, len);
        let mut i = inst(&m);
        let f = i.spawn("main", &[]).unwrap();
        // 3 consts + the trapping op = 4 units exactly, no OutOfBudget
        assert_eq!(i.resume_counted(f, &mut (), 4), (Exit::Trap(Trap::OutOfBounds), 4), "{op:?} {len}");
        assert_eq!(i.resume_counted(f, &mut (), 4), (Exit::Trap(Trap::OutOfBounds), 0));
    }
}

#[test]
fn failed_grow_is_charged() {
    let m = grow(100); // max 4 pages: fails, cost 1 + 25600
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_counted(f, &mut (), 100), (Exit::OutOfBudget, 2)); // const + grow
    assert_eq!(i.resume_counted(f, &mut (), 25_600 - 98), (Exit::OutOfBudget, 0));
    // debt was 25600 - 98 (units left after const and grow's own 1 are 98)
    assert_eq!(i.resume_counted(f, &mut (), 1), (Exit::Done, 1));
}

#[test]
fn limits_defaults() {
    let l = Limits::default();
    assert_eq!(l.max_memory_pages, 1024);
    assert_eq!(l.max_call_depth, 4096);
    assert_eq!(l.max_registers, 262_144);
    assert_eq!(l.max_fibers, 256);
}

// ------------------------------------------------------------ stack release

fn nested_ok() -> Module {
    let mut mb = ModuleBuilder::new();
    let callee = mb.declare_func(ty(0, 1));
    let mut c = FunctionBuilder::new(8);
    c.const_i64(0, 5);
    c.ret(0);
    mb.define(callee, c);
    let mut f = FunctionBuilder::new(16);
    f.call(callee, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let mut t = FunctionBuilder::new(16);
    t.call(callee, 4);
    t.unreachable();
    let tid = mb.add_func(ty(0, 0), t);
    mb.export_func("trap", tid);
    mb.finish().unwrap()
}

#[test]
fn stack_released_after_done() {
    let mut i = inst(&nested_ok());
    let f = i.spawn("main", &[]).unwrap();
    assert!(i.fiber_stack_capacity(f).unwrap() > 0);
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Done);
    assert_eq!(i.fiber_stack_capacity(f), Some(0));
    assert_eq!(i.results(f), Some(&[5u64][..]));
    assert_eq!(i.status(f), Some(FiberStatus::Done));
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Done);
    assert_eq!(i.results(f), Some(&[5u64][..]));
}

#[test]
fn stack_released_after_trap() {
    let mut i = inst(&nested_ok());
    let f = i.spawn("trap", &[]).unwrap();
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Trap(Trap::Unreachable));
    assert_eq!(i.fiber_stack_capacity(f), Some(0));
    assert_eq!(i.status(f), Some(FiberStatus::Trapped(Trap::Unreachable)));
    assert_eq!(i.trap_location(f), Some((2, 1)));
    assert_eq!(i.backtrace(f).unwrap().len(), 1);
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Trap(Trap::Unreachable));
    assert_eq!(i.results(f), None);
}

#[test]
fn stack_kept_while_suspended() {
    let m = module(0, 2, |f| f.yield_());
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume(f, &mut (), 10), Exit::Yield);
    assert!(i.fiber_stack_capacity(f).unwrap() > 0);
}

// ------------------------------------------------------------------- call

fn call_module() -> Module {
    let mut mb = ModuleBuilder::new();
    let callee = mb.declare_func(ty(1, 1));
    let mut c = FunctionBuilder::new(4);
    c.op3(Op::I64Add, 0, 0, 0);
    c.unreachable();
    mb.define(callee, c);
    let mut add = FunctionBuilder::new(3);
    add.op3(Op::I64Add, 2, 0, 1);
    add.ret(2);
    let add_id = mb.add_func(ty(2, 1), add);
    mb.export_func("add", add_id);
    let mut tr = FunctionBuilder::new(2);
    tr.const_i64(1, 3);
    tr.call(callee, 1);
    tr.ret(0);
    let tid = mb.add_func(ty(0, 0), tr);
    mb.export_func("trap", tid);
    let mut lp = FunctionBuilder::new(1);
    let l = lp.new_label();
    lp.bind(l);
    lp.jmp(l);
    let lid = mb.add_func(ty(0, 0), lp);
    mb.export_func("spin", lid);
    let mut y = FunctionBuilder::new(1);
    y.yield_();
    y.ret(0);
    let yid = mb.add_func(ty(0, 0), y);
    mb.export_func("yield", yid);
    let mut bk = FunctionBuilder::new(3);
    bk.const_i64(0, 0);
    bk.const_i64(1, 0);
    bk.const_i64(2, 25_600);
    bk.memory_fill(0, 1, 2);
    bk.ret(0);
    let bid = mb.add_func(ty(0, 0), bk);
    mb.export_func("fill", bid);
    mb.memory(1, Some(1));
    mb.finish().unwrap()
}

#[test]
fn call_success() {
    let mut i = inst(&call_module());
    assert_eq!(i.call("add", &[2, 40], &mut (), 100), Ok(vec![42]));
}

#[test]
fn call_trap_keeps_backtrace_after_removal() {
    let mut i = inst(&call_module());
    match i.call("trap", &[], &mut (), 100) {
        Err(CallError::Trap { trap, location, backtrace }) => {
            assert_eq!(trap, Trap::Unreachable);
            assert_eq!(location, Some((0, 1)));
            // callee's trap site, then the call instruction in "trap"
            assert_eq!(backtrace, vec![(0, 1), (2, 1)]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn call_out_of_budget_yield_and_spawn_errors() {
    let mut i = inst(&call_module());
    assert_eq!(i.call("spin", &[], &mut (), 1000), Err(CallError::OutOfBudget));
    assert_eq!(i.call("add", &[1, 2], &mut (), 0), Err(CallError::OutOfBudget));
    assert_eq!(i.call("yield", &[], &mut (), 100), Err(CallError::Yield));
    // fill of 25600 bytes costs 101, budget 50: debt, reported as OutOfBudget
    assert_eq!(i.call("fill", &[], &mut (), 50), Err(CallError::OutOfBudget));
    assert_eq!(i.call("nope", &[], &mut (), 10), Err(CallError::Spawn(SpawnError::NoSuchExport)));
    assert_eq!(
        i.call("add", &[1], &mut (), 10),
        Err(CallError::Spawn(SpawnError::ArgCount { expected: 2, got: 1 }))
    );
    assert!(!format!("{}", CallError::Yield).is_empty());
}

#[test]
fn call_frees_the_fiber_slot() {
    let limits = Limits { max_fibers: 2, ..Limits::default() };
    let mut i = Instance::new(&call_module(), &Imports::<()>::new(), limits).unwrap();
    for n in 0..20u64 {
        assert_eq!(i.call("add", &[n, 1], &mut (), 100), Ok(vec![n + 1]));
        assert!(matches!(i.call("trap", &[], &mut (), 100), Err(CallError::Trap { .. })));
        assert_eq!(i.call("spin", &[], &mut (), 10), Err(CallError::OutOfBudget));
        assert_eq!(i.call("yield", &[], &mut (), 10), Err(CallError::Yield));
    }
    // both slots are still free
    let a = i.spawn("add", &[1, 1]).unwrap();
    let b = i.spawn("add", &[1, 1]).unwrap();
    assert_eq!(i.spawn("add", &[1, 1]), Err(SpawnError::TooManyFibers));
    assert!(i.remove_fiber(a) && i.remove_fiber(b));
}

#[test]
fn resume_units_accounts_bulk_yield_and_debt() {
    // consts(3), fill of 2560 (cost 11), yield, ret
    let m = module(0, 3, |f| {
        f.const_i64(0, 0);
        f.const_i64(1, 0);
        f.const_i64(2, 2560);
        f.memory_fill(0, 1, 2);
        f.yield_();
    });
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 0), (Exit::OutOfBudget, 0));
    // fits in the budget: 3 + 11 + yield(1) = 15 units
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Yield, 15));
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Done, 1));
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Done, 0));
    // over budget: all 8 units consumed, debt 6 paid in later resumes and counted
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 8), (Exit::OutOfBudget, 8));
    assert_eq!(i.resume_units(f, &mut (), 4), (Exit::OutOfBudget, 4));
    // debt 2 left: paid 2, then yield (1)
    assert_eq!(i.resume_units(f, &mut (), 10), (Exit::Yield, 3));
    assert_eq!(i.resume_units(f, &mut (), 10), (Exit::Done, 1));
    // total = 8 + 4 + 3 + 1 = 16 = 3 + 11 + yield + ret
}
