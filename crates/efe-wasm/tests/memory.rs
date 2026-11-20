//! Loads and stores of every width, out of bounds, memory.size/grow/copy/fill, data segments,
//! the lazy start function.
mod common;
use common::*;
use efevm::Trap;

const M: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x80];

fn le(k: usize, n: usize) -> u64 {
    (0..n).fold(0u64, |v, i| v | (M[k + i] as u64) << (8 * i))
}

fn sext(v: u64, bits: u32) -> i64 {
    ((v << (64 - bits)) as i64) >> (64 - bits)
}

#[test]
fn loads_every_width() {
    let mut wat = String::from("(module (memory 1 3) (data (i32.const 8) \"");
    for b in M {
        wat.push_str(&format!("\\{b:02x}"));
    }
    wat.push_str("\")\n");
    let loads = [
        ("i32.load", "i32"), ("i64.load", "i64"), ("f32.load", "f32"), ("f64.load", "f64"),
        ("i32.load8_s", "i32"), ("i32.load8_u", "i32"), ("i32.load16_s", "i32"), ("i32.load16_u", "i32"),
        ("i64.load8_s", "i64"), ("i64.load8_u", "i64"), ("i64.load16_s", "i64"), ("i64.load16_u", "i64"),
        ("i64.load32_s", "i64"), ("i64.load32_u", "i64"),
    ];
    for (name, ty) in loads {
        wat.push_str(&format!("(func (export \"{name}\") (param i32) (result {ty}) local.get 0 {name} offset=2)\n"));
    }
    wat.push(')');
    let mut p = Pair::new(&tr_wat(&wat));
    for k in 0..=8usize {
        let base = (6 + k) as u64; // + offset 2 = 8 + k
        let z32 = |v: i64| v as i32 as u32 as u64;
        assert_eq!(p.call1("i32.load", &[base]), le(k, 4));
        assert_eq!(p.call1("i64.load", &[base]), le(k, 8));
        assert_eq!(p.call1("f32.load", &[base]), le(k, 4));
        assert_eq!(p.call1("f64.load", &[base]), le(k, 8));
        assert_eq!(p.call1("i32.load8_s", &[base]), z32(sext(le(k, 1), 8)));
        assert_eq!(p.call1("i32.load8_u", &[base]), le(k, 1));
        assert_eq!(p.call1("i32.load16_s", &[base]), z32(sext(le(k, 2), 16)));
        assert_eq!(p.call1("i32.load16_u", &[base]), le(k, 2));
        assert_eq!(p.call1("i64.load8_s", &[base]), sext(le(k, 1), 8) as u64);
        assert_eq!(p.call1("i64.load8_u", &[base]), le(k, 1));
        assert_eq!(p.call1("i64.load16_s", &[base]), sext(le(k, 2), 16) as u64);
        assert_eq!(p.call1("i64.load16_u", &[base]), le(k, 2));
        assert_eq!(p.call1("i64.load32_s", &[base]), sext(le(k, 4), 32) as u64);
        assert_eq!(p.call1("i64.load32_u", &[base]), le(k, 4));
    }
    // The last valid address and one past it; base + offset does not wrap at 32 bits.
    let last = 65536 - 4 - 2;
    assert_eq!(p.call("i32.load", &[last]).map(|_| ()), Ok(()));
    assert_eq!(p.call("i32.load", &[last + 1]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("i32.load", &[0xffff_fffe]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("i32.load", &[0xffff_ffff]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("i64.load8_u", &[65536 - 2 - 1]).map(|_| ()), Ok(()));
    assert_eq!(p.call("i64.load8_u", &[65536 - 2]), Err(Trap::OutOfBounds));
}

#[test]
fn stores_every_width() {
    let stores = [
        ("i32.store", "i32", 4), ("i64.store", "i64", 8), ("f32.store", "f32", 4), ("f64.store", "f64", 8),
        ("i32.store8", "i32", 1), ("i32.store16", "i32", 2), ("i64.store8", "i64", 1), ("i64.store16", "i64", 2), ("i64.store32", "i64", 4),
    ];
    let mut wat = String::from("(module (memory 1)\n");
    for (name, ty, _) in stores {
        wat.push_str(&format!("(func (export \"{name}\") (param i32 {ty}) local.get 0 local.get 1 {name} offset=3)\n"));
    }
    wat.push(')');
    let m = tr_wat(&wat);
    let mut p = Pair::new(&m);
    let v: u64 = 0x1122_3344_5566_7788;
    for (name, _, width) in stores {
        let arg = if name.starts_with("f32") { v & 0xffff_ffff } else { v };
        let arg = if name.starts_with("i32") || name.starts_with("f32") { arg & 0xffff_ffff } else { arg };
        let before = p.a.memory().as_slice().to_vec();
        p.call(name, &[100, arg]).unwrap();
        let after = p.a.memory().as_slice().to_vec();
        assert_eq!(p.b.memory().as_slice(), &after[..], "{name}: JIT and interpreter memory");
        for (i, (x, y)) in before.iter().zip(&after).enumerate() {
            let want = if (103..103 + width).contains(&i) { (arg >> (8 * (i - 103))) as u8 } else { *x };
            assert_eq!(*y, want, "{name}: byte {i}");
        }
    }
    // Out of bounds: nothing is written, the trap is OutOfBounds.
    let snapshot = p.a.memory().as_slice().to_vec();
    assert_eq!(p.call("i64.store", &[65536 - 3 - 7, 1]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("i32.store8", &[65536 - 3, 1]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("i32.store", &[0xffff_ffff, 1]), Err(Trap::OutOfBounds));
    assert_eq!(p.a.memory().as_slice(), &snapshot[..]);
    assert!(p.call("i32.store8", &[65536 - 4, 1]).is_ok());
}

#[test]
fn memory_size_grow_fill_copy() {
    let mut p = Pair::new(&tr_wat(
        r#"(module (memory 1 3)
      (func (export "size") (result i32) memory.size)
      (func (export "grow") (param i32) (result i32) local.get 0 memory.grow)
      (func (export "fill") (param i32 i32 i32) local.get 0 local.get 1 local.get 2 memory.fill)
      (func (export "copy") (param i32 i32 i32) local.get 0 local.get 1 local.get 2 memory.copy)
      (func (export "get8") (param i32) (result i32) local.get 0 i32.load8_u)
      (func (export "put8") (param i32 i32) local.get 0 local.get 1 i32.store8))
    "#,
    ));
    assert_eq!(p.call1("size", &[]), 1);
    assert_eq!(p.call1("grow", &[1]), 1);
    assert_eq!(p.call1("size", &[]), 2);
    // Failure: -1 as an i32 (zero-extended in the register), memory unchanged.
    assert_eq!(p.call1("grow", &[2]), 0xffff_ffff);
    assert_eq!(p.call1("grow", &[0xffff_ffff]), 0xffff_ffff);
    assert_eq!(p.call1("size", &[]), 2);
    assert_eq!(p.call1("grow", &[0]), 2);
    assert_eq!(p.call1("grow", &[1]), 2);
    assert_eq!(p.call1("grow", &[1]), 0xffff_ffff);
    // The grown memory is usable and zero.
    assert_eq!(p.call1("get8", &[3 * 65536 - 1]), 0);
    assert!(p.call("put8", &[3 * 65536 - 1, 9]).is_ok());
    assert_eq!(p.call("put8", &[3 * 65536, 9]), Err(Trap::OutOfBounds));

    p.call("fill", &[10, 0x1ab, 5]).unwrap();
    for i in 9..16 {
        assert_eq!(p.call1("get8", &[i]), if (10..15).contains(&i) { 0xab } else { 0 }, "byte {i}");
    }
    // Overlapping copies behave like memmove, both directions.
    for i in 0..8u64 {
        p.call("put8", &[100 + i, i + 1]).unwrap();
    }
    p.call("copy", &[102, 100, 6]).unwrap();
    let got: Vec<u64> = (100..108).map(|i| p.call1("get8", &[i])).collect();
    assert_eq!(got, [1, 2, 1, 2, 3, 4, 5, 6]);
    p.call("copy", &[100, 102, 6]).unwrap();
    let got: Vec<u64> = (100..108).map(|i| p.call1("get8", &[i])).collect();
    assert_eq!(got, [1, 2, 3, 4, 5, 6, 5, 6]);
    // Bounds: the whole range must fit; zero length at the very end is fine.
    let end = 3 * 65536;
    assert_eq!(p.call("fill", &[end - 2, 1, 3]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("fill", &[end, 1, 0]).map(|_| ()), Ok(()));
    assert_eq!(p.call("copy", &[0, end - 1, 2]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("copy", &[end - 1, 0, 2]), Err(Trap::OutOfBounds));
    assert_eq!(p.call("copy", &[0, 0, 0xffff_ffff]), Err(Trap::OutOfBounds));
    // The fill value is a byte.
    assert_eq!(p.call("fill", &[0, 0xffff_ff00 | 0x42, 1]).map(|_| ()), Ok(()));
    assert_eq!(p.call1("get8", &[0]), 0x42);
}

#[test]
fn data_segments_and_limits() {
    let m = tr_wat(r#"(module (memory 2 5) (data (i32.const 0) "ab") (data (i32.const 65540) "cd") (data (i32.const 7) ""))"#);
    let d = m.data();
    assert_eq!((d.memory.min_pages, d.memory.max_pages), (2, Some(5)));
    assert_eq!(d.data.len(), 2, "empty segments are dropped");
    assert_eq!((d.data[1].offset, &d.data[1].bytes[..]), (65540, &b"cd"[..]));
    let inst = instance(&m, Backend::Interp);
    assert_eq!(&inst.memory().as_slice()[65540..65542], b"cd");
}

#[test]
fn start_function_runs_lazily_once() {
    let m = tr_wat(
        r#"(module (memory 1) (global $c (mut i32) (i32.const 0))
      (func $start global.get $c i32.const 1 i32.add global.set $c)
      (start $start)
      (func (export "count") (result i32) global.get $c)
      (func (export "twice") (result i32) global.get $c i32.const 10 i32.mul))"#,
    );
    // Not run at load: the first call of an export runs it, later calls do not.
    let mut a = instance(&m, Backend::Interp);
    assert_eq!(call(&mut a, "count", &[]), Ok(vec![1]));
    assert_eq!(call(&mut a, "count", &[]), Ok(vec![1]));
    assert_eq!(call(&mut a, "twice", &[]), Ok(vec![10]));
    // A fresh instance starts over.
    let mut b = instance(&m, Backend::Jit);
    assert_eq!(call(&mut b, "twice", &[]), Ok(vec![10]));
    assert_eq!(call(&mut b, "count", &[]), Ok(vec![1]));

    // __wasm_call_ctors is run the same way, and calling it yourself does not run it twice,
    // whichever comes first.
    let m = tr_wat(
        r#"(module (global $c (mut i32) (i32.const 0))
      (func (export "__wasm_call_ctors") global.get $c i32.const 5 i32.add global.set $c)
      (func (export "get") (result i32) global.get $c))"#,
    );
    for jit in [false, true] {
        let b = if jit { Backend::Jit } else { Backend::Interp };
        let mut a = instance(&m, b);
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![5]));
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![5]));
        assert_eq!(call(&mut a, "__wasm_call_ctors", &[]), Ok(vec![]), "already initialised: nothing to do");
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![5]));
        let mut a = instance(&m, b);
        assert_eq!(call(&mut a, "__wasm_call_ctors", &[]), Ok(vec![]));
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![5]), "ctors ran once");
        assert_eq!(call(&mut a, "__wasm_call_ctors", &[]), Ok(vec![]));
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![5]));
    }

    // Start function and ctors together, the start function exported too.
    let m = tr_wat(
        r#"(module (global $c (mut i32) (i32.const 0))
      (func $start (export "start") global.get $c i32.const 1 i32.add global.set $c)
      (start $start)
      (func (export "__wasm_call_ctors") global.get $c i32.const 10 i32.add global.set $c)
      (func (export "get") (result i32) global.get $c))"#,
    );
    for name in ["start", "__wasm_call_ctors", "get"] {
        let mut a = instance(&m, Backend::Interp);
        call(&mut a, name, &[]).unwrap();
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![11]), "first call: {name}");
        call(&mut a, name, &[]).unwrap();
        assert_eq!(call(&mut a, "get", &[]), Ok(vec![11]), "second call: {name}");
    }
}
