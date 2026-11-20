//! Differential fuzzing of the translator against `wasmi` (interpreter and JIT of efevm).
//!
//! * `fuzz_short` runs in a normal `cargo test`: `EFE_WASM_FUZZ_CASES` (default 150) cases from
//!   `EFE_WASM_FUZZ_SEED` (default 1).
//! * `fuzz_long` is `#[ignore]`d:
//!   `EFE_WASM_FUZZ_SECS=600 EFE_WASM_FUZZ_SEED=1 cargo test -p efe-wasm --release --test fuzz -- --ignored fuzz_long --nocapture`
//!   (also `EFE_WASM_FUZZ_ITERS` to stop after a count). It prints every failing seed and fails
//!   at the end; replay one with
//!   `EFE_WASM_FUZZ_SEED=<seed> EFE_WASM_FUZZ_CASES=1 cargo test -p efe-wasm --test fuzz fuzz_short`.
//!   The failing module is written to `$CARGO_TARGET_TMPDIR/efe_wasm_fuzz_<seed>.wat`.
mod common;
use common::fuzz::fuzz_case;
use std::collections::BTreeMap;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
fn fuzz_short() {
    let seed = env_u64("EFE_WASM_FUZZ_SEED", 1);
    let cases = env_u64("EFE_WASM_FUZZ_CASES", 150);
    let (mut calls, mut bytes) = (0, 0);
    let mut ends = BTreeMap::<String, u64>::new();
    for s in seed..seed + cases {
        eprintln!("seed {s}");
        match fuzz_case(s) {
            Ok(o) => {
                calls += o.calls;
                bytes += o.wasm_bytes;
                for (k, v) in o.ends {
                    *ends.entry(k).or_default() += v;
                }
            }
            Err(e) => panic!("{e}"),
        }
    }
    eprintln!("fuzz_short: {cases} cases from seed {seed}, {calls} calls compared (wasmi, interpreter, JIT), {bytes} bytes of wasm, ends {ends:?}");
}

#[test]
#[ignore]
fn fuzz_long() {
    let seed = env_u64("EFE_WASM_FUZZ_SEED", 1);
    let secs = env_u64("EFE_WASM_FUZZ_SECS", 60);
    let iters = env_u64("EFE_WASM_FUZZ_ITERS", u64::MAX);
    let start = std::time::Instant::now();
    let (mut n, mut calls, mut failures) = (0u64, 0u64, Vec::<(u64, String)>::new());
    let mut ends = BTreeMap::<String, u64>::new();
    while start.elapsed().as_secs() < secs && n < iters {
        let s = seed + n;
        n += 1;
        match std::panic::catch_unwind(|| fuzz_case(s)) {
            Ok(Ok(o)) => {
                calls += o.calls;
                for (k, v) in o.ends {
                    *ends.entry(k).or_default() += v;
                }
            }
            Ok(Err(e)) => {
                eprintln!("FAIL {e}");
                failures.push((s, e));
            }
            Err(p) => {
                let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                eprintln!("PANIC seed {s}: {msg}");
                failures.push((s, msg));
            }
        }
        if failures.len() >= 20 {
            break;
        }
        if n % 1000 == 0 {
            eprintln!("fuzz_long: {n} cases, {calls} calls, {:?}", start.elapsed());
        }
    }
    eprintln!("fuzz_long: {n} cases from seed {seed}, {calls} calls compared, {:?}, {} failures", start.elapsed(), failures.len());
    eprintln!("how the calls ended: {ends:#?}");
    assert!(failures.is_empty(), "failing seeds: {:?}", failures.iter().map(|f| f.0).collect::<Vec<_>>());
}
