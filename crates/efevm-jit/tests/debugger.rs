//! The efescript debugger always runs on the interpreter: a JIT installed on its instance is
//! removed before the first step and no native code runs.
use efescript::debugger::Debugger;
use efevm::ValType::I64;
use efevm::*;
use efevm_jit::{JitBackend, JitOptions, compile, native_runs};

#[test]
fn the_debugger_forces_the_interpreter() {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(4);
    f.const_i64(1, 0);
    f.const_i64(2, 1);
    let lp = f.new_label();
    f.bind(lp);
    f.op3(Op::I64Add, 1, 1, 2);
    f.const_i64(3, 5);
    f.op3(Op::I64LtU, 3, 1, 3);
    f.br_if(3, lp);
    f.ret(1);
    let func = mb.add_func(FuncType::new(vec![], vec![I64]), f);
    mb.export_func("main", func);
    let m = mb.finish().unwrap();

    let imports = HostRegistry::<()>::new().imports();
    let mut dbg = Debugger::<()>::new(m.clone(), &imports, "main", None).unwrap();
    assert!(
        !dbg.instance().has_backend(),
        "a fresh debugger has no backend"
    );
    let code = compile(&m, &JitOptions::default()).unwrap();
    dbg.instance_mut()
        .set_backend(Some(Box::new(JitBackend::<()>::new(code))));
    assert!(dbg.instance().has_backend());
    let before = native_runs();
    let mut out = Vec::new();
    let mut host = ();
    dbg.command("step", &mut host, &mut out).unwrap();
    assert!(!dbg.instance().has_backend(), "stepping drops the backend");
    dbg.command("continue", &mut host, &mut out).unwrap();
    assert!(dbg.is_ended());
    assert_eq!(native_runs(), before, "no native run");
}
