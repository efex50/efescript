//! The real guests of E7.4a (`crates/efescript/tests/fixtures/{guest_mod,guest_c}`): built for
//! wasm32, translated against `guest_test.efapi`, checked against what the bindings describe
//! and run on a mock host. Skipped, with a printed note, when the toolchain is missing.
mod common;
use common::host::*;
use common::*;
use efevm::{ApiType, Exit, Instance, Limits, Module, Param, Trap};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../efescript/tests/fixtures").join(name)
}

/// Sysroots of the toolchains to look in: the active one first, then every installed one
/// (a build server's default toolchain may lack the wasm32 target another one has).
fn sysroots() -> Vec<PathBuf> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let mut v = Vec::new();
    if let Ok(o) = Command::new(rustc).args(["--print", "sysroot"]).output() {
        v.push(PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    }
    let home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".rustup")));
    if let Some(dir) = home.map(|h| h.join("toolchains")).and_then(|d| std::fs::read_dir(d).ok()) {
        let mut found: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
        found.sort();
        v.extend(found);
    }
    v
}

/// The sysroot of a toolchain that has the wasm32-unknown-unknown target.
fn wasm32_toolchain() -> Option<PathBuf> {
    sysroots().into_iter().find(|r| r.join("lib/rustlib/wasm32-unknown-unknown").is_dir() && r.join("bin/cargo").is_file())
}

fn build_rust_guest(toolchain: &Path) -> Vec<u8> {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guest_mod");
    let o = Command::new(toolchain.join("bin/cargo"))
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(fixture("guest_mod/Cargo.toml"))
        .env("RUSTC", toolchain.join("bin/rustc"))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "2")
        // RUSTFLAGS wins over a cargo config's `[target.wasm32-unknown-unknown] rustflags`
        // (the repo config asks for atomics and shared, imported memory for the web build).
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", "-Cdebuginfo=0")
        .output()
        .expect("run cargo");
    assert!(o.status.success(), "building the guest mod failed:\n{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read(target.join("wasm32-unknown-unknown/release/guest_mod.wasm")).expect("guest wasm")
}

/// `Some(None)`: `wasm-ld` is on PATH. `Some(Some(path))`: the one rustup ships (it is lld, as
/// `wasm-ld`). `None`: neither.
fn wasm_ld() -> Option<Option<PathBuf>> {
    if Command::new("wasm-ld").arg("--version").output().is_ok_and(|o| o.status.success()) {
        return Some(None);
    }
    sysroots()
        .into_iter()
        .map(|r| r.join("lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld/wasm-ld"))
        .find(|p| p.is_file())
        .map(Some)
}

fn build_c_guest() -> Option<Vec<u8>> {
    if !Command::new("clang").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("SKIPPED: clang is not installed");
        return None;
    }
    let Some(ld) = wasm_ld() else {
        eprintln!("SKIPPED: no wasm-ld (neither on PATH nor in a rustup toolchain)");
        return None;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guest_c.wasm");
    let inc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../efe-guest/include");
    let mut cmd = Command::new("clang");
    cmd.args(["--target=wasm32", "-nostdlib", "-O2", "-Wl,--no-entry", "-Wl,--export=efe_scratch"]);
    if let Some(p) = ld {
        // (`--ld-path` is ignored by the wasm driver; an absolute `-fuse-ld` is not.)
        cmd.arg(format!("-fuse-ld={}", p.display()));
    }
    let o = cmd.arg("-I").arg(&inc).arg("-I").arg(fixture(".")).arg(fixture("guest_c/mod.c")).arg("-o").arg(&out).output().unwrap();
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        if err.contains("unknown target") || err.contains("No available targets") {
            eprintln!("SKIPPED: this clang cannot target wasm32: {err}");
            return None;
        }
        panic!("clang failed:\n{err}");
    }
    Some(std::fs::read(&out).unwrap())
}

/// The provides every example guest has (docs and types as in `guest_common::expected_records`).
fn expected_provides() -> Vec<(&'static str, &'static str, Vec<Param>, Vec<Param>)> {
    let p = |n: &str, t: ApiType| Param::new(n, t);
    let res = |t: ApiType| vec![Param::new("result", t)];
    vec![
        ("add", "Adds two numbers, saturating.", vec![p("a", ApiType::I32), p("b", ApiType::I32)], res(ApiType::I32)),
        (
            "weigh",
            "Sums the bytes of `data`, the length of `name`, and the score and handle it is given.",
            vec![p("name", ApiType::Str), p("data", ApiType::Bytes), p("cfg", ApiType::Struct("Score".into())), p("who", ApiType::Entity), p("on", ApiType::Bool)],
            res(ApiType::F64),
        ),
        ("checked", "Traps when `n` is negative.", vec![p("n", ApiType::I32)], res(ApiType::I32)),
        ("run", "Calls imports of the bindings.", vec![p("n", ApiType::I32)], res(ApiType::I32)),
    ]
}

/// What the C guest's provides would say in mod.toml (design doc 4.4): the toolchain does not
/// emit the `efe.provide` section for wasm.
fn c_fallback() -> efe_wasm::Fallback {
    let mut provides: Vec<efe_wasm::FallbackProvide> = expected_provides()
        .into_iter()
        .map(|(name, doc, params, results)| efe_wasm::FallbackProvide { name: name.into(), export: format!("efe_provide_{name}"), doc: doc.into(), params, results })
        .collect();
    provides.push(efe_wasm::FallbackProvide { name: "noop".into(), export: "efe_provide_noop".into(), doc: "No parameters and no result.".into(), params: vec![], results: vec![] });
    efe_wasm::Fallback {
        provides,
        events: vec![
            efe_wasm::FallbackEvent { name: "coin_picked".into(), payload: Some("Score".into()), doc: "A coin was picked up.".into() },
            efe_wasm::FallbackEvent { name: "level_restarted".into(), payload: None, doc: "The level restarted.".into() },
        ],
        scratch: Some(4096),
    }
}

fn instance(m: &Module, jit: bool) -> Instance<Mock> {
    let reg = registry(&fixture_index());
    let mut inst = Instance::new(m, &reg.imports(), Limits::default()).expect("link the translated guest against the mock host");
    if jit {
        efevm_jit::enable(&mut inst, &efevm_jit::JitOptions { verify: true, ..Default::default() }).expect("jit");
    }
    inst
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Call the functions section 13 provides (what the engine does).
    Provides,
    /// The module has no `efe.provide` section: call the `efe_provide_<name>` exports.
    Exports,
}

fn invoke(inst: &mut Instance<Mock>, m: &Module, host: &mut Mock, mode: Mode, name: &str, args: &[u64]) -> (Exit, Option<Vec<u64>>) {
    let id = match mode {
        Mode::Provides => {
            let f = m.data().interface.as_ref().unwrap().provide(name).unwrap_or_else(|| panic!("provide {name}")).func;
            inst.spawn_func(f, args).expect("spawn")
        }
        Mode::Exports => inst.spawn(&format!("efe_provide_{name}"), args).expect("spawn"),
    };
    let exit = inst.resume(id, host, u64::MAX);
    let res = inst.results(id).map(<[u64]>::to_vec);
    inst.remove_fiber(id);
    (exit, res)
}

fn custom_sections(wasm: &[u8]) -> Vec<String> {
    wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| match p {
            Ok(wasmparser::Payload::CustomSection(c)) => Some(c.name().to_string()),
            _ => None,
        })
        .collect()
}

/// `run_status`: what `run(3)` returns against the mock host (3 events readable per frame).
fn check_guest(wasm: &[u8], rust: bool, run_status: u32, fallback: efe_wasm::Fallback) {
    let ix = fixture_index();
    let m = efe_wasm::translate(wasm, &ix, &efe_wasm::TranslateOptions { fallback, ..Default::default() }).unwrap_or_else(|e| panic!("translate: {e}"));
    let d = m.data();
    let has_provide = d.interface.is_some();
    let mode = if has_provide { Mode::Provides } else { Mode::Exports };
    let scratch_at;

    if has_provide {
        // Section 13 against what the bindings describe.
        let iface = d.interface.as_ref().expect("section 13");
        for (name, doc, params, results) in expected_provides() {
            let p = iface.provide(name).unwrap_or_else(|| panic!("provide {name}"));
            assert_eq!((&p.doc[..], &p.params, &p.results), (doc, &params, &results), "provide {name}");
        }
        assert_eq!(iface.provides.len(), if rust { 4 } else { 5 }, "{:?}", iface.provides.iter().map(|p| &p.name).collect::<Vec<_>>());
        let mut evs: Vec<(String, Option<String>, String)> = iface.events.iter().map(|e| (e.name.clone(), e.payload.clone(), e.doc.clone())).collect();
        evs.sort();
        assert_eq!(
            evs,
            [
                ("coin_picked".to_string(), Some("Score".to_string()), "A coin was picked up.".to_string()),
                ("level_restarted".to_string(), None, "The level restarted.".to_string())
            ]
        );
        assert_eq!(iface.structs.iter().map(|s| (s.name.as_str(), s.owner.as_str(), s.size)).collect::<Vec<_>>(), [("Score", "game", 16)]);
        let (at, len) = iface.scratch.expect("scratch region");
        assert_eq!(len, 4096);
        scratch_at = at;
        // `Interface::index` is the index of the mod: the provides as functions of the mod.
        let own = iface.index("guest", 1, "The guest.");
        own.validate().expect("the mod's own index is valid");
        assert_eq!(own.functions.len(), iface.provides.len());
        assert_eq!(own.events.len(), 2);
        let again = efevm::ApiIndex::from_json(&own.to_json()).expect("the own index round trips as a .efapi");
        assert_eq!(again, own);
    } else {
        eprintln!(
            "NOTE: this guest has no `efe.provide` section (custom sections: {:?}): the toolchain did not emit it, so section 13 is \
             not checked and the provided functions are called through their `efe_provide_*` exports",
            custom_sections(wasm)
        );
        assert!(d.interface.is_none());
        let g = d.exports.iter().find_map(|e| match (e.name.as_str(), e.kind) {
            ("efe_scratch", efevm::ExportKind::Global(g)) => Some(g),
            _ => None,
        });
        scratch_at = instance(&m, false).global(g.expect("export efe_scratch")).unwrap();
    }

    // Section 12: the namespaces the guest imports from, at the versions of its import modules.
    let api = d.api.as_ref().expect("section 12");
    assert_eq!(api.hash, ix.hash());
    for (n, v) in [("engine.world", 2), ("game", 1)] {
        assert!(api.namespaces.contains(&(n.to_string(), v)), "{n}@{v} in {:?}", api.namespaces);
    }
    // Section 14: the events the guest sends or reads, sorted; the Rust guest uses all three.
    let events: Vec<String> = d.events.iter().map(|e| e.qualified()).collect();
    if rust {
        assert_eq!(events, ["game.round_reset", "game.score_changed", "game.async.tick"]);
        assert!(api.namespaces.contains(&("game.async".to_string(), 3)));
    } else {
        assert_eq!(events, ["game.round_reset", "game.score_changed"]);
    }
    assert_eq!(d.events.iter().find(|e| e.name == "score_changed").unwrap().payload, Some(("Score".to_string(), 16)));
    // It round-trips through the binary format.
    assert_eq!(Module::decode(&m.encode()).expect("decode").data(), d);
    // The names section carries something (export names at least).
    assert!(!d.names.funcs.is_empty());

    for jit in [false, true] {
        let mut inst = instance(&m, jit);
        let mut host = Mock::default();
        let mut call = |inst: &mut Instance<Mock>, name: &str, args: &[u64]| invoke(inst, &m, &mut host, mode, name, args);
        assert_eq!(call(&mut inst, "add", &[2, 3]), (Exit::Done, Some(vec![5])));
        assert_eq!(call(&mut inst, "add", &[0x7fff_ffff, 1]), (Exit::Done, Some(vec![0x7fff_ffff])), "saturates");
        assert_eq!(call(&mut inst, "add", &[0x8000_0000, 0xffff_ffff]), (Exit::Done, Some(vec![0x8000_0000])), "saturates down");
        assert_eq!(call(&mut inst, "checked", &[7]), (Exit::Done, Some(vec![7])));
        assert_eq!(call(&mut inst, "checked", &[(-1i32) as u32 as u64]).0, Exit::Trap(Trap::Unreachable));
        // weigh: arguments are copied into the scratch region by the embedder.
        let base = scratch_at;
        inst.memory_mut().write(base, b"ab").unwrap();
        inst.memory_mut().write(base + 16, &[1, 2, 3]).unwrap();
        let mut score = [0u8; 16];
        score[8..12].copy_from_slice(&10i32.to_le_bytes());
        inst.memory_mut().write(base + 32, &score).unwrap();
        let (exit, res) = call(&mut inst, "weigh", &[base, 2, base + 16, 3, base + 32, 1, 1]);
        assert_eq!(exit, Exit::Done);
        assert_eq!(f64::from_bits(res.unwrap()[0]), 6.0 + 2.0 + 10.0 + 1.0);
        let (exit, res) = call(&mut inst, "weigh", &[base, 2, base + 16, 3, base + 32, 1, 0]);
        assert_eq!((exit, f64::from_bits(res.unwrap()[0])), (Exit::Done, 6.0 + 2.0 + 1.0));
        // run: touches the imports.
        let (exit, res) = call(&mut inst, "run", &[3]);
        assert_eq!(exit, Exit::Done, "run");
        assert_eq!(res, Some(vec![run_status as u64]), "run(3) on {}", if jit { "the JIT" } else { "the interpreter" });
        let log = &host.log;
        assert!(log.iter().any(|l| l.starts_with("engine.world.spawn")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("game.log")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("engine.events.send")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("engine.events.count")), "{log:?}");
    }
}

#[test]
fn rust_guest_translates_and_runs() {
    let Some(toolchain) = wasm32_toolchain() else {
        eprintln!("SKIPPED: no installed toolchain has the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown)");
        return;
    };
    let wasm = build_rust_guest(&toolchain);
    eprintln!("rust guest: {} bytes of wasm", wasm.len());
    // 0x23456789 (the low half of the mock's pointer) + 15 events counted + 3 pushes.
    check_guest(&wasm, true, 0x2345_6789 + 18, Default::default());
}

#[test]
fn c_guest_translates_and_runs() {
    let Some(wasm) = build_c_guest() else { return };
    eprintln!("c guest: {} bytes of wasm", wasm.len());
    // `efe.api` is a real custom section now (inline assembly in the generated header).
    let sections = custom_sections(&wasm);
    assert!(sections.iter().any(|n| n == "efe.api"), "custom sections: {sections:?}");
    // The records of `EFE_PROVIDE` are not emitted for wasm (clang + lld make data segments of
    // `.custom_section.*` variables): the provides come from the fallback.
    assert!(!sections.iter().any(|n| n == "efe.provide"), "custom sections: {sections:?}");
    // Without the fallback the module translates but provides nothing.
    let bare = efe_wasm::translate(&wasm, &fixture_index(), &Default::default()).expect("translates without a fallback");
    assert!(bare.data().interface.is_none());
    // set_transform 0, is_alive 0, count 3, scale 0, match 0, and p[0] = n (3).
    check_guest(&wasm, false, 6, c_fallback());
}
