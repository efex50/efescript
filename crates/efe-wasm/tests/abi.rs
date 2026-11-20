//! The guest ABI on hand-written modules: import resolution, events, `efe.api`, `efe.provide`
//! and section 12/13/14 of the result.
mod common;
use common::host::*;
use common::prov::*;
use common::*;
use efe_wasm::{TranslateError, TranslateOptions, translate};
use efevm::{Exit, Instance, Limits, Trap};

const IMPORTS: &str = r#"
  (import "game@1" "log" (func $log (param i32 i32)))
  (import "game@1" "pointer" (func $pointer (param i32) (result i32)))
  (import "game@1" "scale" (func $scale (param f64) (result f64)))
  (import "game@1" "score_changed.send" (func $send (param i32)))
  (import "game@1" "score_changed.count" (func $count (result i32)))
  (import "game@1" "score_changed.read" (func $read (param i32 i32) (result i32)))
  (import "game@1" "round_reset.send" (func $rr))
  (import "game.async@3" "tick.send" (func $tick (param i32)))
  (import "engine.world@2" "spawn" (func $spawn (result i64)))
"#;

fn go_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module {IMPORTS}
  (memory (export "memory") 1)
  (func (export "go") (result i32)
    i32.const 16 i32.const 5 call $log
    i32.const 8 call $pointer
    call $spawn drop
    f64.const 1.5 call $scale drop
    i32.const 64 call $send
    call $rr
    i32.const 32 call $tick
    call $count drop
    i32.const 0 i32.const 128 call $read
    i32.add)
  (func (export "indirect") (result i32)
    ;; a host function called through the table: needs a wrapper with the wasm signature
    i32.const 8 i32.const 1 call_indirect (param i32) (result i32))
  (table 2 funcref)
  (elem (i32.const 1) $pointer))"#
    ))
}

fn run_go(m: &efevm::Module, name: &str) -> (Result<Vec<u64>, efevm::CallError>, Vec<String>) {
    let ix = fixture_index();
    let reg = registry(&ix);
    let mut inst = Instance::new(m, &reg.imports(), Limits::default()).expect("link");
    let mut host = Mock::default();
    let r = inst.call(name, &[], &mut host, u64::MAX);
    (r, host.log)
}

#[test]
fn imports_events_and_sections() {
    let ix = fixture_index();
    let m = tr_with(&go_module(), &ix).unwrap_or_else(|e| panic!("{e}"));
    let d = m.data();

    // Imports: each host function once, then engine.events (only send/count/read the module uses).
    let imports: Vec<String> = d.imports.iter().map(|i| format!("{}.{}", i.namespace, i.name)).collect();
    assert_eq!(
        imports,
        ["game.log", "game.pointer", "game.scale", "engine.world.spawn", "engine.events.send", "engine.events.count", "engine.events.read"],
        "{imports:?}"
    );
    // Section 14: sorted by (namespace, name); the id is the position.
    let events: Vec<(String, String, Option<(String, u32)>)> = d.events.iter().map(|e| (e.namespace.clone(), e.name.clone(), e.payload.clone())).collect();
    assert_eq!(
        events,
        [
            ("game".to_string(), "round_reset".to_string(), None),
            ("game".to_string(), "score_changed".to_string(), Some(("Score".to_string(), 16))),
            ("game.async".to_string(), "tick".to_string(), Some(("Wide".to_string(), 8))),
        ]
    );
    // Section 12: the index hash and the namespaces imported from, with the guest's versions.
    let api = d.api.as_ref().expect("section 12");
    assert_eq!(api.hash, ix.hash());
    let mut ns = api.namespaces.clone();
    ns.sort();
    assert_eq!(ns, [("engine.world".to_string(), 2), ("game".to_string(), 1), ("game.async".to_string(), 3)]);
    assert!(d.interface.is_none(), "no provide records, no section 13");
    // It encodes and decodes (all sections are well-formed).
    let again = efevm::Module::decode(&m.encode()).expect("round trip");
    assert_eq!(again.data(), m.data());

    // Run it against the mock host (both backends).
    let (r, log) = run_go(&m, "go");
    // pointer() returns 0x1_2345_6789 as a ptr; the guest sees the low 32 bits. read() returns 0.
    assert_eq!(r.unwrap(), vec![0x2345_6789]);
    assert_eq!(
        log,
        [
            "game.log[16, 5]",
            "game.pointer[8]",
            "engine.world.spawn[]",
            "game.scale[4609434218613702656]",
            "engine.events.send[1, 64]",
            "engine.events.send[0, 0]",
            "engine.events.send[2, 32]",
            "engine.events.count[1]",
            "engine.events.read[1, 0, 128]",
        ]
    );
    // The same function called through the table: the wrapper wraps the ptr result too.
    let (r, _) = run_go(&m, "indirect");
    assert_eq!(r.unwrap(), vec![0x2345_6789]);
}

#[test]
fn import_errors_name_the_import() {
    let ix = fixture_index();
    let bad = |imp: &str| match tr_with(&wasm(&format!("(module {imp})")), &ix) {
        Ok(_) => panic!("{imp} should be rejected"),
        Err(e) => e.to_string(),
    };
    let e = bad(r#"(import "game@1" "log" (func (param i32 i64)))"#);
    assert_eq!(e, "import `game@1::log`: has (i32, i64), the index says (str) = (i32, i32)", "{e}");
    let e = bad(r#"(import "game@1" "nope" (func))"#);
    assert!(e.contains("no function `nope` in namespace `game`"), "{e}");
    let e = bad(r#"(import "nothere@1" "f" (func))"#);
    assert!(e.contains("namespace `nothere` is not in the API index"), "{e}");
    let e = bad(r#"(import "game" "log" (func))"#);
    assert!(e.contains("<namespace>@<version>"), "{e}");
    let e = bad(r#"(import "game@x" "log" (func))"#);
    assert!(e.contains("<namespace>@<version>"), "{e}");
    let e = bad(r#"(import "game@1" "log" (func (param i32 i32))) (import "game@2" "pointer" (func (param i32) (result i32)))"#);
    assert!(e.contains("both as version 1 and 2"), "{e}");
    // Events.
    let e = bad(r#"(import "game@1" "unknown_event.send" (func))"#);
    assert!(e.contains("event `game.unknown_event` is not in the API index"), "{e}");
    let e = bad(r#"(import "game@1" "round_reset.read" (func (param i32 i32) (result i32)))"#);
    assert!(e.contains("has no payload"), "{e}");
    let e = bad(r#"(import "game@1" "round_reset.frob" (func))"#);
    assert!(e.contains("not an event operation"), "{e}");
    let e = bad(r#"(import "game@1" "score_changed.send" (func (param i64)))"#);
    assert!(e.contains("has (i64), the event ABI says (i32)"), "{e}");
    let e = bad(r#"(import "game@1" "score_changed.count" (func (result i64)))"#);
    assert!(e.contains("the event ABI says () -> i32"), "{e}");
}

#[test]
fn efe_api_hash_is_the_build_skew_check() {
    let ix = fixture_index();
    let base = wasm(r#"(module (import "game@1" "scale" (func (param f64) (result f64))))"#);
    // Right hash: accepted.
    let ok = with_custom(base.clone(), "efe.api", &ix.hash().to_le_bytes());
    tr_with(&ok, &ix).expect("right hash");
    // Wrong hash: the error names both.
    let bad = with_custom(base.clone(), "efe.api", &(ix.hash() ^ 1).to_le_bytes());
    match tr_with(&bad, &ix) {
        Err(TranslateError::ApiHash { module, index }) => assert_eq!((module, index), (ix.hash() ^ 1, ix.hash())),
        other => panic!("{other:?}"),
    }
    // Wrong length.
    let short = with_custom(base.clone(), "efe.api", &[1, 2, 3]);
    assert!(matches!(tr_with(&short, &ix), Err(TranslateError::Section { section: "efe.api", .. })));
    // Missing: accepted (the index's hash is used) unless the caller insists.
    let m = tr_with(&base, &ix).unwrap();
    assert_eq!(m.data().api.as_ref().unwrap().hash, ix.hash());
    let r = translate(&base, &ix, &TranslateOptions { require_efe_api: true, ..Default::default() });
    assert!(matches!(r, Err(TranslateError::Section { section: "efe.api", .. })), "{r:?}");
}

fn provide_module(extra: &[u8]) -> Vec<u8> {
    let w = wasm(
        r#"(module
  (memory (export "memory") 1)
  (global (export "efe_scratch") i32 (i32.const 1024))
  (func (export "efe_provide_add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)
  ;; weigh(name: str, data: bytes, cfg: &Score, who: entity, on: bool) -> f64
  (func (export "efe_provide_weigh") (param i32 i32 i32 i32 i32 i64 i32) (result f64)
    local.get 5 f64.convert_i64_u
    local.get 3 f64.convert_i32_u f64.add
    local.get 1 f64.convert_i32_u f64.add
    local.get 0 f64.convert_i32_u f64.add
    local.get 4 f64.convert_i32_u f64.add
    local.get 2 f64.convert_i32_u f64.add
    local.get 6 f64.convert_i32_u f64.add)
  (func (export "efe_provide_checked") (param i32) (result i32)
    local.get 0 i32.const 0 i32.lt_s if unreachable end local.get 0)
  (func (export "other") (result i32) i32.const 1))"#,
    );
    let mut recs = Vec::new();
    recs.extend(scratch(4096));
    recs.extend(event("coin_picked", Some("Score"), "A coin was picked up."));
    recs.extend(event("level_restarted", None, "The level restarted."));
    recs.extend(func("add", "efe_provide_add", "Adds.", &[("a", I32, None), ("b", I32, None)], &[("result", I32, None)]));
    recs.extend(func(
        "weigh",
        "efe_provide_weigh",
        "Weighs.",
        &[("name", STR, None), ("data", BYTES, None), ("cfg", STRUCT, Some("Score")), ("who", ENTITY, None), ("on", BOOL, None)],
        &[("result", F64, None)],
    ));
    recs.extend(func("checked", "efe_provide_checked", "Traps on negative.", &[("n", I32, None)], &[("result", I32, None)]));
    recs.extend(extra);
    with_custom(w, "efe.provide", &recs)
}

#[test]
fn provide_records_become_section_13() {
    let ix = fixture_index();
    let m = tr_with(&provide_module(&[]), &ix).unwrap_or_else(|e| panic!("{e}"));
    let i = m.data().interface.as_ref().expect("section 13");
    assert_eq!(i.scratch, Some((1024, 4096)));
    assert_eq!(i.structs.len(), 1);
    assert_eq!((i.structs[0].name.as_str(), i.structs[0].owner.as_str(), i.structs[0].size), ("Score", "game", 16));
    assert_eq!(i.events.iter().map(|e| (e.name.as_str(), e.payload.as_deref())).collect::<Vec<_>>(), [("coin_picked", Some("Score")), ("level_restarted", None)]);
    assert_eq!(i.provides.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["add", "weigh", "checked"]);
    // The mod's own index (what the registry holds once the mod is loaded).
    let own = i.index("mymod", 1, "A test mod.");
    own.validate().expect("valid index");
    assert_eq!(own.qualified_names(), ["mymod.add", "mymod.checked", "mymod.weigh"]);
    assert_eq!(own.events.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["coin_picked", "level_restarted"]);

    // Run the provides the way the engine does: spawn the provided function with the lowered
    // (i64 pointers) signature.
    let reg = registry(&ix);
    for backend in [Backend::Interp, Backend::Jit] {
        let mut inst = Instance::new(&m, &reg.imports(), Limits::default()).expect("link");
        if backend == Backend::Jit {
            efevm_jit::enable(&mut inst, &efevm_jit::JitOptions::default()).expect("jit");
        }
        let mut host = Mock::default();
        let mut spawn = |inst: &mut Instance<Mock>, name: &str, args: &[u64]| {
            let f = m.data().interface.as_ref().unwrap().provide(name).unwrap().func;
            let id = inst.spawn_func(f, args).unwrap();
            let exit = inst.resume(id, &mut host, u64::MAX);
            let res = inst.results(id).map(|r| r.to_vec());
            (exit, res)
        };
        assert_eq!(spawn(&mut inst, "add", &[2, 40]), (Exit::Done, Some(vec![42])));
        // weigh(name=(1000, 5), data=(1100, 7), cfg=1200, who=9, on=1): the wasm function sums
        // all of them as f64; the pointers arrive as i64 and are wrapped for the guest.
        let (exit, res) = spawn(&mut inst, "weigh", &[1000, 5, 1100, 7, 1200, 9, 1]);
        assert_eq!(exit, Exit::Done);
        assert_eq!(f64::from_bits(res.unwrap()[0]), 9.0 + 7.0 + 5.0 + 1000.0 + 1200.0 + 1100.0 + 1.0, "weigh sums its arguments");
        assert_eq!(spawn(&mut inst, "checked", &[5]), (Exit::Done, Some(vec![5])));
        let (exit, _) = spawn(&mut inst, "checked", &[(-1i32) as u32 as u64]);
        assert_eq!(exit, Exit::Trap(Trap::Unreachable));
    }
}

#[test]
fn unknown_record_kinds_are_skipped() {
    let ix = fixture_index();
    let m = tr_with(&provide_module(&record(3, vec![1, 2, 3, 4])), &ix).expect("kind 3 is skipped");
    assert_eq!(m.data().interface.as_ref().unwrap().provides.len(), 3);
    let m = tr_with(&provide_module(&record(200, vec![])), &ix).expect("kind 200 is skipped");
    assert_eq!(m.data().interface.as_ref().unwrap().provides.len(), 3);
}

#[test]
fn provide_errors() {
    let ix = fixture_index();
    let err = |extra: &[u8]| tr_with(&provide_module(extra), &ix).err().map(|e| e.to_string()).expect("an error");
    // The export of a record is missing.
    let e = err(&func("ghost", "efe_provide_ghost", "A ghost.", &[], &[]));
    assert!(e.contains("provide `ghost`") && e.contains("`efe_provide_ghost` is missing"), "{e}");
    // The export's signature is not the lowering of the record.
    let e = err(&func("addx", "efe_provide_add", "Wrong.", &[("a", I64, None), ("b", I32, None)], &[("result", I32, None)]));
    assert!(e.contains("provide `addx`") && e.contains("has (i32, i32) -> i32") && e.contains("lowers to (i64, i32) -> i32"), "{e}");
    // A struct the index does not have.
    let e = err(&func("s", "efe_provide_checked", "Struct.", &[("n", STRUCT, Some("Nope"))], &[]));
    assert!(e.contains("struct `Nope` is not in the API index") || e.contains("lowers to"), "{e}");
    // Garbage in a record.
    let e = err(&record(0, vec![0xff]));
    assert!(e.contains("efe.provide"), "{e}");
    // A second scratch record.
    let e = err(&scratch(8));
    assert!(e.contains("more than one scratch"), "{e}");
    // A record whose body has trailing bytes.
    let mut body = string("x");
    body.extend([1, 2, 3]);
    let e = err(&record(2, body));
    assert!(e.contains("efe.provide"), "{e}");
    // A scratch record without the export `efe_scratch`.
    let w = with_custom(wasm("(module (memory 1))"), "efe.provide", &scratch(64));
    let e = tr_with(&w, &ix).err().unwrap().to_string();
    assert!(e.contains("efe_scratch"), "{e}");
    // The verifier's rules still apply: a doc must not be blank.
    let e = err(&func("blank", "efe_provide_checked", " ", &[("n", I32, None)], &[("result", I32, None)]));
    assert!(e.contains("does not verify"), "{e}");
}

#[test]
fn start_in_a_mod_with_provides_runs_before_the_first_provide() {
    let ix = fixture_index();
    let w = wasm(
        r#"(module
  (global $c (mut i32) (i32.const 0))
  (func $start i32.const 41 global.set $c)
  (start $start)
  (func (export "efe_provide_get") (result i32) global.get $c i32.const 1 i32.add))"#,
    );
    let w = with_custom(w, "efe.provide", &func("get", "efe_provide_get", "Gets.", &[], &[("result", I32, None)]));
    let m = tr_with(&w, &ix).unwrap();
    let reg = registry(&ix);
    let mut inst = Instance::new(&m, &reg.imports(), Limits::default()).unwrap();
    let f = m.data().interface.as_ref().unwrap().provides[0].func;
    let id = inst.spawn_func(f, &[]).unwrap();
    assert_eq!(inst.resume(id, &mut Mock::default(), u64::MAX), Exit::Done);
    assert_eq!(inst.results(id).unwrap(), [42]);
}

#[test]
fn padded_leb128_in_records_is_accepted() {
    // The C header writes two-byte (zero padded) lengths; readers accept any valid LEB128.
    fn pad(v: u32) -> Vec<u8> {
        vec![(v & 0x7f) as u8 | 0x80, (v >> 7) as u8]
    }
    fn pstr(s: &str) -> Vec<u8> {
        let mut o = pad(s.len() as u32);
        o.extend(s.as_bytes());
        o
    }
    let mut body = pstr("add");
    body.extend(pstr("efe_provide_add"));
    body.extend(pstr("Adds."));
    body.extend(pad(2));
    for n in ["a", "b"] {
        body.extend(pstr(n));
        body.push(I32);
    }
    body.extend(pad(1));
    body.extend(pstr("result"));
    body.push(I32);
    let mut rec = vec![0u8];
    rec.extend(pad(body.len() as u32));
    rec.extend(body);
    let w = with_custom(wasm(r#"(module (func (export "efe_provide_add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#), "efe.provide", &rec);
    let m = tr_with(&w, &fixture_index()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(m.data().interface.as_ref().unwrap().provides[0].name, "add");
    // Too long a LEB128 (six bytes) is rejected.
    let mut bad = vec![0u8, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00];
    bad.extend([0, 0]);
    let w = with_custom(wasm(r#"(module (func (export "efe_provide_add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#), "efe.provide", &bad);
    assert!(matches!(tr_with(&w, &fixture_index()), Err(TranslateError::Section { section: "efe.provide", .. })));
}

#[test]
fn efe_scratch_must_be_an_immutable_i32_global() {
    let ix = fixture_index();
    for g in ["(global (export \"efe_scratch\") (mut i32) (i32.const 8))", "(global (export \"efe_scratch\") f32 (f32.const 1.0))", "(global (export \"efe_scratch\") i64 (i64.const 8))"] {
        let w = with_custom(wasm(&format!("(module (memory 1) {g})")), "efe.provide", &scratch(16));
        let e = tr_with(&w, &ix).err().expect("rejected").to_string();
        assert!(e.contains("immutable i32 global"), "{g}: {e}");
    }
    let w = with_custom(wasm("(module (memory 1) (global (export \"efe_scratch\") i32 (i32.const 8)))"), "efe.provide", &scratch(16));
    assert_eq!(tr_with(&w, &ix).unwrap().data().interface.as_ref().unwrap().scratch, Some((8, 16)));
}

#[test]
fn an_exported_import_runs_the_start_function_first() {
    let ix = fixture_index();
    let m = tr_with(
        &wasm(
            r#"(module
      (import "game@1" "scale" (func $scale (param f64) (result f64)))
      (global $c (mut i32) (i32.const 0))
      (func $start i32.const 7 global.set $c)
      (start $start)
      (export "s" (func $scale))
      (func (export "get") (result i32) global.get $c))"#,
        ),
        &ix,
    )
    .unwrap();
    let reg = registry(&ix);
    for jit in [false, true] {
        let mut inst = Instance::new(&m, &reg.imports(), Limits::default()).unwrap();
        if jit {
            efevm_jit::enable(&mut inst, &efevm_jit::JitOptions::default()).unwrap();
        }
        let mut host = Mock::default();
        inst.call("s", &[0], &mut host, u64::MAX).unwrap();
        assert_eq!(inst.call("get", &[], &mut host, u64::MAX).unwrap(), vec![7]);
        assert_eq!(host.log, ["game.scale[0]"]);
    }
}

fn fallback_add() -> efe_wasm::FallbackProvide {
    efe_wasm::FallbackProvide {
        name: "add".into(),
        export: "efe_provide_add".into(),
        doc: "Adds.".into(),
        params: vec![efevm::Param::new("a", efevm::ApiType::I32), efevm::Param::new("b", efevm::ApiType::I32)],
        results: vec![efevm::Param::new("result", efevm::ApiType::I32)],
    }
}

#[test]
fn fallback_provides_stand_in_for_the_section() {
    let ix = fixture_index();
    let bare = wasm(
        r#"(module (memory 1) (global (export "efe_scratch") i32 (i32.const 256))
      (func (export "efe_provide_add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
    );
    let fb = efe_wasm::Fallback {
        provides: vec![fallback_add()],
        events: vec![efe_wasm::FallbackEvent { name: "coin".into(), payload: Some("Score".into()), doc: "A coin.".into() }],
        scratch: Some(128),
    };
    let opts = TranslateOptions { fallback: fb.clone(), ..Default::default() };
    let m = translate(&bare, &ix, &opts).unwrap_or_else(|e| panic!("{e}"));
    let i = m.data().interface.as_ref().unwrap();
    assert_eq!(i.provides[0].name, "add");
    assert_eq!(i.scratch, Some((256, 128)));
    assert_eq!(i.events[0].name, "coin");
    assert_eq!(i.structs[0].name, "Score");

    // The section and the fallback may both name a provide if they agree...
    let with_section = with_custom(bare.clone(), "efe.provide", &func("add", "efe_provide_add", "Adds.", &[("a", I32, None), ("b", I32, None)], &[("result", I32, None)]));
    let m = translate(&with_section, &ix, &opts).unwrap();
    assert_eq!(m.data().interface.as_ref().unwrap().provides.len(), 1);
    // ...and fail if they do not.
    let mut other = fb.clone();
    other.provides[0].doc = "Something else.".into();
    let e = translate(&with_section, &ix, &TranslateOptions { fallback: other, ..Default::default() }).err().unwrap().to_string();
    assert!(e.contains("provide `add`") && e.contains("disagree"), "{e}");
    let with_scratch = with_custom(bare, "efe.provide", &scratch(64));
    let e = translate(&with_scratch, &ix, &opts).err().unwrap().to_string();
    assert!(e.contains("scratch size is 64") && e.contains("128"), "{e}");
}
