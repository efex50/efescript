//! Bulk memory cost and debt, default limits, fiber stack release, `Instance::call`.
use efevm::ValType::I64;
use efevm::*;

fn ty(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![I64; np], vec![I64; nr])
}

fn module(np: usize, nregs: u16, split: bool, body: impl FnOnce(&mut FunctionBuilder)) -> Module {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(nregs);
    body(&mut f);
    if split {
        let l = f.new_label();
        f.jmp(l);
        f.bind(l);
    }
    f.ret(0);
    let id = mb.add_func(ty(np, 0), f);
    mb.export_func("main", id);
    mb.memory(1, Some(4));
    mb.finish().expect("module verifies")
}

fn inst(m: &Module) -> Instance<()> {
    Instance::new(m, &Imports::<()>::new(), Limits::default()).unwrap()
}

/// `r0 = dst, r1 = src/value, r2 = len`, then the op.
/// Blocks without `split`: `[const, const, const, op, ret]` (cost 5).
/// With `split`: `[const, const, const, op, jmp]` (cost 5) and `[ret]` (cost 1), so there is a
/// leader after the bulk op where an overrun can stop the fiber.
fn bulk(op: Op, len: u64, split: bool) -> Module {
    module(0, 3, split, |f| {
        f.const_i64(0, 0);
        f.const_i64(1, 0);
        f.const_i64(2, len as i64);
        match op {
            Op::MemoryCopy => f.memory_copy(0, 1, 2),
            _ => f.memory_fill(0, 1, 2),
        }
    })
}

/// Blocks without `split`: `[const, grow, ret]` (cost 3). With `split`: `[const, grow, jmp]`
/// (cost 3) and `[ret]` (cost 1).
fn grow(pages: u64, split: bool) -> Module {
    module(0, 2, split, |f| {
        f.const_i64(1, pages as i64);
        f.memory_grow(0, 1);
    })
}

/// Runs the module's `main` with an unlimited budget; returns the units used.
fn units_used(m: &Module) -> u64 {
    let mut i = inst(m);
    let f = i.spawn("main", &[]).unwrap();
    let (e, n) = i.resume_units(f, &mut (), 1 << 50);
    assert_eq!(e, Exit::Done);
    n
}

#[test]
fn bulk_cost_exact() {
    // one block of 5 instructions plus the extra len / 256
    for (len, extra) in [(0u64, 0u64), (255, 0), (256, 1), (511, 1), (512, 2), (1000, 3)] {
        assert_eq!(units_used(&bulk(Op::MemoryCopy, len, false)), 5 + extra, "copy {len}");
        assert_eq!(units_used(&bulk(Op::MemoryFill, len, false)), 5 + extra, "fill {len}");
    }
    // grow: one block of 3 plus pages * 256
    assert_eq!(units_used(&grow(0, false)), 3);
    assert_eq!(units_used(&grow(1, false)), 3 + 256);
    assert_eq!(units_used(&grow(3, false)), 3 + 3 * 256);
    // a failed grow (beyond max 4) is charged for the requested size
    assert_eq!(units_used(&grow(100, false)), 3 + 100 * 256);
    // saturating: an absurd request does not overflow; the whole budget is used up and the
    // fiber is left owing the excess
    let mut i = inst(&grow(u64::MAX, true));
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), u64::MAX), (Exit::OutOfBudget, i64::MAX as u64));
}

#[test]
fn bulk_op_never_exits_immediately() {
    // The copy sits in the middle of a block: even with a budget of 1 the block (cost 5) and
    // the whole bulk operation run, and the fiber stops only at the next leader.
    let m = bulk(Op::MemoryFill, 2560, true);
    let mut i = inst(&m);
    i.memory_mut().write(0, &[0xEE; 8]).unwrap();
    let f = i.spawn("main", &[]).unwrap();
    // fill writes value r1 = 0 over [0, 2560)
    assert_eq!(i.resume_units(f, &mut (), 1), (Exit::OutOfBudget, 1));
    assert_eq!(&i.memory().as_slice()[..8], &[0u8; 8]);
    assert_eq!(i.fiber_location(f), Some((0, 5)));
}

#[test]
fn debt_is_carried_over_resumes() {
    // copy of 2560 bytes: extra 10. Blocks: [3 consts, copy, jmp] (5) and [ret] (1).
    let m = bulk(Op::MemoryCopy, 2560, true);
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    // 8 - 5 = 3, extra 10 -> -7 at the leader of `ret`: stop, debt 7, all 8 units used
    assert_eq!(i.resume_units(f, &mut (), 8), (Exit::OutOfBudget, 8));
    assert_eq!(i.status(f), Some(FiberStatus::Ready));
    // budget smaller than debt: nothing executes, debt 5
    assert_eq!(i.resume_units(f, &mut (), 2), (Exit::OutOfBudget, 2));
    assert_eq!(i.resume_units(f, &mut (), 0), (Exit::OutOfBudget, 0));
    // budget equal to the remaining debt (5): still nothing executes
    assert_eq!(i.resume_units(f, &mut (), 5), (Exit::OutOfBudget, 5));
    // debt is zero: ret runs
    assert_eq!(i.resume_units(f, &mut (), 1), (Exit::Done, 1));
}

#[test]
fn debt_then_larger_budget_runs_the_rest() {
    let m = bulk(Op::MemoryFill, 2560, true);
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 8), (Exit::OutOfBudget, 8)); // debt 7
    // pays 7, then runs ret (cost 1)
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Done, 8));
}

#[test]
fn bulk_op_over_budget_still_completes() {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(3);
    let l = f.new_label();
    f.const_i64(0, 0);
    f.const_i64(1, 0xAB);
    f.const_i64(2, 1000);
    f.memory_fill(0, 1, 2);
    f.jmp(l);
    f.bind(l);
    f.ret(0);
    let id = mb.add_func(ty(0, 0), f);
    mb.export_func("main", id);
    mb.memory(1, Some(1));
    let m = mb.finish().unwrap();
    let mut i = inst(&m);
    let fb = i.spawn("main", &[]).unwrap();
    // block cost 5 and fill's extra 3 exceed the 4 units given: 4 - 5 - 3 = -4
    assert_eq!(i.resume(fb, &mut (), 4), Exit::OutOfBudget);
    assert!(i.memory().as_slice()[..1000].iter().all(|&b| b == 0xAB));
}

#[test]
fn exact_budget_leaves_no_debt() {
    // block 5 + extra 2 equal the budget: the counter is 0, no debt; the stop at the leader
    // of `ret` comes from counter 0
    let m = bulk(Op::MemoryCopy, 512, true);
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 7), (Exit::OutOfBudget, 7));
    assert_eq!(i.resume_units(f, &mut (), 1), (Exit::Done, 1));
}

#[test]
fn trapping_bulk_op_costs_its_block_and_no_debt() {
    for (op, len) in [
        (Op::MemoryCopy, 70_000u64),
        (Op::MemoryFill, 70_000),
        (Op::MemoryCopy, 1 << 40),
        (Op::MemoryFill, u64::MAX),
    ] {
        let m = bulk(op, len, false);
        let mut i = inst(&m);
        let f = i.spawn("main", &[]).unwrap();
        // the whole block (5) is charged on entry; the trapping op adds nothing
        assert_eq!(i.resume_units(f, &mut (), 5), (Exit::Trap(Trap::OutOfBounds), 5), "{op:?} {len}");
        assert_eq!(i.resume_units(f, &mut (), 5), (Exit::Trap(Trap::OutOfBounds), 0));
    }
}

#[test]
fn failed_grow_is_charged() {
    let m = grow(100, true); // max 4 pages: fails, extra 25600
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    // 100 - 3 - 25600 = -25503: stop at the leader of `ret`, debt 25503
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::OutOfBudget, 100));
    assert_eq!(i.resume_units(f, &mut (), 25_503), (Exit::OutOfBudget, 25_503));
    assert_eq!(i.resume_units(f, &mut (), 1), (Exit::Done, 1));
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
    let m = module(0, 2, false, |f| f.yield_());
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
    let after = bk.new_label();
    bk.jmp(after); // a leader after the fill, where the overrun stops the fiber
    bk.bind(after);
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
    // fill of 25600 bytes: block 5 + extra 100 > budget 50: debt, stop at the leader of `ret`
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
    // consts(3), fill of 2560 (extra 10), yield, ret
    let m = module(0, 3, false, |f| {
        f.const_i64(0, 0);
        f.const_i64(1, 0);
        f.const_i64(2, 2560);
        f.memory_fill(0, 1, 2);
        f.yield_();
    });
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 0), (Exit::OutOfBudget, 0));
    // fits in the budget: block 5 + extra 10 = 15 units
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Yield, 15));
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Done, 1));
    assert_eq!(i.resume_units(f, &mut (), 100), (Exit::Done, 0));
    // over budget: blocks [3 consts, fill, yield] (5) and [ret] (1). 8 - 5 - 10 = -7: the yield
    // still runs (same block), all 8 units are consumed and debt 7 is paid in later resumes
    let mut i = inst(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.resume_units(f, &mut (), 8), (Exit::Yield, 8));
    assert_eq!(i.resume_units(f, &mut (), 4), (Exit::OutOfBudget, 4));
    // debt 3 left: paid 3, then ret (1)
    assert_eq!(i.resume_units(f, &mut (), 10), (Exit::Done, 4));
    // total = 8 + 4 + 4 = 16 = 5 + 10 + ret
}
