//! The backend seam: a test backend that wraps `run_interpreter` must be indistinguishable
//! from the plain interpreter, and fibers must move freely between the two.
mod common;
use common::reg_host;
use efevm::ValType::I64;
use efevm::backend::{Backend, FiberStack, RunCx, run_interpreter};
use efevm::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

fn ty(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![I64; np], vec![I64; nr])
}

/// Counts its calls and checks the contract at entry: a leader, a runnable stack.
struct Counting {
    calls: Arc<AtomicU32>,
    entries: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl Backend<u64> for Counting {
    fn run(&mut self, cx: &mut RunCx<'_, u64>, stack: &mut FiberStack, budget: i64) -> (Exit, i64) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(cx.module.is_leader(stack.func, stack.pc), "backend entered mid-block");
        assert!(budget >= 0);
        assert!(stack.regs.len() as u32 >= stack.fp);
        self.entries.lock().unwrap().push((stack.func, stack.pc));
        run_interpreter(cx, stack, budget)
    }
}

/// main(n): for i in 0..n { acc += helper(i); host.ping(acc); mem[64] = ...; global += 1 }
/// then fill 3000 bytes, yield, host yield, return acc.
fn program() -> Module {
    let mut mb = ModuleBuilder::new();
    let ping = mb.import("env", "ping", ty(1, 1));
    let hy = mb.import("env", "hy", ty(0, 0));
    let g = mb.global(I64, true, 0);
    mb.export_global("count", g);
    // helper(x) = x * 2
    let mut h = FunctionBuilder::new(2);
    h.const_i64(1, 2);
    h.op3(Op::I64Mul, 0, 0, 1);
    h.ret(0);
    let helper = mb.add_func(ty(1, 1), h);
    let mut f = FunctionBuilder::new(8);
    let (top, end) = (f.new_label(), f.new_label());
    f.const_i64(1, 0);
    f.const_i64(2, 0);
    f.const_i64(6, 1);
    f.const_i64(5, 64);
    f.bind(top);
    f.op3(Op::I64GeU, 3, 1, 0);
    f.br_if(3, end);
    f.mov(4, 1);
    f.call(helper, 4);
    f.op3(Op::I64Add, 2, 2, 4);
    f.mov(4, 2);
    f.call(ping, 4);
    f.store(Op::I64Store, 4, 5, 0);
    f.global_get(3, g);
    f.op3(Op::I64Add, 3, 3, 6);
    f.global_set(g, 3);
    f.op3(Op::I64Add, 1, 1, 6);
    f.jmp(top);
    f.bind(end);
    f.const_i64(4, 128);
    f.const_i64(5, 0xAB);
    f.const_i64(3, 3000);
    f.memory_fill(4, 5, 3);
    f.yield_();
    f.call(hy, 4);
    f.ret(2);
    let main = mb.add_func(ty(1, 1), f);
    mb.export_func("main", main);
    mb.memory(1, Some(2));
    mb.finish().unwrap()
}

fn instance() -> Instance<u64> {
    let mut reg = HostRegistry::<u64>::new();
    reg_host(&mut reg, "env", "ping", ty(1, 1), |c, a, r| {
        *c.host() += 1;
        r[0] = a[0] + 1;
        Ok(HostOk::Continue)
    });
    reg_host(&mut reg, "env", "hy", ty(0, 0), |c, _, _| {
        *c.host() += 1000;
        Ok(HostOk::Yield)
    });
    Instance::new(&program(), &reg.imports(), Limits::default()).unwrap()
}

#[derive(Debug, PartialEq)]
struct Trace {
    steps: Vec<(Exit, u64, Option<(u32, u32)>)>,
    results: Option<Vec<u64>>,
    memory: Vec<u8>,
    count: Option<u64>,
    host: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Interpreter,
    Backend,
    /// Toggles between backend and interpreter before every resume.
    Alternate,
}

fn drive(mode: Mode, budget: u64, n: u64) -> (Trace, u32) {
    let mut inst = instance();
    let calls = Arc::new(AtomicU32::new(0));
    let entries = Arc::new(Mutex::new(Vec::new()));
    let backend = || -> Option<Box<dyn Backend<u64>>> {
        Some(Box::new(Counting { calls: calls.clone(), entries: entries.clone() }))
    };
    if mode == Mode::Backend {
        inst.set_backend(backend());
    }
    let mut host = 0u64;
    let f = inst.spawn("main", &[n]).unwrap();
    let mut steps = Vec::new();
    for k in 0..100_000 {
        if mode == Mode::Alternate {
            inst.set_backend(if k % 2 == 0 { backend() } else { None });
        }
        let (e, units) = inst.resume_units(f, &mut host, budget);
        assert!(units <= budget);
        let done = e == Exit::Done;
        steps.push((e, units, inst.fiber_location(f)));
        if done {
            break;
        }
    }
    let t = Trace {
        results: inst.results(f).map(<[u64]>::to_vec),
        memory: inst.memory().as_slice().to_vec(),
        count: inst.export_global("count"),
        host,
        steps,
    };
    (t, calls.load(Ordering::SeqCst))
}

const BUDGETS: [u64; 8] = [1, 2, 5, 17, 100, 1000, 100_000, u64::MAX];

#[test]
fn backend_is_indistinguishable_from_the_interpreter() {
    for budget in BUDGETS {
        let (base, c0) = drive(Mode::Interpreter, budget, 40);
        assert_eq!(c0, 0);
        assert_eq!(base.results, Some(vec![(0..40u64).map(|i| i * 2).sum()]));
        assert_eq!(base.count, Some(40));
        assert!(base.steps.iter().any(|s| s.0 == Exit::Yield), "guest and host yields happen");
        let (with, calls) = drive(Mode::Backend, budget, 40);
        assert_eq!(base, with, "budget {budget}");
        assert!(calls > 0);
        // every resume that is not just paying debt enters the backend at a leader
        assert!(calls as usize <= with.steps.len());
    }
}

#[test]
fn fibers_move_between_backend_and_interpreter() {
    for budget in BUDGETS {
        let (base, _) = drive(Mode::Interpreter, budget, 25);
        let (alt, calls) = drive(Mode::Alternate, budget, 25);
        assert_eq!(base, alt, "budget {budget}");
        if budget < 100_000 {
            assert!(calls > 0);
        }
    }
}

#[test]
fn backend_not_used_after_a_step_into_a_block() {
    let calls = Arc::new(AtomicU32::new(0));
    let entries = Arc::new(Mutex::new(Vec::new()));
    let mut inst = instance();
    inst.set_backend(Some(Box::new(Counting { calls: calls.clone(), entries: entries.clone() })));
    let mut host = 0;
    let f = inst.spawn("main", &[10]).unwrap();
    // stepping never uses the backend, and leaves the fiber at pc 1 (not a leader)
    assert_eq!(inst.step(f, &mut host), Exit::OutOfBudget);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let (func, pc) = inst.fiber_location(f).unwrap();
    assert!(!inst.module().is_leader(func, pc));
    // budget 1: the interpreter runs to the next leader and stops there
    assert_eq!(inst.resume(f, &mut host, 1), Exit::OutOfBudget);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "resume from mid-block goes to the interpreter");
    let (func, pc) = inst.fiber_location(f).unwrap();
    assert!(inst.module().is_leader(func, pc));
    // at a leader (after paying the debt) the backend takes over
    let mut guard = 0;
    loop {
        let e = inst.resume(f, &mut host, 1_000);
        if e == Exit::Done {
            break;
        }
        guard += 1;
        assert!(guard < 100);
    }
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert!(entries.lock().unwrap().iter().all(|&(func, pc)| inst.module().is_leader(func, pc)));
    assert_eq!(inst.results(f), Some(&[90u64][..]));
}

#[test]
fn backend_can_be_removed_and_reports_state() {
    let mut inst = instance();
    assert!(!inst.has_backend());
    inst.set_backend(Some(Box::new(Counting {
        calls: Arc::new(AtomicU32::new(0)),
        entries: Arc::new(Mutex::new(Vec::new())),
    })));
    assert!(inst.has_backend());
    inst.set_backend(None);
    assert!(!inst.has_backend());
}

/// A backend that calls a host function through `RunCx::call_host` gets masked results.
#[test]
fn call_host_masks_results() {
    struct Probe(Arc<Mutex<Option<(Result<HostOk, Trap>, u64)>>>);
    impl Backend<u64> for Probe {
        fn run(&mut self, cx: &mut RunCx<'_, u64>, stack: &mut FiberStack, budget: i64) -> (Exit, i64) {
            let mut res = [0xFFFF_FFFF_FFFF_FFFFu64];
            let r = cx.call_host(0, &[5], &mut res);
            *self.0.lock().unwrap() = Some((r, res[0]));
            run_interpreter(cx, stack, budget)
        }
    }
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "wide", FuncType::new([I64], [ValType::I32]));
    let mut f = FunctionBuilder::new(1);
    f.const_i64(0, 5);
    f.call(h, 0);
    f.ret(0);
    let id = mb.add_func(ty(0, 1), f);
    mb.export_func("main", id);
    let m = mb.finish().unwrap();
    let mut reg = HostRegistry::<u64>::new();
    // the host writes 64 bits into an i32 result; the VM keeps the low half
    reg_host(&mut reg, "env", "wide", FuncType::new([I64], [ValType::I32]), |_, a, r| {
        r[0] = 0xAAAA_BBBB_0000_0000 | a[0];
        Ok(HostOk::Continue)
    });
    let mut inst = Instance::new(&m, &reg.imports(), Limits::default()).unwrap();
    let seen = Arc::new(Mutex::new(None));
    inst.set_backend(Some(Box::new(Probe(seen.clone()))));
    let f = inst.spawn("main", &[]).unwrap();
    assert_eq!(inst.resume(f, &mut 0, 100), Exit::Done);
    assert_eq!(seen.lock().unwrap().take(), Some((Ok(HostOk::Continue), 5)));
    assert_eq!(inst.results(f), Some(&[5u64][..]));
}
