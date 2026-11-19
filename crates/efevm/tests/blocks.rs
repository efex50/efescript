//! Basic blocks (SPEC section 5): leaders, costs, and `Instance::step`.
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
    mb.memory(1, Some(1));
    mb.finish().expect("module verifies")
}

fn plain(m: &Module) -> Instance<()> {
    Instance::new(m, &Imports::<()>::new(), Limits::default()).unwrap()
}

/// The blocks of function 0 tile the body: they are sorted, contiguous and cover every
/// instruction, and `block_cost` / `is_leader` agree with the list.
fn check_partition(m: &Module, func: u32, len: u32) {
    let blocks = m.blocks(func);
    let mut next = 0;
    for &(pc, cost) in blocks {
        assert_eq!(pc, next);
        assert!(cost >= 1);
        assert_eq!(m.block_cost(func, pc), cost);
        assert!(m.is_leader(func, pc));
        for inner in pc + 1..pc + cost {
            assert_eq!(m.block_cost(func, inner), 0);
            assert!(!m.is_leader(func, inner));
        }
        next = pc + cost;
    }
    assert_eq!(next, len);
    assert_eq!(m.block_cost(func, len), 0);
    assert_eq!(m.block_cost(func, u32::MAX), 0);
}

#[test]
fn one_straight_block() {
    let m = one(0, 0, 1, |f| {
        f.nop();
        f.nop();
        f.ret(0);
    });
    assert_eq!(m.blocks(0), &[(0, 3)]);
    check_partition(&m, 0, 3);
}

#[test]
fn branch_targets_and_fallthrough_are_leaders() {
    let m = one(0, 0, 1, |f| {
        let l = f.new_label();
        f.const_i32(0, 1); // 0
        f.br_if(0, l); // 1
        f.nop(); // 2
        f.nop(); // 3
        f.bind(l);
        f.nop(); // 4: target
        f.ret(0); // 5
    });
    assert_eq!(m.blocks(0), &[(0, 2), (2, 2), (4, 2)]);
    check_partition(&m, 0, 6);
}

#[test]
fn backward_branch_and_br_ifz() {
    let m = one(0, 0, 1, |f| {
        let top = f.new_label();
        f.const_i32(0, 3); // 0
        f.bind(top);
        f.nop(); // 1: loop head
        f.br_ifz(0, top); // 2
        f.ret(0); // 3: after the branch
    });
    assert_eq!(m.blocks(0), &[(0, 1), (1, 2), (3, 1)]);
    check_partition(&m, 0, 4);
}

#[test]
fn unreachable_code_after_a_terminator_is_its_own_block() {
    let m = one(0, 0, 1, |f| {
        let l = f.new_label();
        f.jmp(l); // 0
        f.nop(); // 1: dead, but a leader (after jmp)
        f.nop(); // 2
        f.bind(l);
        f.ret(0); // 3
        f.nop(); // 4: dead, leader after ret
        f.unreachable(); // 5
    });
    assert_eq!(m.blocks(0), &[(0, 1), (1, 2), (3, 1), (4, 2)]);
    check_partition(&m, 0, 6);
}

#[test]
fn br_table_targets_and_default_are_leaders() {
    let m = one(0, 0, 1, |f| {
        let (a, b, d) = (f.new_label(), f.new_label(), f.new_label());
        f.const_i32(0, 1); // 0
        f.br_table(0, &[a, b, a], d); // 1
        f.nop(); // 2: after br_table
        f.nop(); // 3
        f.bind(a);
        f.nop(); // 4
        f.nop(); // 5
        f.bind(b);
        f.nop(); // 6
        f.bind(d);
        f.ret(0); // 7: default
    });
    assert_eq!(m.blocks(0), &[(0, 2), (2, 2), (4, 2), (6, 1), (7, 1)]);
    check_partition(&m, 0, 8);
}

#[test]
fn br_table_default_only_target() {
    let m = one(0, 0, 1, |f| {
        let d = f.new_label();
        f.const_i32(0, 1); // 0
        f.br_table(0, &[], d); // 1
        f.bind(d);
        f.ret(0); // 2: target and fallthrough at once
    });
    assert_eq!(m.blocks(0), &[(0, 2), (2, 1)]);
}

#[test]
fn leaders_after_call_call_indirect_yield_and_ret() {
    let mut mb = ModuleBuilder::new();
    let callee = mb.declare_func(ty(0, 0));
    let mut c = FunctionBuilder::new(0);
    c.ret(0);
    mb.define(callee, c);
    mb.table(vec![Some(callee)]);
    let t = mb.add_type(ty(0, 0));
    let mut f = FunctionBuilder::new(2);
    f.call(callee, 1); // 0
    f.nop(); // 1: after call
    f.const_i32(0, 0); // 2
    f.call_indirect(t, 0, 1); // 3
    f.nop(); // 4: after call_indirect
    f.yield_(); // 5
    f.nop(); // 6: after yield
    f.nop(); // 7
    f.ret(0); // 8
    f.nop(); // 9: after ret
    f.ret(0); // 10
    let main = mb.add_func(ty(0, 0), f);
    mb.export_func("main", main);
    let m = mb.finish().unwrap();
    assert_eq!(m.blocks(callee), &[(0, 1)]);
    assert_eq!(m.blocks(main), &[(0, 1), (1, 3), (4, 2), (6, 3), (9, 2)]);
    check_partition(&m, main, 11);
}

#[test]
fn indices_are_in_the_whole_function_space() {
    let mut mb = ModuleBuilder::new();
    let imp = mb.import("env", "h", ty(0, 0));
    let mut f = FunctionBuilder::new(0);
    f.nop();
    f.ret(0);
    let main = mb.add_func(ty(0, 0), f);
    let m = mb.finish().unwrap();
    assert_eq!((imp, main), (0, 1));
    // an import has no blocks; neither does an index out of range
    assert!(m.blocks(imp).is_empty());
    assert_eq!(m.block_cost(imp, 0), 0);
    assert!(!m.is_leader(imp, 0));
    assert!(m.blocks(7).is_empty());
    assert_eq!(m.block_cost(7, 0), 0);
    assert_eq!(m.blocks(main), &[(0, 2)]);
    assert!(m.is_leader(main, 0) && !m.is_leader(main, 1));
}

// ------------------------------------------------------------------ step

#[test]
fn step_executes_one_instruction_and_charges_nothing() {
    let m = one(0, 1, 1, |f| {
        f.nop(); // 0
        f.const_i64(0, 7); // 1
        f.ret(0); // 2
    });
    let mut i = plain(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget);
    assert_eq!(i.status(f), Some(FiberStatus::Ready));
    assert_eq!(i.fiber_location(f), Some((0, 1)));
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget);
    assert_eq!(i.fiber_location(f), Some((0, 2)));
    assert_eq!(i.fiber_registers(f), Some(&[7u64][..]));
    assert_eq!(i.step(f, &mut ()), Exit::Done);
    assert_eq!(i.results(f), Some(&[7u64][..]));
    assert_eq!(i.step(f, &mut ()), Exit::Done);
}

#[test]
fn step_into_the_middle_of_a_block_then_resume_skips_the_charge() {
    let m = one(0, 1, 1, |f| {
        f.nop(); // 0
        f.nop(); // 1
        f.const_i64(0, 7); // 2
        f.ret(0); // 3
    });
    let mut i = plain(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget);
    // pc 1 is not a leader: the rest of the block runs without charging, even with budget 0
    assert_eq!(i.resume_units(f, &mut (), 0), (Exit::Done, 0));
    assert_eq!(i.results(f), Some(&[7u64][..]));
}

#[test]
fn step_pays_no_debt() {
    // [fill, jmp] overruns the budget; the debt stays while stepping and is paid by a resume
    let m = one(0, 0, 3, |f| {
        let l = f.new_label();
        f.const_i64(2, 2560);
        f.memory_fill(0, 1, 2);
        f.jmp(l);
        f.bind(l);
        f.nop();
        f.ret(0);
    });
    let mut i = plain(&m);
    let f = i.spawn("main", &[]).unwrap();
    // block 3 + extra 10 against budget 2: overrun 11 -> debt 11
    assert_eq!(i.resume_units(f, &mut (), 2), (Exit::OutOfBudget, 2));
    assert_eq!(i.fiber_location(f), Some((0, 3)));
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget); // nop ran
    assert_eq!(i.fiber_location(f), Some((0, 4)));
    // the debt is still there: a resume with 10 only pays it down
    assert_eq!(i.resume_units(f, &mut (), 10), (Exit::OutOfBudget, 10));
    assert_eq!(i.resume_units(f, &mut (), 1), (Exit::OutOfBudget, 1));
    assert_eq!(i.resume_units(f, &mut (), 5), (Exit::Done, 0));
}

#[test]
fn step_through_calls_yield_and_traps() {
    let mut mb = ModuleBuilder::new();
    let mut c = FunctionBuilder::new(1);
    c.ret(0);
    let callee = mb.add_func(ty(0, 0), c);
    let mut f = FunctionBuilder::new(1);
    f.call(callee, 0); // 0
    f.yield_(); // 1
    f.unreachable(); // 2
    let main = mb.add_func(ty(0, 0), f);
    mb.export_func("main", main);
    let m = mb.finish().unwrap();
    let mut i = plain(&m);
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget); // call
    assert_eq!(i.fiber_location(f), Some((callee, 0)));
    assert_eq!(i.fiber_backtrace(f), vec![(callee, 0), (main, 0)]);
    assert_eq!(i.step(f, &mut ()), Exit::OutOfBudget); // ret
    assert_eq!(i.fiber_location(f), Some((main, 1)));
    assert_eq!(i.step(f, &mut ()), Exit::Yield);
    assert_eq!(i.status(f), Some(FiberStatus::Suspended));
    assert_eq!(i.step(f, &mut ()), Exit::Trap(Trap::Unreachable));
    assert_eq!(i.trap_location(f), Some((main, 2)));
    assert_eq!(i.step(f, &mut ()), Exit::Trap(Trap::Unreachable));
    // a removed fiber answers like `resume` does
    assert!(i.remove_fiber(f));
    assert_eq!(i.step(f, &mut ()), Exit::Trap(Trap::Host("invalid fiber".into())));
}
