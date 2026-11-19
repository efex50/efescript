//! Counting-loop benchmark (SPEC.md section 10): reports ns per instruction.
//!
//! Usage: `cargo run -p efevm --release --example bench [iterations]`

use std::time::Instant;

use efevm::{Exit, FuncType, FunctionBuilder, Imports, Instance, Limits, ModuleBuilder, Op, ValType};

const REPS: usize = 5;
const DEFAULT_ITERS: i64 = 100_000_000;
const RESUME_BUDGET: u64 = 10_000;
const TARGET_NS: f64 = 10.0;

/// `count(n: i64) -> i64`: counts r1 up to n, returns it.
///
/// r0 = n (param), r1 = counter, r2 = 1, r3 = compare flag.
/// Each iteration executes 3 instructions: i64.add, i64.lt_u, br_if.
fn build_module() -> efevm::Module {
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

struct Sample {
    instructions: u64,
    ms: f64,
    ns_per_instr: f64,
}

fn sample(instructions: u64, elapsed: std::time::Duration) -> Sample {
    let ms = elapsed.as_secs_f64() * 1e3;
    let ns_per_instr = elapsed.as_secs_f64() * 1e9 / instructions as f64;
    Sample { instructions, ms, ns_per_instr }
}

fn report(name: &str, samples: &[Sample]) {
    println!("{name}");
    for (i, s) in samples.iter().enumerate() {
        println!(
            "  run {}: {} instructions, {:.3} ms, {:.3} ns/instr",
            i + 1,
            s.instructions,
            s.ms,
            s.ns_per_instr
        );
    }
    let mut ns: Vec<f64> = samples.iter().map(|s| s.ns_per_instr).collect();
    ns.sort_by(|a, b| a.partial_cmp(b).expect("finite timings"));
    let best = ns[0];
    let median = ns[ns.len() / 2];
    println!("  best:   {best:.3} ns/instr");
    println!("  median: {median:.3} ns/instr");
    println!(
        "  target < {TARGET_NS} ns: {}",
        if best < TARGET_NS { "PASS" } else { "FAIL" }
    );
}

/// Single resume with an unlimited budget.
fn bench_full(inst: &mut Instance<()>, n: i64) -> Sample {
    let fiber = inst.spawn("count", &[n as u64]).expect("spawn count");
    let start = Instant::now();
    let (exit, executed) = inst.resume_counted(fiber, &mut (), u64::MAX);
    let elapsed = start.elapsed();
    check_done(inst, fiber, exit, n);
    inst.remove_fiber(fiber);
    sample(executed, elapsed)
}

/// Repeated resumes with a small budget until the fiber is Done.
fn bench_budgeted(inst: &mut Instance<()>, n: i64) -> Sample {
    let fiber = inst.spawn("count", &[n as u64]).expect("spawn count");
    let mut total = 0u64;
    let start = Instant::now();
    let exit = loop {
        let (exit, executed) = inst.resume_counted(fiber, &mut (), RESUME_BUDGET);
        total += executed;
        match exit {
            Exit::OutOfBudget => continue,
            other => break other,
        }
    };
    let elapsed = start.elapsed();
    check_done(inst, fiber, exit, n);
    inst.remove_fiber(fiber);
    sample(total, elapsed)
}

fn check_done(inst: &Instance<()>, fiber: efevm::FiberId, exit: Exit, n: i64) {
    assert!(matches!(exit, Exit::Done), "expected Exit::Done, got {exit:?}");
    let results = inst.results(fiber).expect("done fiber has results");
    assert_eq!(results, &[n as u64], "count({n}) result");
}

fn main() {
    let n: i64 = match std::env::args().nth(1) {
        Some(arg) => arg.parse().expect("iteration count must be an integer"),
        None => DEFAULT_ITERS,
    };

    let module = build_module();
    let mut inst = Instance::new(&module, &Imports::<()>::new(), Limits::default())
        .expect("instance links");

    println!("counting loop: count({n}), 3 instructions per iteration");
    println!();

    // Warm-up run so caches and the allocator are primed before timing.
    let _ = bench_full(&mut inst, n.min(1_000_000));

    let full: Vec<Sample> = (0..REPS).map(|_| bench_full(&mut inst, n)).collect();
    report("benchmark 1: single resume, unlimited budget", &full);
    println!();

    let budgeted: Vec<Sample> = (0..REPS).map(|_| bench_budgeted(&mut inst, n)).collect();
    report(
        &format!("benchmark 2: resume with budget {RESUME_BUDGET} until done"),
        &budgeted,
    );
}
