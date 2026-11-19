//! Host registry, API index, section 12 `api`, linking with namespace versions, guest memory
//! helpers and debugger hooks (crates/efescript/docs/e3-design.md sections 2, 3, 6, 8).

use efevm::ValType::{I32, I64};
use efevm::*;

#[derive(Default)]
struct Out {
    lines: Vec<String>,
}

fn std_registry() -> HostRegistry<Out> {
    let mut reg = HostRegistry::<Out>::new();
    reg.set_game("test");
    reg.define_namespace("std", 1, "std", "Console output.").unwrap();
    reg.register(
        HostFn::new("std", "println").doc("Write `text` and a newline.").param("text", ApiType::Str),
        |caller, args, _res| {
            let text = caller.read_str(args.str(0)?)?;
            caller.host().lines.push(text);
            Ok(HostOk::Continue)
        },
    )
    .unwrap();
    reg.register(
        HostFn::new("std", "add")
            .doc("Add two numbers.")
            .param("a", ApiType::I32)
            .param("b", ApiType::I32)
            .result("sum", ApiType::I32),
        |_caller, args, res| {
            res.set_i32(0, args.i32(0)?.wrapping_add(args.i32(1)?))?;
            Ok(HostOk::Continue)
        },
    )
    .unwrap();
    reg.register(HostFn::new("std", "misuse").doc("Reads a parameter that does not exist."), |_c, args, _r| {
        args.i64(3)?;
        Ok(HostOk::Continue)
    })
    .unwrap();
    reg
}

/// `main() -> i32`: println("hello"), then add(40, 2). With `api`, records section 12.
fn hello_module(api: Option<(u64, Vec<(&str, u32)>)>, misuse: bool) -> Module {
    let mut mb = ModuleBuilder::new();
    let println = mb.import("std", "println", FuncType::new([I64, I64], []));
    let add = mb.import("std", "add", FuncType::new([I32, I32], [I32]));
    let bad = mb.import("std", "misuse", FuncType::new([], []));
    mb.memory(1, None);
    mb.data(16, b"hello");
    if let Some((hash, ns)) = api {
        mb.api_ref(hash, ns);
    }
    let mut fb = FunctionBuilder::new(4);
    fb.const_i64(0, 16);
    fb.const_i64(1, 5);
    fb.call(println, 0);
    if misuse {
        fb.call(bad, 0);
    }
    fb.const_i32(0, 40);
    fb.const_i32(1, 2);
    fb.call(add, 0);
    fb.ret(0);
    let main = mb.add_func(FuncType::new([], [I32]), fb);
    mb.export_func("main", main);
    mb.finish().unwrap()
}

#[test]
fn registry_functions_are_callable() {
    let reg = std_registry();
    let mut inst = Instance::new(&hello_module(None, false), &reg.imports(), Limits::default()).unwrap();
    let mut out = Out::default();
    assert_eq!(inst.call("main", &[], &mut out, 1000), Ok(vec![42]));
    assert_eq!(out.lines, ["hello"]);
    // A host function that asks for a wrong parameter traps cleanly.
    let mut inst = Instance::new(&hello_module(None, true), &reg.imports(), Limits::default()).unwrap();
    match inst.call("main", &[], &mut out, 1000) {
        Err(CallError::Trap { trap: Trap::Host(m), .. }) => assert!(m.contains("std.misuse"), "{m}"),
        e => panic!("{e:?}"),
    }
}

#[test]
fn imports_are_exactly_the_index() {
    let reg = std_registry();
    let imports = reg.imports();
    let ix = reg.index();
    let names: Vec<String> = imports.names().map(|(ns, n, _)| format!("{ns}.{n}")).collect();
    assert_eq!(names, ix.qualified_names());
    for (ns, n, ty) in imports.names() {
        let f = ix.function(ns, n).unwrap();
        assert_eq!(&f.func_type(), ty);
        assert!(!f.doc.is_empty());
        assert_eq!(f.owner, "std");
    }
    assert_eq!(imports.namespace_version("std"), Some(1));
    assert_eq!(imports.namespaces().collect::<Vec<_>>(), [("std", 1)]);
    assert_eq!(imports.api_hash(), Some(ix.hash()));
    assert_eq!(Imports::<Out>::new().api_hash(), None);
    let json = ix.to_json();
    let back = ApiIndex::from_json(&json).unwrap();
    assert_eq!(back, ix);
    assert_eq!(back.hash(), ix.hash());
    assert!(json.contains("\"game\": \"test\""));
    // A snapshot does not see later registrations.
    let mut reg = reg;
    reg.define_namespace("extra", 1, "x", "More.").unwrap();
    reg.register(HostFn::new("extra", "f").doc("F."), |_c, _a, _r| Ok(HostOk::Continue)).unwrap();
    assert_eq!(imports.names().count(), 3);
    assert_eq!(reg.imports().names().count(), 4);
    assert_ne!(reg.imports().api_hash(), imports.api_hash());
    assert_eq!(reg.unregister_owner("x"), Ok(2));
    assert_eq!(reg.imports().api_hash(), imports.api_hash());
}

#[test]
fn link_checks_namespace_versions() {
    let reg = std_registry();
    let hash = reg.index().hash();
    // Same versions: links, even with a different hash.
    for h in [hash, hash ^ 1] {
        let m = hello_module(Some((h, vec![("std", 1)])), false);
        assert!(reg.imports().check_module(&m).is_empty());
        assert!(Instance::new(&m, &reg.imports(), Limits::default()).is_ok());
    }
    // Module built against std v1, host has std v2.
    let mut v2 = std_registry();
    v2.define_namespace("std", 2, "std", "Console output, v2.").unwrap();
    let m = hello_module(Some((hash, vec![("std", 1)])), false);
    let want = LinkError::NamespaceVersion { namespace: "std".into(), module: 1, host: 2 };
    assert_eq!(Instance::new(&m, &v2.imports(), Limits::default()).err(), Some(LoadError::Link(want.clone())));
    assert_eq!(want.to_string(), "namespace std is version 2, module was built against version 1");
    let errs = v2.imports().check_module(&m);
    assert_eq!(errs, vec![want, LinkError::ApiHash { module: hash, host: v2.imports().api_hash() }]);
    // Unknown namespace.
    let m = hello_module(Some((hash, vec![("std", 1), ("engine.world", 1)])), false);
    assert_eq!(
        Instance::new(&m, &reg.imports(), Limits::default()).err(),
        Some(LoadError::Link(LinkError::UnknownNamespace { namespace: "engine.world".into() }))
    );
    // Without section 12 versions are not checked.
    let m = hello_module(None, false);
    assert!(Instance::new(&m, &v2.imports(), Limits::default()).is_ok());
}

#[test]
fn check_module_lists_every_problem() {
    let mut reg = HostRegistry::<Out>::new();
    reg.define_namespace("std", 3, "std", "Other std.").unwrap();
    reg.register(HostFn::new("std", "add").doc("Wrong type.").param("a", ApiType::I64).result("s", ApiType::I64), |_c, _a, _r| {
        Ok(HostOk::Continue)
    })
    .unwrap();
    let m = hello_module(Some((7, vec![("std", 1), ("gone", 1)])), false);
    let errs = reg.imports().check_module(&m);
    assert_eq!(
        errs,
        vec![
            LinkError::NamespaceVersion { namespace: "std".into(), module: 1, host: 3 },
            LinkError::UnknownNamespace { namespace: "gone".into() },
            LinkError::MissingImport { namespace: "std".into(), name: "println".into() },
            LinkError::ImportTypeMismatch {
                namespace: "std".into(),
                name: "add".into(),
                module: "(i32, i32) -> i32".into(),
                host: "(i64) -> i64".into(),
            },
            LinkError::MissingImport { namespace: "std".into(), name: "misuse".into() },
            LinkError::ApiHash { module: 7, host: reg.imports().api_hash() },
        ]
    );
    assert_eq!(
        errs[3].to_string(),
        "signature mismatch for std.add: module imports (i32, i32) -> i32, host has (i64) -> i64"
    );
    assert!(errs[5].to_string().contains("fnv1a64:0000000000000007"));
    // `Instance::new` reports the first one.
    assert_eq!(Instance::new(&m, &reg.imports(), Limits::default()).err(), Some(LoadError::Link(errs[0].clone())));
}

#[test]
fn api_section_round_trips() {
    let m = hello_module(Some((0x0123_4567_89ab_cdef, vec![("std", 1), ("engine.world", 70000)])), false);
    let bytes = m.encode();
    let back = Module::decode(&bytes).unwrap();
    assert_eq!(back.data(), m.data());
    let api = back.data().api.as_ref().unwrap();
    assert_eq!(api.hash, 0x0123_4567_89ab_cdef);
    assert_eq!(api.namespaces, vec![("std".to_string(), 1), ("engine.world".to_string(), 70000)]);
    // Some with no namespaces is kept distinct from None.
    let empty = ModuleData { api: Some(ApiRef::default()), ..ModuleData::default() };
    assert_eq!(binary::decode(&binary::encode(&empty)).unwrap(), empty);
    assert_eq!(binary::encode(&empty), b"EFVM\x01\x00\x0c\x09\0\0\0\0\0\0\0\0\0");
    assert_eq!(binary::encode(&ModuleData::default()), b"EFVM\x01\x00");
    // Every truncation of the api section fails to decode instead of panicking.
    let full = binary::encode(&m.data().clone());
    let start = full.iter().rposition(|&b| b == 12).unwrap();
    for n in start..full.len() {
        assert!(binary::decode(&full[..n]).is_err(), "prefix {n}");
    }
}

#[test]
fn api_section_rejections() {
    let sec = |payload: &[u8]| {
        let mut b = b"EFVM\x01\x00\x0c".to_vec();
        b.push(payload.len() as u8);
        b.extend_from_slice(payload);
        b
    };
    let dec = |b: &[u8]| match binary::decode(b) {
        Err(LoadError::Decode { error, .. }) => error,
        r => panic!("{r:?}"),
    };
    assert_eq!(dec(&sec(&[1, 2, 3])), DecodeError::UnexpectedEnd);
    assert_eq!(dec(&sec(&[0, 0, 0, 0, 0, 0, 0, 0, 5])), DecodeError::CountTooLarge);
    assert_eq!(dec(&sec(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0xFF, 1])), DecodeError::BadUtf8);
    assert_eq!(dec(&sec(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 9])), DecodeError::SectionSizeMismatch(12));
    assert_eq!(dec(&sec(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 1, b'a', 0x80, 0x80, 0x80, 0x80, 0x10])), DecodeError::LebTooLong);
    for id in 15..=127u8 {
        let mut b = b"EFVM\x01\x00".to_vec();
        b.extend_from_slice(&[id, 0]);
        assert_eq!(dec(&b), DecodeError::UnknownSection(id));
    }
    // Section 12 must come after 11.
    let lines = ModuleData { lines: Lines { files: vec!["a".into()], funcs: vec![] }, ..ModuleData::default() };
    let mut b = sec(&[0; 9]);
    b.extend_from_slice(&binary::encode(&lines)[6..]);
    assert_eq!(dec(&b), DecodeError::SectionOutOfOrder(11));
    // The verifier rejects duplicate or invalid namespace names.
    for ns in [vec![("std", 1), ("std", 2)], vec![("", 1)], vec![("a..b", 1)]] {
        let mut mb = ModuleBuilder::new();
        mb.api_ref(1, ns);
        assert!(
            matches!(mb.finish(), Err(LoadError::Verify { error: VerifyError::ApiNamespace, .. })),
            "namespace list should be rejected"
        );
    }
}

#[test]
fn caller_memory_helpers() {
    let mut reg = HostRegistry::<Vec<String>>::new();
    reg.define_namespace("t", 1, "t", "Tests.").unwrap();
    reg.register(HostFn::new("t", "check").doc("Runs the helper checks."), |c, _a, _r| {
        let len = c.memory().len();
        assert_eq!(len, 65536);
        c.write_bytes(100, b"caf\xc3\xa9")?;
        assert_eq!(c.read_bytes(GuestSlice::new(100, 5))?, b"caf\xc3\xa9");
        assert_eq!(c.read_str(GuestSlice::new(100, 5))?, "caf\u{e9}");
        assert!(matches!(c.read_str(GuestSlice::new(100, 4)), Err(Trap::Host(_))));
        assert_eq!(c.read_bytes(GuestSlice::new(len, 0))?, b"");
        for s in [
            GuestSlice::new(len, 1),
            GuestSlice::new(len - 1, 2),
            GuestSlice::new(0, u64::MAX),
            GuestSlice::new(u64::MAX, 1),
            GuestSlice::new((-1i64) as u64, 0),
            GuestSlice::new(0, (-5i64) as u64),
            GuestSlice::new(1 << 62, 1 << 62),
        ] {
            assert_eq!(c.read_bytes(s), Err(Trap::OutOfBounds), "{s:?}");
        }
        assert_eq!(c.write_bytes(len - 1, &[1, 2]), Err(Trap::OutOfBounds));
        assert_eq!(c.write_bytes((-1i64) as u64, &[]), Err(Trap::OutOfBounds));
        c.write_u32(0, 0xDEAD_BEEF)?;
        assert_eq!(c.read_u32(0)?, 0xDEAD_BEEF);
        assert_eq!(c.memory().as_slice()[..4], [0xEF, 0xBE, 0xAD, 0xDE]);
        c.write_u64(8, u64::MAX - 1)?;
        assert_eq!(c.read_u64(8)?, u64::MAX - 1);
        c.write_f32(16, 1.5)?;
        assert_eq!(c.read_f32(16)?, 1.5);
        c.write_f64(24, -2.25)?;
        assert_eq!(c.read_f64(24)?, -2.25);
        assert_eq!(c.read_u32(len - 3), Err(Trap::OutOfBounds));
        assert_eq!(c.read_u64(u64::MAX), Err(Trap::OutOfBounds));
        assert_eq!(c.write_f64(len - 7, 0.0), Err(Trap::OutOfBounds));
        let t = StructDesc::new("V", 8, 4).field(FieldDesc::new("x", 0, Prim::F32, 2));
        c.write_struct(200, &t, &[7; 8])?;
        assert_eq!(c.read_struct(200, &t)?, vec![7; 8]);
        assert!(matches!(c.write_struct(200, &t, &[7; 4]), Err(Trap::Host(_))));
        assert_eq!(c.read_struct(len - 4, &t), Err(Trap::OutOfBounds));
        c.host().push("ok".into());
        Ok(HostOk::Continue)
    })
    .unwrap();
    let mut mb = ModuleBuilder::new();
    let check = mb.import("t", "check", FuncType::new([], []));
    mb.memory(1, Some(1));
    let mut fb = FunctionBuilder::new(1);
    fb.call(check, 0);
    fb.ret(0);
    let f = mb.add_func(FuncType::new([], []), fb);
    mb.export_func("main", f);
    let m = mb.finish().unwrap();
    let mut inst = Instance::new(&m, &reg.imports(), Limits::default()).unwrap();
    let mut log = Vec::new();
    assert_eq!(inst.call("main", &[], &mut log, 10), Ok(vec![]));
    assert_eq!(log, ["ok"]);
}

#[test]
fn single_step_debugger_hooks() {
    // f(x) = x + 1 ; main(a) = f(a) * 2
    let mut mb = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(2);
    fb.const_i64(1, 1); // f:0
    fb.op3(Op::I64Add, 0, 0, 1); // f:1
    fb.ret(0); // f:2
    let f = mb.add_func(FuncType::new([I64], [I64]), fb);
    let mut fb = FunctionBuilder::new(3);
    fb.call(f, 0); // main:0
    fb.const_i64(1, 2); // main:1
    fb.op3(Op::I64Mul, 0, 0, 1); // main:2
    fb.ret(0); // main:3
    let main = mb.add_func(FuncType::new([I64], [I64]), fb);
    mb.export_func("main", main);
    let m = mb.finish().unwrap();
    let mut inst = Instance::<()>::new(&m, &Imports::new(), Limits::default()).unwrap();
    let id = inst.spawn("main", &[20]).unwrap();
    assert_eq!(inst.fiber_location(id), Some((main, 0)));
    assert_eq!(inst.fiber_registers(id), Some(&[20u64, 0, 0][..]));
    assert_eq!(inst.fiber_backtrace(id), vec![(main, 0)]);
    let expect = [
        // (location after the step, backtrace, registers of the current frame)
        ((f, 0), vec![(f, 0), (main, 0)], vec![20, 0]),
        ((f, 1), vec![(f, 1), (main, 0)], vec![20, 1]),
        ((f, 2), vec![(f, 2), (main, 0)], vec![21, 1]),
        ((main, 1), vec![(main, 1)], vec![21, 0, 0]),
        ((main, 2), vec![(main, 2)], vec![21, 2, 0]),
        ((main, 3), vec![(main, 3)], vec![42, 2, 0]),
    ];
    for (loc, bt, regs) in expect {
        assert_eq!(inst.resume_counted(id, &mut (), 1), (Exit::OutOfBudget, 1));
        assert_eq!(inst.status(id), Some(FiberStatus::Ready));
        assert_eq!(inst.fiber_location(id), Some(loc));
        assert_eq!(inst.fiber_backtrace(id), bt);
        assert_eq!(inst.fiber_registers(id), Some(&regs[..]));
    }
    assert_eq!(inst.resume_counted(id, &mut (), 1), (Exit::Done, 1));
    assert_eq!(inst.results(id), Some(&[42u64][..]));
    assert_eq!(inst.fiber_location(id), None);
    assert_eq!(inst.fiber_registers(id), None);
    assert!(inst.fiber_backtrace(id).is_empty());
    // Suspended fibers expose their location too; trapped ones do not.
    let mut mb = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(1);
    fb.yield_();
    fb.unreachable();
    let y = mb.add_func(FuncType::new([], []), fb);
    mb.export_func("y", y);
    let mut inst = Instance::<()>::new(&mb.finish().unwrap(), &Imports::new(), Limits::default()).unwrap();
    let id = inst.spawn("y", &[]).unwrap();
    assert_eq!(inst.resume(id, &mut (), 10), Exit::Yield);
    assert_eq!(inst.fiber_location(id), Some((y, 1)));
    assert_eq!(inst.resume(id, &mut (), 10), Exit::Trap(Trap::Unreachable));
    assert_eq!(inst.fiber_location(id), None);
    assert_eq!(inst.trap_location(id), Some((y, 1)));
    inst.remove_fiber(id);
    assert_eq!(inst.fiber_location(id), None);
}
