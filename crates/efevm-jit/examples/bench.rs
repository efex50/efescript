//! Counting-loop benchmark mirroring `crates/efevm/examples/bench.rs`: ns per instruction for
//! the interpreter and the JIT (`cargo run -p efevm-jit --release --example bench [iterations]`).
//! Instruction counts are the interpreter's (3 per loop iteration), so both columns are
//! directly comparable. A second program (recursive fib) exercises calls.

use std::time::Instant;

use efevm::{Exit, FuncType, FunctionBuilder, Imports, Instance, Limits, ModuleBuilder, Op, ValType};
use efevm_jit::{JitOptions, enable};

const REPS: usize = 5;
const DEFAULT_ITERS: i64 = 100_000_000;
const RESUME_BUDGET: u64 = 10_000;

fn count_module() -> efevm::Module {
    let mut mb = ModuleBuilder::new();
    let ty = FuncType::new([ValType::I64], [ValType::I64]);
    let mut f = FunctionBuilder::new(4);
    f.const_i64(1, 0);
    f.const_i64(2, 1);
    let lp = f.new_label();
    f.bind(lp);
    f.op3(Op::I64Add, 1, 1, 2);
    f.op3(Op::I64LtU, 3, 1, 0);
    f.br_if(3, lp);
    f.ret(1);
    let func = mb.add_func(ty, f);
    mb.export_func("count", func);
    mb.finish().expect("count module verifies")
}

/// fib(n) = n < 2 ? n : fib(n-1) + fib(n-2)
fn fib_module() -> efevm::Module {
    let mut mb = ModuleBuilder::new();
    let ty = FuncType::new([ValType::I64], [ValType::I64]);
    let fib = mb.declare_func(ty);
    let mut f = FunctionBuilder::new(6);
    let rec = f.new_label();
    f.const_i64(1, 2);
    f.op3(Op::I64GeU, 2, 0, 1);
    f.br_if(2, rec);
    f.ret(0);
    f.bind(rec);
    f.const_i64(1, 1);
    f.op3(Op::I64Sub, 3, 0, 1);
    f.call(fib, 3);
    f.const_i64(1, 2);
    f.op3(Op::I64Sub, 4, 0, 1);
    f.call(fib, 4);
    f.op3(Op::I64Add, 3, 3, 4);
    f.ret(3);
    mb.define(fib, f);
    mb.export_func("fib", fib);
    mb.finish().expect("fib module verifies")
}

fn instance(m: &efevm::Module, jit: bool) -> Instance<()> {
    let mut inst = Instance::new(m, &Imports::<()>::new(), Limits::default()).expect("links");
    if jit {
        enable(&mut inst, &JitOptions::default()).expect("jit compiles");
    }
    inst
}

/// Runs `entry(arg)` with the budget per resume; returns (units, seconds, result).
fn run(inst: &mut Instance<()>, entry: &str, arg: i64, budget: u64) -> (u64, f64, u64) {
    let f = inst.spawn(entry, &[arg as u64]).unwrap();
    let mut total = 0;
    let start = Instant::now();
    loop {
        let (exit, u) = inst.resume_units(f, &mut (), budget);
        total += u;
        match exit {
            Exit::OutOfBudget => continue,
            Exit::Done => break,
            e => panic!("unexpected {e:?}"),
        }
    }
    let t = start.elapsed().as_secs_f64();
    let r = inst.results(f).unwrap()[0];
    inst.remove_fiber(f);
    (total, t, r)
}

fn best(inst: &mut Instance<()>, entry: &str, arg: i64, budget: u64) -> (f64, u64) {
    let mut b = f64::MAX;
    let mut units = 0;
    for _ in 0..REPS {
        let (u, t, _) = run(inst, entry, arg, budget);
        units = u;
        b = b.min(t * 1e9 / u as f64);
    }
    (b, units)
}

fn main() {
    let n: i64 = std::env::args().nth(1).map_or(DEFAULT_ITERS, |a| a.parse().expect("iterations"));
    let m = count_module();
    let fm = fib_module();
    let mut interp = instance(&m, false);
    let mut jit = instance(&m, true);
    println!("counting loop: count({n}), 3 instructions per iteration (ns/instruction, best of {REPS})");
    for (name, budget) in [("single resume, unlimited budget", u64::MAX), ("resume with budget 10000 until done", RESUME_BUDGET)] {
        let (i, ui) = best(&mut interp, "count", n, budget);
        let (j, uj) = best(&mut jit, "count", n, budget);
        assert_eq!(ui, uj, "units differ");
        println!("  {name}: interpreter {i:.3}, jit {j:.3}  (x{:.1})", i / j);
    }
    let fibn = 32;
    let mut interp = instance(&fm, false);
    let mut jit = instance(&fm, true);
    println!("recursive fib({fibn}), calls and returns");
    for (name, budget) in [("single resume, unlimited budget", u64::MAX), ("resume with budget 10000 until done", RESUME_BUDGET)] {
        let (i, ui) = best(&mut interp, "fib", fibn, budget);
        let (j, uj) = best(&mut jit, "fib", fibn, budget);
        assert_eq!(ui, uj, "units differ");
        println!("  {name}: interpreter {i:.3}, jit {j:.3}  (x{:.1})", i / j);
    }
}
