//! Shared harness: run a module under the interpreter and under the JIT and compare
//! everything observable after every resume.
#![allow(dead_code)]

use efevm::*;
use efevm_jit::{JitBackend, JitOptions, compile};
use std::sync::Arc;

pub mod prog;

/// Host state: a log of what host functions saw.
#[derive(Debug, Default, PartialEq, Clone)]
pub struct Hs {
    pub log: Vec<u64>,
}

pub fn ty(p: &[ValType], r: &[ValType]) -> FuncType {
    FuncType::new(p.to_vec(), r.to_vec())
}
pub fn ty64(np: usize, nr: usize) -> FuncType {
    FuncType::new(vec![ValType::I64; np], vec![ValType::I64; nr])
}

pub fn raw_desc(namespace: &str, name: &str, ty: &FuncType) -> HostFn {
    let mut d = HostFn::new(namespace, name).doc("Test function.");
    for (i, t) in ty.params.iter().enumerate() {
        d = d.param(format!("p{i}"), ApiType::from_val(*t));
    }
    for (i, t) in ty.results.iter().enumerate() {
        d = d.result(format!("r{i}"), ApiType::from_val(*t));
    }
    d
}

pub fn reg_host(
    reg: &mut HostRegistry<Hs>,
    name: &str,
    ty: FuncType,
    f: impl Fn(&mut Caller<'_, Hs>, &[u64], &mut [u64]) -> Result<HostOk, Trap> + Send + Sync + 'static,
) {
    if reg.namespace("env").is_none() {
        reg.define_namespace("env", 1, "test", "Test functions.").unwrap();
    }
    let d = raw_desc("env", name, &ty);
    let w = move |c: &mut Caller<'_, Hs>, a: &Args<'_>, r: &mut Results<'_>| f(c, a.raw(), r.raw());
    reg.register(d, w).unwrap();
}

#[derive(Clone)]
pub struct Cfg {
    pub limits: Limits,
    pub jit: JitOptions,
    /// Compare NaN results as equivalent.
    pub nan: bool,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { limits: Limits::default(), jit: JitOptions { verify: true, ..JitOptions::default() }, nan: false }
    }
}

pub fn norm(v: u64, nan: bool) -> u64 {
    if !nan {
        return v;
    }
    if f64::from_bits(v).is_nan() {
        return 0x7ff8_0000_0000_0000;
    }
    if v <= u32::MAX as u64 && f32::from_bits(v as u32).is_nan() {
        return 0x7fc0_0000;
    }
    v
}

fn normv(v: &[u64], nan: bool) -> Vec<u64> {
    v.iter().map(|x| norm(*x, nan)).collect()
}

#[derive(Debug, PartialEq)]
pub struct Snap {
    pub exit: Exit,
    pub units: u64,
    pub status: Option<FiberStatus>,
    pub results: Option<Vec<u64>>,
    pub trap_loc: Option<(u32, u32)>,
    pub backtrace: Option<Vec<(u32, u32)>>,
    pub f_loc: Option<(u32, u32)>,
    pub f_bt: Vec<(u32, u32)>,
    pub f_regs: Option<Vec<u64>>,
    pub memory: (usize, u64),
    pub globals: Vec<u64>,
    pub host: Hs,
}

pub fn mem_hash(b: &[u8]) -> (usize, u64) {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for c in b.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x100_0000_01b3);
        h ^= h >> 29;
    }
    (b.len(), h)
}

pub fn snap(inst: &Instance<Hs>, f: FiberId, exit: Exit, units: u64, host: &Hs, nan: bool) -> Snap {
    let ng = inst.module().data().globals.len() as u32;
    Snap {
        exit,
        units,
        status: inst.status(f),
        results: inst.results(f).map(|r| normv(r, nan)),
        trap_loc: inst.trap_location(f),
        backtrace: inst.backtrace(f).map(<[_]>::to_vec),
        f_loc: inst.fiber_location(f),
        f_bt: inst.fiber_backtrace(f),
        f_regs: inst.fiber_registers(f).map(|r| normv(r, nan)),
        memory: mem_hash(inst.memory().as_slice()),
        globals: (0..ng).map(|g| norm(inst.global(g).unwrap(), nan)).collect(),
        host: host.clone(),
    }
}

pub fn interp_inst(m: &Module, reg: &HostRegistry<Hs>, limits: Limits) -> Instance<Hs> {
    Instance::new(m, &reg.imports(), limits).expect("link")
}

pub fn jit_backend(m: &Module, cfg: &Cfg) -> JitBackend<Hs> {
    JitBackend::new(compile(m, &cfg.jit).expect("jit compile"))
}

pub fn jit_inst(m: &Module, reg: &HostRegistry<Hs>, cfg: &Cfg) -> Instance<Hs> {
    let mut i = interp_inst(m, reg, cfg.limits);
    i.set_backend(Some(Box::new(jit_backend(m, cfg))));
    i
}

/// Max resumes before a test gives up (a runaway program).
const MAX_STEPS: usize = 300_000;

/// An interpreter instance and a JIT instance of the same module.
pub struct Pair {
    pub a: Instance<Hs>,
    pub b: Instance<Hs>,
    pub ha: Hs,
    pub hb: Hs,
    pub cfg: Cfg,
}

impl Pair {
    pub fn new(m: &Module, reg: &HostRegistry<Hs>, cfg: &Cfg) -> Pair {
        Pair {
            a: interp_inst(m, reg, cfg.limits),
            b: jit_inst(m, reg, cfg),
            ha: Hs::default(),
            hb: Hs::default(),
            cfg: cfg.clone(),
        }
    }

    /// Spawns `entry(args)` on both and resumes both with the cycled budgets until the fiber
    /// ends, comparing after every resume. Removes the fibers; returns the final snapshot.
    pub fn run(&mut self, entry: &str, args: &[u64], budgets: &[u64]) -> Snap {
        let fa = self.a.spawn(entry, args).unwrap();
        let fb = self.b.spawn(entry, args).unwrap();
        let nan = self.cfg.nan;
        let mut last = None;
        for k in 0..MAX_STEPS {
            let budget = budgets[k % budgets.len()];
            let (ea, ua) = self.a.resume_units(fa, &mut self.ha, budget);
            let (eb, ub) = self.b.resume_units(fb, &mut self.hb, budget);
            let sa = snap(&self.a, fa, ea.clone(), ua, &self.ha, nan);
            let sb = snap(&self.b, fb, eb, ub, &self.hb, nan);
            assert_eq!(sa, sb, "step {k}, budget {budget}, entry {entry}({args:?})");
            let end = matches!(ea, Exit::Done | Exit::Trap(_));
            last = Some(sa);
            if end {
                self.a.remove_fiber(fa);
                self.b.remove_fiber(fb);
                return last.unwrap();
            }
        }
        panic!("runaway program: {:?}", last.map(|s| s.f_loc));
    }
}

pub fn diff_cfg(
    m: &Module,
    reg: &HostRegistry<Hs>,
    entry: &str,
    args: &[u64],
    budgets: &[u64],
    cfg: &Cfg,
) -> Snap {
    Pair::new(m, reg, cfg).run(entry, args, budgets)
}

pub fn diff(m: &Module, reg: &HostRegistry<Hs>, entry: &str, args: &[u64], budgets: &[u64]) -> Snap {
    diff_cfg(m, reg, entry, args, budgets, &Cfg::default())
}

/// The standard budget sequences.
pub fn budget_sets() -> Vec<Vec<u64>> {
    vec![
        vec![u64::MAX],
        vec![1],
        vec![2],
        vec![3],
        vec![7],
        vec![1000],
        vec![5, 1, 9, 2, 100],
        vec![0, 4, 0, 1, u64::MAX],
    ]
}

/// Like `diff_cfg` over every standard budget set.
pub fn diff_all(m: &Module, reg: &HostRegistry<Hs>, entry: &str, args: &[u64]) -> Snap {
    let mut last = None;
    for b in budget_sets() {
        last = Some(diff(m, reg, entry, args, &b));
    }
    last.unwrap()
}

/// Cross-backend resume: a reference interpreter run, and a second instance that switches
/// between the JIT and the interpreter at every resume following `pattern` (cycled; `true` =
/// JIT). Both use `budgets`; all observations must match the reference after every resume.
pub fn cross(code: &Arc<efevm_jit::JitCode>, m: &Module, reg: &HostRegistry<Hs>, entry: &str, args: &[u64], budgets: &[u64], pattern: &[bool], cfg: &Cfg) {
    let mut a = interp_inst(m, reg, cfg.limits);
    let mut b = interp_inst(m, reg, cfg.limits);
    let (mut ha, mut hb) = (Hs::default(), Hs::default());
    let fa = a.spawn(entry, args).unwrap();
    let fb = b.spawn(entry, args).unwrap();
    for k in 0..MAX_STEPS {
        let budget = budgets[k % budgets.len()];
        let jit = pattern[k % pattern.len()];
        b.set_backend(if jit { Some(Box::new(JitBackend::<Hs>::new(Arc::clone(&code)))) } else { None });
        let (ea, ua) = a.resume_units(fa, &mut ha, budget);
        let (eb, ub) = b.resume_units(fb, &mut hb, budget);
        let sa = snap(&a, fa, ea.clone(), ua, &ha, cfg.nan);
        let sb = snap(&b, fb, eb, ub, &hb, cfg.nan);
        assert_eq!(sa, sb, "cross step {k} (jit={jit}, budget {budget})");
        if matches!(ea, Exit::Done | Exit::Trap(_)) {
            return;
        }
    }
    panic!("runaway program");
}
