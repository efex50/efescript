//! The efescript examples under both backends, with the real `std` host registry.
use std::cell::RefCell;
use std::rc::Rc;

use efescript::host_std::{StdEnv, StdOptions, std_registry};
use efescript::prelude::CaptureHost;
use efescript::run_v2;
use efevm::{Exit, Instance, Limits};
use efevm_jit::{JitOptions, enable};

type Cap = Rc<RefCell<CaptureHost>>;

struct Run {
    inst: Instance<StdEnv<Cap>>,
    env: StdEnv<Cap>,
    cap: Cap,
}

fn make(prog: &run_v2::Program, jit: bool, input: &[u8]) -> Run {
    let imports = std_registry::<Cap>(&StdOptions::default()).imports();
    let mut inst = Instance::new(&prog.module, &imports, Limits::default()).expect("link");
    if jit {
        enable(&mut inst, &JitOptions { verify: true, ..Default::default() }).expect("jit");
    }
    let cap = CaptureHost::shared();
    let env = StdEnv::with_input(cap.clone(), input.to_vec());
    Run { inst, env, cap }
}

fn drive(prog: &run_v2::Program, budgets: &[u64], input: &[u8]) {
    let mut a = make(prog, false, input);
    let mut b = make(prog, true, input);
    let fa = a.inst.spawn("main", &[]).unwrap();
    let fb = b.inst.spawn("main", &[]).unwrap();
    for k in 0..2_000_000usize {
        let bud = budgets[k % budgets.len()];
        let (ea, ua) = a.inst.resume_units(fa, &mut a.env, bud);
        let (eb, ub) = b.inst.resume_units(fb, &mut b.env, bud);
        assert_eq!((&ea, ua), (&eb, ub), "step {k}");
        assert_eq!(a.cap.borrow().out, b.cap.borrow().out, "output at step {k}");
        assert_eq!(a.cap.borrow().sounds, b.cap.borrow().sounds);
        assert_eq!(a.env.exit_code(), b.env.exit_code());
        assert_eq!(a.inst.fiber_location(fa), b.inst.fiber_location(fb), "step {k}");
        assert_eq!(a.inst.fiber_backtrace(fa), b.inst.fiber_backtrace(fb), "step {k}");
        assert_eq!(a.inst.fiber_registers(fa), b.inst.fiber_registers(fb), "step {k}");
        assert_eq!(a.inst.memory().as_slice(), b.inst.memory().as_slice(), "memory at step {k}");
        match ea {
            Exit::Done | Exit::Trap(_) => {
                assert_eq!(a.inst.results(fa), b.inst.results(fb));
                assert_eq!(a.inst.trap_location(fa), b.inst.trap_location(fb));
                assert_eq!(a.inst.backtrace(fa), b.inst.backtrace(fb));
                return;
            }
            _ => {}
        }
    }
    panic!("runaway");
}

#[test]
fn examples_match() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../efescript/examples");
    let mut n = 0;
    let mut files: Vec<_> = std::fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for path in files {
        if path.extension().and_then(|e| e.to_str()) != Some("efa") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let prog = run_v2::load_path(path.to_str().unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        for budgets in [vec![u64::MAX], vec![1], vec![3], vec![17], vec![5, 1, 100]] {
            drive(&prog, &budgets, b"hello world\nsecond line\n");
        }
        n += 1;
    }
    assert!(n >= 8, "found {n} examples");
}
