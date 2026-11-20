//! Findings of the review of efe-wasm: components, GC type groups, block types, return windows,
//! multi-value moves, data segments, memory limits, malformed input.
mod common;
use common::prov::*;
use common::*;
use efe_wasm::{TranslateOptions, translate};
use efevm::Trap;

fn rejects(wat: &str, needle: &str) {
    let e = tr_err(wat);
    assert!(e.contains(needle), "error for\n{wat}\nshould contain `{needle}`, is: {e}");
}

#[test]
fn components_are_rejected() {
    let r = translate(b"\0asm\x0d\x00\x01\x00", &empty_index(), &TranslateOptions::default());
    assert!(r.unwrap_err().to_string().contains("components"));
    let c = wat::parse_str("(component (core module $a (type (func (param i32))) (func (type 0))) (core module $b (func (call 0))))").unwrap();
    let r = translate(&c, &empty_index(), &TranslateOptions::default());
    assert!(r.unwrap_err().to_string().contains("components"));
}

#[test]
fn gc_groups_subtypes_and_block_types() {
    rejects("(module (rec (type $a (func)) (type $b (func))) (func (type $a)))", "GC type groups");
    rejects("(module (rec (type $a (func))))", "GC type groups");
    rejects("(module (type $a (sub (func))) (func (type $a)))", "GC subtyping");
    rejects("(module (func (block (result v128) unreachable) drop))", "block of type `v128`");
    rejects("(module (func (block (result externref) unreachable) drop))", "externref");
    rejects("(module (func unreachable select (result v128)))", "`select` of type");
    rejects("(module (func (param i32) (loop (result v128) unreachable) drop))", "block of type");
    rejects("(module (func (param i32) local.get 0 (if (result v128) (then unreachable) (else unreachable)) drop))", "block of type");
}

#[test]
fn multi_value_results_after_dead_blocks_verify() {
    let mut p = Pair::new(&tr_wat(
        r#"(module
      (func (export "a") (result i32 i32) (block (result i32 i32) unreachable))
      (func (export "b") (result i32 i32 i32) (block (result i32 i32 i32) unreachable) )
      (func (export "c") (result i32 i32) (block (result i32 i32) unreachable) return)
      (func (export "d") (result i32 i32) (block (result i32 i32) unreachable) br 0)
      (func (export "e") (param i32) (result i32 i32) (block (result i32 i32) unreachable) local.get 0 br_if 0 return)
      (func (export "f") (param i32) (result i32 i32) (block (result i32 i32) unreachable) local.get 0 br_table 0 0)
      (func (export "g") (param i32) (result i32 i32) (local.get 0) (if (result i32 i32) (then unreachable) (else unreachable))))"#,
    ));
    for f in ["a", "b", "c", "d"] {
        assert_eq!(p.call(f, &[]), Err(Trap::Unreachable), "{f}");
    }
    for f in ["e", "f", "g"] {
        assert_eq!(p.call(f, &[1]), Err(Trap::Unreachable), "{f}");
    }
}

#[test]
fn multi_value_moves_with_src_not_dst() {
    let mut p = Pair::new(&tr_wat(
        r#"(module
      (func (export "mv") (param i32) (result i32 i32)
        i32.const 9
        (block (result i32 i32)
          i32.const 100 i32.const 1 i32.const 2
          local.get 0 br_if 0
          drop drop drop i32.const 7 i32.const 8)
        ;; [9, a, b] -> [a, b+9]: keep the results
        local.set 0 i32.add local.get 0 i32.const 0 i32.add)
      (func (export "mv3") (param i32) (result i32 i32 i32) (local i32 i32 i32)
        i32.const 11 i32.const 12
        (block (result i32 i32 i32)
          i32.const 100 i32.const 1 i32.const 2 i32.const 3
          br 0)
        local.set 1 local.set 2 local.set 3 drop drop
        local.get 3 local.get 2 local.get 1)
      (func (export "mv3b") (param i32) (result i32 i32 i32)
        (block (result i32 i32 i32)
          i32.const 100 i32.const 101 i32.const 1 i32.const 2 i32.const 3
          local.get 0 br_if 0
          drop drop drop drop drop i32.const 4 i32.const 5 i32.const 6))
      (func (export "lp") (param i32) (result i32) (local i32 i32)
        i32.const 0 local.get 0
        (loop $l (param i32 i32) (result i32)
          local.set 2 local.set 1
          local.get 2 i32.eqz
          if (result i32)
            local.get 1
          else
            i32.const 77
            local.get 1 local.get 2 i32.add
            local.get 2 i32.const 1 i32.sub
            br $l
          end))
      (func (export "lp_if") (param i32) (result i32) (local i32 i32)
        i32.const 0 local.get 0
        (loop $l (param i32 i32) (result i32)
          local.set 2 local.set 1
          i32.const 55
          local.get 1 local.get 2 i32.add
          local.get 2 i32.const 1 i32.sub
          local.get 2 br_if $l
          drop drop drop
          local.get 1)
        )
      (func (export "tbl") (param i32) (result i32 i32)
        i32.const 5 i32.const 1 i32.const 2 local.get 0 br_table 0 0)
      (func (export "tbl2") (param i32) (result i32 i32)
        (block (result i32 i32)
          i32.const 50
          (block (result i32 i32)
            i32.const 60 i32.const 1 i32.const 2 local.get 0 br_table 0 1 1)
          ;; the inner block ended normally: [50, 1, 2]
          i32.add))
    )"#,
    ));
    assert_eq!(p.call("mv", &[1]), Ok(vec![10, 2]));
    assert_eq!(p.call("mv", &[0]), Ok(vec![16, 8]));
    assert_eq!(p.call("mv3", &[0]), Ok(vec![1, 2, 3]));
    assert_eq!(p.call("mv3b", &[1]), Ok(vec![1, 2, 3]));
    assert_eq!(p.call("mv3b", &[0]), Ok(vec![4, 5, 6]));
    assert_eq!(p.call1("lp", &[4]), 10);
    assert_eq!(p.call1("lp", &[0]), 0);
    assert_eq!(p.call1("lp_if", &[4]), 10);
    for x in [0, 1, 7] {
        assert_eq!(p.call("tbl", &[x]), Ok(vec![1, 2]));
    }
    assert_eq!(p.call("tbl2", &[0]), Ok(vec![50, 3]));
    assert_eq!(p.call("tbl2", &[1]), Ok(vec![1, 2]));
    assert_eq!(p.call("tbl2", &[5]), Ok(vec![1, 2]));
}

#[test]
fn if_with_params_and_no_else() {
    let mut p = Pair::new(&tr_wat(
        r#"(module (func (export "f") (param i32 i32) (result i32)
      local.get 1 local.get 0
      if (param i32) (result i32) i32.const 1 i32.add end))"#,
    ));
    assert_eq!(p.call1("f", &[1, 5]), 6);
    assert_eq!(p.call1("f", &[0, 5]), 5);
}

#[test]
fn data_segments_and_memory_limits() {
    rejects(r#"(module (memory 1) (data (i32.const 999999) ""))"#, "does not fit");
    rejects(r#"(module (memory 1) (data (i32.const 65535) "ab"))"#, "does not fit");
    // wasm32 memories are capped at 65536 pages, even when the module says nothing.
    let m = tr_wat("(module (memory 1))");
    assert_eq!((m.data().memory.min_pages, m.data().memory.max_pages), (1, Some(65536)));
    let m = tr_wat("(module (memory 0 65536))");
    assert_eq!(m.data().memory.max_pages, Some(65536));
    let mut p = Pair::new(&tr_wat(r#"(module (memory 0) (func (export "grow") (param i32) (result i32) local.get 0 memory.grow) (func (export "size") (result i32) memory.size))"#));
    assert_eq!(p.call1("size", &[]), 0);
    assert_eq!(p.call1("grow", &[2]), 0);
    assert_eq!(p.call1("grow", &[0xffff_ffff]), 0xffff_ffff);
    assert_eq!(p.call1("grow", &[0x8000_0000]), 0xffff_ffff);
    assert_eq!(p.call1("size", &[]), 2);
}

#[test]
fn import_versions_are_plain_digits() {
    let e = tr_err(r#"(module (import "game@+1" "log" (func)))"#);
    assert!(e.contains("<namespace>@<version>"), "{e}");
}

/// A tiny xorshift, so the mutations do not depend on the generator module.
fn xs(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn malformed_input_never_panics() {
    let ix = fixture_index();
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    for w in [
        r#"(module (memory 1) (table 4 funcref) (func $a (param i32) (result i32) local.get 0 i32.const 1 i32.add)
            (elem (i32.const 0) $a) (data (i32.const 0) "hello")
            (func (export "f") (param i32) (result i32) (block (result i32) local.get 0 local.get 0 br_if 0 drop i32.const 2)
              i32.const 0 call_indirect (param i32) (result i32)))"#,
        r#"(module (import "game@1" "scale" (func (param f64) (result f64))) (import "game@1" "score_changed.send" (func (param i32)))
            (global (export "efe_scratch") i32 (i32.const 64)) (memory 1) (func (export "efe_provide_x") (param f64) (result f64) local.get 0 call 0))"#,
    ] {
        let mut b = wasm(w);
        if w.contains("efe_scratch") {
            let mut recs = scratch(64);
            recs.extend(func("x", "efe_provide_x", "X.", &[("a", F64, None)], &[("result", F64, None)]));
            b = with_custom(b, "efe.provide", &recs);
            b = with_custom(b, "efe.api", &ix.hash().to_le_bytes());
        }
        seeds.push(b);
    }
    let iterations: u64 = std::env::var("EFE_WASM_MALFORMED_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
    let mut st = 0x1234_5678_9abc_def1u64;
    for k in 0..iterations {
        let mut b = seeds[(k % seeds.len() as u64) as usize].clone();
        match xs(&mut st) % 4 {
            0 => {
                for _ in 0..1 + xs(&mut st) % 4 {
                    let i = (xs(&mut st) % b.len() as u64) as usize;
                    b[i] ^= 1 << (xs(&mut st) % 8);
                }
            }
            1 => b.truncate((xs(&mut st) % b.len() as u64) as usize),
            2 => {
                let i = (xs(&mut st) % b.len() as u64) as usize;
                b[i] = xs(&mut st) as u8;
                let j = (xs(&mut st) % b.len() as u64) as usize;
                b.remove(j);
            }
            _ => {
                // random bytes in a custom section payload
                let mut junk = Vec::new();
                for _ in 0..xs(&mut st) % 48 {
                    junk.push(xs(&mut st) as u8);
                }
                let name = if xs(&mut st) % 2 == 0 { "efe.provide" } else { "efe.api" };
                b = with_custom(b, name, &junk);
            }
        }
        // Whatever comes out, the translator returns: Ok(verified module) or an error.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| translate(&b, &ix, &TranslateOptions::default())));
        assert!(r.is_ok(), "translate panicked on mutation {k}: {b:02x?}");
    }
}
