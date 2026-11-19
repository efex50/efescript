mod common;
use common::*;
use efevm::ValType::I64;
use efevm::*;

#[test]
fn counting_loop() {
    let mut mb = ModuleBuilder::new();
    let mut f = FunctionBuilder::new(4);
    f.const_i64(1, 0);
    f.const_i64(2, 1);
    let lp = f.new_label();
    f.bind(lp);
    f.op3(Op::I64Add, 1, 1, 2);
    f.op3(Op::I64LtU, 3, 1, 0);
    f.br_if(3, lp);
    f.ret(1);
    let func = mb.add_func(ty(&[I64], &[I64]), f);
    mb.export_func("count", func);
    let m = mb.finish().unwrap();
    let s = diff_all(&m, &HostRegistry::new(), "count", &[1000]);
    assert_eq!(s.results, Some(vec![1000]));
}
