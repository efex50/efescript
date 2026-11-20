//! Control flow, locals, globals, select, calls, multi-value, call_indirect.
mod common;
use common::*;
use efevm::Trap;

fn module(wat: &str) -> Pair {
    Pair::new(&tr_wat(wat))
}

#[test]
fn block_loop_if_else() {
    let mut p = module(
        r#"(module
      (func (export "abs") (param i32) (result i32)
        local.get 0 i32.const 0 i32.lt_s
        if (result i32) i32.const 0 local.get 0 i32.sub else local.get 0 end)
      (func (export "if_noelse") (param i32) (result i32) (local i32)
        local.get 0 if i32.const 7 local.set 1 end local.get 1)
      (func (export "fact") (param i64) (result i64) (local i64)
        i64.const 1 local.set 1
        (block (loop
          local.get 0 i64.eqz br_if 1
          local.get 1 local.get 0 i64.mul local.set 1
          local.get 0 i64.const 1 i64.sub local.set 0
          br 0))
        local.get 1)
      (func (export "nested") (param i32) (result i32)
        (block (result i32)
          (block (result i32)
            (block (result i32)
              i32.const 100
              local.get 0 br_table 0 1 2 0)
            i32.const 1 i32.add)
          i32.const 10 i32.add))
      (func (export "br_value") (param i32) (result i32)
        (block (result i32)
          i32.const 5
          local.get 0 br_if 0
          drop
          i32.const 9))
    )"#,
    );
    assert_eq!(p.call1("abs", &[5]), 5);
    assert_eq!(p.call1("abs", &[(-5i32) as u32 as u64]), 5);
    assert_eq!(p.call1("if_noelse", &[0]), 0);
    assert_eq!(p.call1("if_noelse", &[1]), 7);
    assert_eq!(p.call1("fact", &[10]), 3628800);
    assert_eq!(p.call1("nested", &[0]), 111);
    assert_eq!(p.call1("nested", &[1]), 110);
    assert_eq!(p.call1("nested", &[2]), 100);
    assert_eq!(p.call1("nested", &[3]), 111);
    assert_eq!(p.call1("nested", &[99]), 111);
    assert_eq!(p.call1("br_value", &[1]), 5);
    assert_eq!(p.call1("br_value", &[0]), 9);
}

#[test]
fn br_table_with_values_and_moves() {
    // Targets at different depths carry a value: it moves down to the target block's slot.
    let mut p = module(
        r#"(module
      (func (export "f") (param i32) (result i32)
        (block $outer (result i32)
          i32.const 1000
          (block $mid (result i32)
            i32.const 200
            (block $inner (result i32)
              i32.const 30
              local.get 0
              br_table $inner $mid $outer $mid)
            i32.add)
          i32.add)
        i32.const 1 i32.add)
      (func (export "to_fn") (param i32) (result i32)
        i32.const 77
        local.get 0
        br_table 0 0 0)
      (func (export "no_values") (param i32) (result i32)
        (block (block (block local.get 0 br_table 0 1 2))
          i32.const 1 return)
        i32.const 2)
    )"#,
    );
    assert_eq!(p.call1("f", &[0]), 1231);
    assert_eq!(p.call1("f", &[1]), 1031);
    assert_eq!(p.call1("f", &[2]), 31);
    assert_eq!(p.call1("f", &[3]), 1031);
    assert_eq!(p.call1("f", &[0xffff_ffff]), 1031);
    assert_eq!(p.call1("to_fn", &[0]), 77);
    assert_eq!(p.call1("to_fn", &[5]), 77);
    // idx 0 leaves the innermost block (then both others fall through to `i32.const 1 return`),
    // idx 1 leaves the middle one, idx 2 (and the default) the outer one.
    assert_eq!(p.call1("no_values", &[0]), 1);
    assert_eq!(p.call1("no_values", &[1]), 1);
    assert_eq!(p.call1("no_values", &[2]), 2);
    assert_eq!(p.call1("no_values", &[9]), 2);
}

#[test]
fn return_and_unreachable() {
    let mut p = module(
        r#"(module
      (func (export "early") (param i32) (result i32)
        local.get 0 if i32.const 1 return end i32.const 2)
      (func (export "trap") unreachable)
      (func (export "dead_code") (result i32)
        i32.const 3 return i32.const 4 i32.add)
      (func (export "after_br") (result i32)
        (block (result i32) i32.const 8 br 0 i32.const 1 i32.add))
      (func (export "dead_blocks") (result i32)
        i32.const 6 return
        (block (loop i32.const 0 (if (then unreachable) (else br 1))))
        i32.const 0)
    )"#,
    );
    assert_eq!(p.call1("early", &[1]), 1);
    assert_eq!(p.call1("early", &[0]), 2);
    assert_eq!(p.call("trap", &[]), Err(Trap::Unreachable));
    assert_eq!(p.call1("dead_code", &[]), 3);
    assert_eq!(p.call1("after_br", &[]), 8);
    assert_eq!(p.call1("dead_blocks", &[]), 6);
}

#[test]
fn select_drop_locals() {
    let mut p = module(
        r#"(module
      (func (export "sel") (param i32 i32 i32) (result i32) local.get 0 local.get 1 local.get 2 select)
      (func (export "sel64") (param i64 i64 i32) (result i64) local.get 0 local.get 1 local.get 2 select (result i64))
      (func (export "self") (param f32 f32 i32) (result f32) local.get 0 local.get 1 local.get 2 select)
      (func (export "tee") (param i32) (result i32) (local i32)
        local.get 0 local.tee 1 local.get 1 i32.add)
      (func (export "swap") (param i32 i32) (result i32 i32)
        local.get 1 local.get 0)
      (func (export "set_after_get") (param i32) (result i32)
        ;; the symbolic stack entry for local 0 must be materialised before local 0 changes
        local.get 0 i32.const 100 local.set 0 local.get 0 i32.add)
      (func (export "tee_hazard") (param i32) (result i32)
        local.get 0 local.get 0 i32.const 5 local.tee 0 drop i32.add)
      (func (export "dropper") (param i32) (result i32) local.get 0 i32.const 9 drop)
    )"#,
    );
    assert_eq!(p.call1("sel", &[10, 20, 1]), 10);
    assert_eq!(p.call1("sel", &[10, 20, 0]), 20);
    assert_eq!(p.call1("sel64", &[1 << 40, 2, 1]), 1 << 40);
    assert_eq!(p.call1("sel64", &[1 << 40, 2, 0]), 2);
    assert_eq!(p.call1("self", &[1.5f32.to_bits() as u64, 2.5f32.to_bits() as u64, 0]), 2.5f32.to_bits() as u64);
    assert_eq!(p.call1("tee", &[21]), 42);
    assert_eq!(p.call("swap", &[1, 2]), Ok(vec![2, 1]));
    assert_eq!(p.call1("set_after_get", &[7]), 107);
    assert_eq!(p.call1("tee_hazard", &[3]), 6);
    assert_eq!(p.call1("dropper", &[4]), 4);
}

#[test]
fn multi_value_blocks_and_functions() {
    let mut p = module(
        r#"(module
      (type $t (func (param i32 i32) (result i32 i32)))
      (func $pair (param i32) (result i32 i64)
        local.get 0 local.get 0 i64.extend_i32_u i64.const 1 i64.add)
      (func (export "use_pair") (param i32) (result i64) (local i64)
        local.get 0 call $pair
        local.set 1 i64.extend_i32_u local.get 1 i64.add)
      (func (export "block_params") (param i32 i32) (result i32)
        local.get 0 local.get 1
        (block (type $t) i32.add i32.const 3)
        i32.sub)
      (func (export "loop_params") (param i32) (result i32) (local i32)
        i32.const 0 local.get 0
        (loop $l (param i32 i32) (result i32)
          ;; acc n -> if n==0 return acc, else acc+n n-1
          local.set 0 local.set 1
          local.get 0 i32.eqz
          if (result i32) local.get 1
          else
            local.get 1 local.get 0 i32.add local.get 0 i32.const 1 i32.sub
            br $l
          end))
      (func (export "if_params") (param i32 i32) (result i32)
        local.get 1 i32.const 10
        local.get 0
        if (param i32 i32) (result i32) i32.add else i32.sub end)
      (func (export "br_multi") (param i32) (result i32 i32)
        (block (result i32 i32)
          i32.const 1 i32.const 2 local.get 0 br_if 0
          drop drop i32.const 8 i32.const 9))
    )"#,
    );
    assert_eq!(p.call1("use_pair", &[5]), 11);
    assert_eq!(p.call1("block_params", &[10, 20]), 30 - 3);
    assert_eq!(p.call1("loop_params", &[4]), 10);
    assert_eq!(p.call1("if_params", &[1, 5]), 15);
    assert_eq!(p.call1("if_params", &[0, 5]), 5u64.wrapping_sub(10) & 0xffff_ffff);
    assert_eq!(p.call("br_multi", &[1]), Ok(vec![1, 2]));
    assert_eq!(p.call("br_multi", &[0]), Ok(vec![8, 9]));
}

#[test]
fn globals() {
    let mut p = module(
        r#"(module
      (global $g (mut i32) (i32.const 5))
      (global $h (mut i64) (i64.const -1))
      (global $f (mut f32) (f32.const 1.5))
      (global $d f64 (f64.const 2.5))
      (func (export "bump") (result i32) global.get $g i32.const 1 i32.add global.set $g global.get $g)
      (func (export "h") (result i64) global.get $h)
      (func (export "f") (result f32) global.get $f)
      (func (export "d") (result f64) global.get $d)
      (export "g" (global $g)))
    "#,
    );
    assert_eq!(p.call1("bump", &[]), 6);
    assert_eq!(p.call1("bump", &[]), 7);
    assert_eq!(p.call1("h", &[]), u64::MAX);
    assert_eq!(p.call1("f", &[]), 1.5f32.to_bits() as u64);
    assert_eq!(p.call1("d", &[]), 2.5f64.to_bits());
}

#[test]
fn calls_and_recursion() {
    let mut p = module(
        r#"(module
      (func $fib (export "fib") (param i32) (result i32)
        local.get 0 i32.const 2 i32.lt_u
        if (result i32) local.get 0
        else
          local.get 0 i32.const 1 i32.sub call $fib
          local.get 0 i32.const 2 i32.sub call $fib
          i32.add
        end)
      (func $add3 (param i32 i32 i32) (result i32) local.get 0 local.get 1 i32.add local.get 2 i32.add)
      (func (export "args_in_middle") (param i32) (result i32)
        i32.const 1000
        local.get 0 i32.const 2 i32.const 3 call $add3
        i32.add)
      (func $inf (export "inf") call $inf))
    "#,
    );
    assert_eq!(p.call1("fib", &[15]), 610);
    assert_eq!(p.call1("args_in_middle", &[1]), 1006);
    assert_eq!(p.call("inf", &[]), Err(Trap::StackOverflow));
}

#[test]
fn call_indirect() {
    let mut p = module(
        r#"(module
      (type $bin (func (param i32 i32) (result i32)))
      (type $un (func (param i32) (result i32)))
      (table 6 funcref)
      (elem (i32.const 1) $add $sub $neg)
      (func $add (type $bin) local.get 0 local.get 1 i32.add)
      (func $sub (type $bin) local.get 0 local.get 1 i32.sub)
      (func $neg (type $un) i32.const 0 local.get 0 i32.sub)
      (func (export "bin") (param i32 i32 i32) (result i32)
        i32.const 1000 drop
        local.get 1 local.get 2 local.get 0 call_indirect (type $bin))
      (func (export "un") (param i32 i32) (result i32)
        local.get 1 local.get 0 call_indirect (type $un)))
    "#,
    );
    assert_eq!(p.call1("bin", &[1, 7, 3]), 10);
    assert_eq!(p.call1("bin", &[2, 7, 3]), 4);
    assert_eq!(p.call1("un", &[3, 9]), (-9i32) as u32 as u64);
    // Null entry (0 and 4), out of range (6, huge), wrong type.
    assert_eq!(p.call("bin", &[0, 1, 2]), Err(Trap::UndefinedElement));
    assert_eq!(p.call("bin", &[4, 1, 2]), Err(Trap::UndefinedElement));
    assert_eq!(p.call("bin", &[6, 1, 2]), Err(Trap::UndefinedElement));
    assert_eq!(p.call("bin", &[0xffff_ffff, 1, 2]), Err(Trap::UndefinedElement));
    assert_eq!(p.call("bin", &[3, 1, 2]), Err(Trap::IndirectCallTypeMismatch));
    assert_eq!(p.call("un", &[1, 2]), Err(Trap::IndirectCallTypeMismatch));
}

#[test]
fn names_are_copied() {
    let m = tr_wat(r#"(module (func $answer (export "x") (result i32) i32.const 42) (func $other (result i32) i32.const 1))"#);
    let names = &m.data().names.funcs;
    assert!(names.iter().any(|(_, n)| n == "answer"), "{names:?}");
    assert!(names.iter().any(|(_, n)| n == "other"), "{names:?}");
}
