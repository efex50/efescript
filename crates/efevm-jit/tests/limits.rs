//! Call depth, register-stack and native-stack limits.
mod common;
use common::*;
use efevm::*;

/// deep(n): n == 0 -> yield, return 0; else 1 + deep(n-1) (with a few extra registers).
fn deep_module(nregs: u16, with_yield: bool) -> Module {
    let mut mb = ModuleBuilder::new();
    let deep = mb.declare_func(ty64(1, 1));
    let mut f = FunctionBuilder::new(nregs);
    let base = f.new_label();
    f.br_ifz(0, base);
    f.const_i64(1, 1);
    f.op3(Op::I64Sub, 2, 0, 1);
    f.call(deep, 2);
    f.op3(Op::I64Add, 2, 2, 1);
    f.ret(2);
    f.bind(base);
    if with_yield {
        f.yield_();
    }
    f.const_i64(2, 0);
    f.ret(2);
    mb.define(deep, f);
    mb.export_func("deep", deep);
    mb.finish().unwrap()
}

#[test]
fn call_depth_limit_same_location() {
    let m = deep_module(6, true);
    for depth_limit in [1u32, 2, 3, 10, 50] {
        let cfg = Cfg { limits: Limits { max_call_depth: depth_limit, ..Limits::default() }, ..Cfg::default() };
        for n in [0u64, 1, 2, 8, 9, 10, 49, 50, 60] {
            for b in budget_sets() {
                let s = diff_cfg(&m, &HostRegistry::new(), "deep", &[n], &b, &cfg);
                if n + 1 > depth_limit as u64 {
                    assert_eq!(s.exit, Exit::Trap(Trap::StackOverflow));
                    // innermost trap location is the call instruction of the deepest allowed frame
                    assert_eq!(s.backtrace.as_ref().unwrap().len(), depth_limit as usize);
                }
            }
        }
    }
}

#[test]
fn register_limit_same_location() {
    for nregs in [3u16, 6, 20] {
        let m = deep_module(nregs, true);
        for max_regs in [nregs as u32, nregs as u32 * 2, nregs as u32 * 5 + 1, 1000] {
            let cfg = Cfg { limits: Limits { max_registers: max_regs, ..Limits::default() }, ..Cfg::default() };
            for n in [0u64, 1, 4, 5, 6, 100] {
                for b in [vec![u64::MAX], vec![3], vec![1, 2, 50]] {
                    let s = diff_cfg(&m, &HostRegistry::new(), "deep", &[n], &b, &cfg);
                    let need = (n + 1) * nregs as u64;
                    if need > max_regs as u64 {
                        assert_eq!(s.exit, Exit::Trap(Trap::StackOverflow));
                    }
                }
            }
        }
    }
}

#[test]
fn deep_recursion_past_native_stack() {
    let m = deep_module(8, true);
    let before = efevm_jit::native_stack_fallbacks();
    for stack in [0usize, 2 << 10, 8 << 10, 64 << 10] {
        let cfg = Cfg {
            jit: efevm_jit::JitOptions { max_native_stack: stack, verify: true, ..Default::default() },
            limits: Limits { max_call_depth: 5000, max_registers: 1_000_000, ..Limits::default() },
            ..Cfg::default()
        };
        let mut p = Pair::new(&m, &HostRegistry::new(), &cfg);
        for n in [0u64, 5, 100, 1500, 3000] {
            for b in [vec![u64::MAX], vec![2000], vec![7, 1, 5000, 3], vec![1]] {
                if n >= 1500 && b == vec![1] {
                    continue;
                }
                let s = p.run("deep", &[n], &b);
                assert_eq!(s.results, Some(vec![n]));
            }
        }
        // past the interpreter limit too
        let s = p.run("deep", &[6000], &[u64::MAX]);
        assert_eq!(s.exit, Exit::Trap(Trap::StackOverflow));
    }
    assert!(efevm_jit::native_stack_fallbacks() > before);
    assert!(efevm_jit::native_stack_resume_fallbacks() > 0, "the re-entry fallback must be exercised");
}

#[test]
fn native_stack_fallback_with_calls_and_hosts() {
    // the full test program, tiny native stack: every suspension re-enters, falls back, etc.
    let m = prog::module();
    let reg = prog::registry();
    for stack in [0usize, 1 << 10, 4 << 10] {
        let cfg = Cfg { jit: efevm_jit::JitOptions { max_native_stack: stack, verify: true, ..Default::default() }, ..Cfg::default() };
        let mut p = Pair::new(&m, &reg, &cfg);
        for n in [0u64, 3, 8, 13] {
            for sel in [0u64, 1, 3] {
                for b in [vec![u64::MAX], vec![1], vec![17], vec![3, 40, 2]] {
                    p.run("main", &[n, sel], &b);
                }
            }
        }
    }
}

/// Found by running the efevm binary tests on the JIT: a function with 65 535 registers made
/// Cranelift overflow its block-parameter count and panic. Such modules are now refused
/// (`TooManyRegisters`) and stay on the interpreter.
#[test]
fn a_function_with_too_many_registers_is_refused() {
    use efevm::*;
    use efevm_jit::{JitError, JitOptions, MAX_JIT_REGISTERS, compile};
    for nregs in [u16::MAX, MAX_JIT_REGISTERS as u16 + 1] {
        let mut mb = ModuleBuilder::new();
        let mut f = FunctionBuilder::new(nregs);
        f.const_i64(1, 7);
        f.ret(1);
        let id = mb.add_func(FuncType::new(vec![], vec![ValType::I64]), f);
        mb.export_func("main", id);
        let m = mb.finish().unwrap();
        match compile(&m, &JitOptions::default()) {
            Err(JitError::TooManyRegisters { func: 0, nregs: n }) => assert_eq!(n, nregs as u32),
            other => panic!("expected TooManyRegisters, got {:?}", other.map(|_| ())),
        }
        // the interpreter runs it
        let mut inst = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
        assert!(efevm_jit::enable(&mut inst, &JitOptions::default()).is_err());
        assert!(!inst.has_backend());
        let f = inst.spawn("main", &[]).unwrap();
        assert_eq!(inst.resume(f, &mut (), u64::MAX), Exit::Done);
        assert_eq!(inst.results(f), Some(&[7][..]));
    }
}
