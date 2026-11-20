mod common;
use common::*;

#[test]
fn add_and_loop() {
    let wat = r#"(module
      (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)
      (func (export "sum") (param i32) (result i32) (local i32 i32)
        (block (loop
          local.get 1 local.get 0 i32.ge_u br_if 1
          local.get 2 local.get 1 i32.add local.set 2
          local.get 1 i32.const 1 i32.add local.set 1
          br 0))
        local.get 2))"#;
    assert_eq!(ok1(run_wat(wat, "add", &[2, 3])), 5);
    assert_eq!(ok1(run_wat(wat, "sum", &[10])), 45);
}
