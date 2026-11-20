//! Everything outside the subset is rejected with an error naming the construct.
mod common;
use common::*;
use efe_wasm::{TranslateError, TranslateOptions, translate};

fn rejects(wat: &str, needle: &str) {
    let e = tr_err(wat);
    assert!(e.contains(needle), "error for\n{wat}\nshould contain `{needle}`, is: {e}");
}

#[test]
fn atomics_and_shared_memory() {
    rejects("(module (memory 1 1 shared))", "shared memory (atomics)");
    rejects("(module (memory 1) (func (param i32) (result i32) local.get 0 i32.atomic.load))", "atomics (threads)");
    rejects("(module (memory 1 1 shared) (func (param i32) (result i32) local.get 0 i32.const 1 i32.atomic.rmw.add))", "shared memory");
}

#[test]
fn browser_style_flags_are_explained() {
    let e = tr_err(r#"(module (import "env" "memory" (memory 1 1 shared)))"#);
    assert!(e.contains("imported memory `env::memory`") && e.contains("--import-memory") && e.contains("RUSTFLAGS"), "{e}");
}

#[test]
fn simd() {
    rejects("(module (func (result v128) v128.const i64x2 0 0))", "SIMD");
    rejects("(module (func v128.const i32x4 0 0 0 0 drop))", "SIMD");
    rejects("(module (func (param i32) (local v128)))", "SIMD");
}

#[test]
fn exceptions() {
    rejects("(module (tag (param i32)))", "exceptions");
    rejects(r#"(module (import "e" "t" (tag (param i32))))"#, "exceptions");
}

#[test]
fn gc_tail_calls_function_references() {
    rejects("(module (type (struct (field i32))))", "GC types");
    rejects("(module (type (array i8)))", "GC types");
    rejects("(module (func $a return_call $a))", "tail calls");
}

#[test]
fn memory64_multi_memory_extended_const() {
    rejects("(module (memory i64 1))", "memory64");
    rejects("(module (memory 1) (memory 1))", "multi-memory");
    rejects("(module (global i32 (i32.add (i32.const 1) (i32.const 2))))", "extended-const");
    rejects("(module (memory 1) (memory 1) (func (param i32) (result i32) local.get 0 i32.load 1))", "multi-memory");
}

#[test]
fn bulk_memory_init_and_passive_segments() {
    rejects(r#"(module (memory 1) (data "abc"))"#, "passive data segments");
    rejects(r#"(module (memory 1) (data $d "abc") (func i32.const 0 i32.const 0 i32.const 3 memory.init $d))"#, "passive data segments");
    rejects(r#"(module (memory 1) (data $d "abc") (func data.drop $d))"#, "passive data segments");
}

#[test]
fn reference_types_beyond_one_funcref_table() {
    rejects("(module (table 1 funcref) (func (param i32) (result funcref) local.get 0 table.get 0))", "reference types");
    rejects("(module (func ref.null func drop))", "reference types");
    rejects("(module (func $f) (elem declare func $f))", "element segments");
    rejects("(module (table 1 funcref) (table 1 funcref))", "more than one table");
    rejects("(module (table 1 externref))", "funcref table");
    rejects("(module (table 1 funcref) (func (result i32) table.size 0))", "reference types");
    rejects("(module (table 1 funcref) (func $f) (elem (i32.const 0) func $f) (func i32.const 0 i32.const 0 i32.const 0 table.init 0 0))", "reference types");
    rejects("(module (func $f) (global funcref (ref.func $f)))", "reference");
}

#[test]
fn imported_memory_table_globals() {
    rejects(r#"(module (import "e" "m" (memory 1)))"#, "imported memory");
    rejects(r#"(module (import "e" "t" (table 1 funcref)))"#, "imported table");
    rejects(r#"(module (import "e" "g" (global i32)))"#, "imported global");
    rejects(r#"(module (global i32 (i32.const 0)) (memory 1) (data (global.get 0) "a"))"#, "constant expression");
}

#[test]
fn errors_name_function_and_offset() {
    let e = tr_err("(module (func $first) (func $second (param i32) (local v128) v128.const i32x4 0 0 0 0 drop))");
    assert!(e.contains("SIMD"), "{e}");
    // The v128 local is a valid declaration; the operator is what is rejected, in function 1.
    let e = tr_err("(module (func $first) (func $second v128.const i32x4 0 0 0 0 drop))");
    assert!(e.contains("function 1 `second`") && e.contains("byte "), "{e}");
}

#[test]
fn invalid_wasm() {
    let r = translate(b"not wasm at all", &empty_index(), &TranslateOptions::default());
    assert!(matches!(r, Err(TranslateError::Invalid { .. })), "{r:?}");
    // A type error the validator catches.
    let bytes = wasm("(module (func (result i32) i32.const 1))");
    let mut bad = bytes.clone();
    let n = bad.len();
    bad[n - 3] = 0x42; // i32.const -> i64.const: the function now leaves an i64
    let r = translate(&bad, &empty_index(), &TranslateOptions::default());
    assert!(matches!(r, Err(TranslateError::Invalid { .. })), "{r:?}");
}

#[test]
fn function_that_needs_too_many_registers() {
    let mut wat = String::from("(module (func (export \"f\") (local");
    for _ in 0..40_000 {
        wat.push_str(" i32");
    }
    wat.push(')');
    for _ in 0..30_000 {
        wat.push_str(" i32.const 1");
    }
    for _ in 0..30_000 {
        wat.push_str(" drop");
    }
    wat.push_str("))");
    let e = tr_err(&wat);
    assert!(e.contains("function too large") && e.contains("65535"), "{e}");
    // The operand stack can overflow the limit too.
    let mut wat = String::from("(module (func (export \"f\") (result i32)");
    for _ in 0..66_000 {
        wat.push_str(" i32.const 1");
    }
    for _ in 0..65_999 {
        wat.push_str(" i32.add");
    }
    wat.push_str("))");
    let e = tr_err(&wat);
    assert!(e.contains("function too large"), "{e}");
}

#[test]
fn imports_must_follow_the_abi() {
    // Not `<namespace>@<version>`.
    let e = tr_err(r#"(module (import "env" "log" (func)))"#);
    assert!(e.contains("env::log") && e.contains("<namespace>@<version>"), "{e}");
    // Namespace not in the index.
    let e = tr_err(r#"(module (import "game@1" "log" (func)))"#);
    assert!(e.contains("namespace `game` is not in the API index"), "{e}");
}
