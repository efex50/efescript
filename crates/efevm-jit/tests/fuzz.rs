//! Differential fuzzing: random verified modules (`common/randmod.rs`) on the interpreter, the JIT
//! and an instance that switches backends at random resumes, with random budgets.
//!
//! * `fuzz_short` runs in a normal `cargo test` (`EFEVM_FUZZ_CASES`, default 150, from
//!   `EFEVM_FUZZ_SEED`, default 1).
//! * `fuzz_long` is `#[ignore]`d:
//!   `EFEVM_FUZZ_SECS=600 EFEVM_FUZZ_SEED=1 cargo test -p efevm-jit --release --test fuzz -- --ignored fuzz_long --nocapture`
//!   (also `EFEVM_FUZZ_ITERS` to stop after a count). It prints every failing seed and fails at
//!   the end; replay one with `EFEVM_FUZZ_SEED=<seed> EFEVM_FUZZ_CASES=1 cargo test -p efevm-jit --test fuzz fuzz_short`.
mod common;
use common::fuzz_case;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn fuzz_short() {
    let seed = env_u64("EFEVM_FUZZ_SEED", 1);
    let cases = env_u64("EFEVM_FUZZ_CASES", 150);
    let mut steps = 0;
    for s in seed..seed + cases {
        match fuzz_case(s) {
            Ok(o) => steps += o.steps,
            Err(e) => panic!("{e}"),
        }
    }
    eprintln!("fuzz_short: {cases} cases, {steps} resumes compared");
}

#[test]
#[ignore]
fn fuzz_long() {
    let seed = env_u64("EFEVM_FUZZ_SEED", 1);
    let secs = env_u64("EFEVM_FUZZ_SECS", 60);
    let iters = env_u64("EFEVM_FUZZ_ITERS", u64::MAX);
    let start = std::time::Instant::now();
    let (mut n, mut steps, mut units, mut failures) =
        (0u64, 0usize, 0u64, Vec::<(u64, String)>::new());
    let mut ends = std::collections::BTreeMap::<String, u64>::new();
    while start.elapsed().as_secs() < secs && n < iters {
        let s = seed + n;
        n += 1;
        let r = std::panic::catch_unwind(|| fuzz_case(s));
        match r {
            Ok(Ok(o)) => {
                steps += o.steps;
                units += o.units;
                *ends
                    .entry(
                        o.end.split('(').next().unwrap_or("").to_string()
                            + &o.end
                                .split("Trap")
                                .nth(1)
                                .map(|t| t.chars().take(24).collect::<String>())
                                .unwrap_or_default(),
                    )
                    .or_insert(0u64) += 1;
            }
            Ok(Err(e)) => {
                eprintln!("FAIL seed {s}: {e}");
                failures.push((s, e));
            }
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                eprintln!("PANIC seed {s}: {msg}");
                failures.push((s, msg));
            }
        }
        if failures.len() >= 20 {
            break;
        }
        if n % 2000 == 0 {
            eprintln!(
                "fuzz_long: {n} cases, {steps} resumes, {:?}",
                start.elapsed()
            );
        }
    }
    eprintln!(
        "fuzz_long: {n} cases from seed {seed}, {steps} resumes and {units} units compared, {:?}, {} failures",
        start.elapsed(),
        failures.len()
    );
    eprintln!("how the programs ended: {ends:#?}");
    assert!(
        failures.is_empty(),
        "failing seeds: {:?}",
        failures.iter().map(|f| f.0).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn fuzz_stats() {
    let mut ends = std::collections::BTreeMap::<String, u64>::new();
    let (mut units, mut steps) = (0u64, 0usize);
    for s in 1..=300 {
        let o = fuzz_case(s).unwrap_or_else(|e| panic!("{e}"));
        units += o.units;
        steps += o.steps;
        *ends.entry(o.end).or_default() += 1;
    }
    eprintln!("300 cases: {units} units, {steps} resumes, ends {ends:#?}");
}
