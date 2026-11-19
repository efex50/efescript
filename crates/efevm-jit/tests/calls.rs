//! Calls, host calls, yields, suspension and cross-backend resume.
mod common;
use common::*;
use efevm::*;

fn args_set() -> Vec<[u64; 2]> {
    let mut v = vec![];
    for n in [0u64, 1, 3, 6, 13, 14] {
        // sel: 0 = rec, 1 = ping, 3 = helper, others: errors / mismatches
        for sel in [0u64, 1, 3, 2, 4, 5, 100] {
            v.push([n, sel]);
        }
    }
    v
}

#[test]
fn program_diff_all_budgets() {
    let m = prog::module();
    let reg = prog::registry();
    let mut ended = 0;
    let before = efevm_jit::native_runs();
    for b in budget_sets() {
        let mut p = Pair::new(&m, &reg, &Cfg::default());
        for a in args_set() {
            let s = p.run("main", &a, &b);
            ended += 1;
            let _ = s;
        }
    }
    assert!(ended > 100);
    assert!(efevm_jit::native_runs() > before);
}

#[test]
fn trap_kinds_seen() {
    // make sure the program exercises the interesting traps
    let m = prog::module();
    let reg = prog::registry();
    let mut p = Pair::new(&m, &reg, &Cfg::default());
    let mut kinds = std::collections::HashSet::new();
    for a in args_set() {
        if let Exit::Trap(t) = p.run("main", &a, &[u64::MAX]).exit {
            kinds.insert(format!("{t:?}"));
        }
    }
    for k in ["UndefinedElement", "IndirectCallTypeMismatch", "Host(\"bad\")"] {
        assert!(kinds.contains(k), "missing {k}: {kinds:?}");
    }
}

#[test]
fn cross_backend_every_suspension_point() {
    let m = prog::module();
    let reg = prog::registry();
    let cfg = Cfg::default();
    let patterns: Vec<Vec<bool>> = vec![
        vec![true],
        vec![false],
        vec![true, false],
        vec![false, true],
        vec![true, true, false],
        vec![false, false, true],
        vec![true, false, false, true, false],
    ];
    let budgets: Vec<Vec<u64>> = vec![vec![1], vec![2], vec![3], vec![7], vec![1, 100, 2], vec![u64::MAX]];
    let code = efevm_jit::compile(&m, &cfg.jit).unwrap();
    for n in [0u64, 1, 4, 7, 13] {
        for sel in [0u64, 1, 2] {
            for b in &budgets {
                for pt in &patterns {
                    cross(&code, &m, &reg, "main", &[n, sel], b, pt, &cfg);
                }
            }
        }
    }
}

#[test]
fn step_then_resume_in_jit() {
    // A fiber stepped to mid-block is resumed by the interpreter, later blocks by the JIT.
    let m = prog::module();
    let reg = prog::registry();
    let cfg = Cfg::default();
    let mut a = interp_inst(&m, &reg, cfg.limits);
    let mut b = jit_inst(&m, &reg, &cfg);
    let (mut ha, mut hb) = (Hs::default(), Hs::default());
    let fa = a.spawn("main", &[6, 0]).unwrap();
    let fb = b.spawn("main", &[6, 0]).unwrap();
    for k in 0..20_000usize {
        let (ea, eb);
        if k % 5 == 0 {
            ea = a.step(fa, &mut ha);
            eb = b.step(fb, &mut hb);
            assert_eq!(ea, eb);
        } else {
            let bud = [1u64, 3, 11][k % 3];
            let (x, ux) = a.resume_units(fa, &mut ha, bud);
            let (y, uy) = b.resume_units(fb, &mut hb, bud);
            assert_eq!((ux, &x), (uy, &y), "k={k}");
            ea = x;
            eb = y;
        }
        let sa = snap(&a, fa, ea.clone(), 0, &ha, false);
        let sb = snap(&b, fb, eb, 0, &hb, false);
        assert_eq!(sa, sb, "k={k}");
        if matches!(ea, Exit::Done | Exit::Trap(_)) {
            return;
        }
    }
    panic!("runaway");
}
