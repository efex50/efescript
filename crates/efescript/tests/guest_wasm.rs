//! Guest mods built for wasm32 (E7.4a): the generated bindings plus `efe-guest` compile to a
//! `.wasm` whose imports, exports and custom sections follow the ABI of
//! `docs/e7.4-wasm-design.md`. Skipped, with a message, when the toolchain is missing.

mod guest_common;

use efescript::bindmodel::{wasm_lower, Wasm};
use guest_common::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use wasmparser::{CompositeInnerType, ExternalKind, Parser, Payload, TypeRef, ValType};

/// The parts of a wasm module the checks need.
#[derive(Default)]
struct Wasm_ {
    /// `(module, name)` -> (params, results)
    imports: BTreeMap<(String, String), (Vec<ValType>, Vec<ValType>)>,
    exports: BTreeMap<String, (ExternalKind, u32)>,
    func_types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// type index of every function in the function index space (imports first)
    func_type_of: Vec<u32>,
    customs: BTreeMap<String, Vec<Vec<u8>>>,
    memory: bool,
}

fn parse(bytes: &[u8]) -> Wasm_ {
    let mut w = Wasm_::default();
    let mut types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    for p in Parser::new(0).parse_all(bytes) {
        match p.expect("valid wasm") {
            Payload::TypeSection(r) => {
                for g in r {
                    for t in g.unwrap().into_types() {
                        match t.composite_type.inner {
                            CompositeInnerType::Func(f) => types.push((f.params().to_vec(), f.results().to_vec())),
                            _ => panic!("unexpected non-function type"),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.unwrap();
                    let (TypeRef::Func(t) | TypeRef::FuncExact(t)) = i.ty else { panic!("import {}::{} is not a function", i.module, i.name) };
                    w.func_type_of.push(t);
                    w.imports.insert((i.module.to_string(), i.name.to_string()), types[t as usize].clone());
                }
            }
            Payload::FunctionSection(r) => w.func_type_of.extend(r.into_iter().map(|t| t.unwrap())),
            Payload::MemorySection(r) => w.memory = r.count() > 0,
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.unwrap();
                    w.exports.insert(e.name.to_string(), (e.kind, e.index));
                }
            }
            Payload::CustomSection(c) => w.customs.entry(c.name().to_string()).or_default().push(c.data().to_vec()),
            _ => {}
        }
    }
    w.func_types = types;
    w
}

fn val(v: Wasm) -> ValType {
    match v {
        Wasm::I32 => ValType::I32,
        Wasm::I64 => ValType::I64,
        Wasm::F32 => ValType::F32,
        Wasm::F64 => ValType::F64,
    }
}

/// Every import of the fixture index as the ABI defines it: functions under `ns@version` with
/// their lowered signature, events as `<event>.send|count|read` (design doc, section 3.4).
fn expected_imports() -> BTreeMap<(String, String), (Vec<ValType>, Vec<ValType>)> {
    let ix = fixture_index();
    let version = |ns: &str| ix.namespaces.iter().find(|n| n.name == ns).unwrap().version;
    let mut m = BTreeMap::new();
    for f in &ix.functions {
        let params = f.params.iter().flat_map(|p| wasm_lower(&p.ty).iter().map(|v| val(*v))).collect();
        let results = f.results.iter().flat_map(|p| wasm_lower(&p.ty).iter().map(|v| val(*v))).collect();
        m.insert((format!("{}@{}", f.namespace, version(&f.namespace)), f.name.clone()), (params, results));
    }
    for e in &ix.events {
        let module = format!("{}@{}", e.namespace, version(&e.namespace));
        let has = e.payload.is_some();
        m.insert((module.clone(), format!("{}.send", e.name)), (if has { vec![ValType::I32] } else { vec![] }, vec![]));
        m.insert((module.clone(), format!("{}.count", e.name)), (vec![], vec![ValType::I32]));
        if has {
            m.insert((module, format!("{}.read", e.name)), (vec![ValType::I32, ValType::I32], vec![ValType::I32]));
        }
    }
    m
}

/// The checks both languages share: the `efe.api` hash, the provide records and exports.
fn check_common(w: &Wasm_) -> Vec<Rec> {
    assert!(w.memory || w.exports.contains_key("memory"), "the module has a linear memory");
    let api = w.customs.get("efe.api").expect("custom section `efe.api`");
    assert_eq!(api.len(), 1, "exactly one efe.api section (generate one bindings file from the merged index)");
    assert_eq!(api[0], fixture_index().hash().to_le_bytes(), "efe.api is the 8-byte little-endian index hash");
    let prov: Vec<u8> = w.customs.get("efe.provide").expect("custom section `efe.provide`").concat();
    let recs = decode_provide(&prov).expect("efe.provide decodes");
    assert_records(&recs);
    for r in &recs {
        if let Rec::Func { export, params, results, .. } = r {
            let (kind, idx) = *w.exports.get(export).unwrap_or_else(|| panic!("export `{export}` missing"));
            assert_eq!(kind, ExternalKind::Func);
            let (p, res) = &w.func_types[w.func_type_of[idx as usize] as usize];
            let want_p: Vec<ValType> = params.iter().flat_map(|(_, t)| lower(t)).map(val).collect();
            let want_r: Vec<ValType> = results.iter().flat_map(|(_, t)| lower(t)).map(val).collect();
            assert_eq!((p, res), (&want_p, &want_r), "export `{export}` has the lowered signature of its record");
        }
    }
    let (kind, _) = *w.exports.get("efe_scratch").expect("export `efe_scratch` (the scratch region address)");
    assert_eq!(kind, ExternalKind::Global);
    recs
}

fn wasm32_installed() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let Ok(o) = Command::new(rustc).args(["--print", "sysroot"]).current_dir(fixture("guest_mod")).output() else { return false };
    let root = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
    root.join("lib/rustlib/wasm32-unknown-unknown").is_dir()
}

fn build_rust_guest() -> PathBuf {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guest_mod");
    let o = Command::new(env!("CARGO"))
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(fixture("guest_mod/Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "2")
        // RUSTFLAGS wins over `[target.wasm32-unknown-unknown] rustflags` of any cargo config
        // (a developer's config may ask for atomics and shared, imported memory for browser
        // builds; a guest mod must not use them: design doc, section 5).
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", "-Cdebuginfo=0")
        .output()
        .expect("run cargo");
    assert!(o.status.success(), "building the guest mod failed:\n{}", String::from_utf8_lossy(&o.stderr));
    target.join("wasm32-unknown-unknown/release/guest_mod.wasm")
}

#[test]
fn rust_guest_compiles_and_follows_the_abi() {
    if !wasm32_installed() {
        eprintln!("SKIPPED: the wasm32-unknown-unknown target is not installed (rustup target add wasm32-unknown-unknown)");
        return;
    }
    let wasm = std::fs::read(build_rust_guest()).unwrap();
    let w = parse(&wasm);

    // Imports: exactly the bindings' imports, module `<ns>@<version>`, lowered types.
    let want = expected_imports();
    assert_eq!(w.imports.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>(), "the guest imports exactly the functions and events it uses");
    for (k, v) in &want {
        assert_eq!(&w.imports[k], v, "import {}::{}", k.0, k.1);
    }
    // Spot checks written out by hand, so a bug in `expected_imports` cannot hide.
    let get = &w.imports[&("engine.world@2".to_string(), "get".to_string())];
    assert_eq!(get.0, [ValType::I64, ValType::I32, ValType::I32], "entity is i64, ptr is i32");
    assert_eq!(w.imports[&("engine.world@2".into(), "spawn".into())].1, [ValType::I64]);
    assert_eq!(w.imports[&("game@1".into(), "log".into())].0, [ValType::I32, ValType::I32], "str is (ptr, len) as two i32");
    assert_eq!(w.imports[&("game@1".into(), "match".into())].0.len(), 6);
    assert_eq!(w.imports[&("game@1".into(), "move".into())].0, [ValType::I64, ValType::F32, ValType::F32, ValType::I32], "bool is i32");
    assert_eq!(w.imports[&("game.async@3".into(), "tick.send".into())].0, [ValType::I32]);

    let recs = check_common(&w);
    let funcs = recs.iter().filter(|r| matches!(r, Rec::Func { .. })).count();
    assert_eq!(funcs, 4);
    // `checked` traps through `unreachable`; the panic handler is in the module.
    assert!(w.exports.contains_key("efe_provide_checked"));
    eprintln!("rust guest: {} bytes, {} imports", wasm.len(), w.imports.len());
}

/// `Some(None)`: `wasm-ld` is on PATH. `Some(Some(path))`: the lld that rustup ships as
/// `gcc-ld/wasm-ld` (no system wasm-ld needed). `None`: neither exists.
fn wasm_ld() -> Option<Option<PathBuf>> {
    if Command::new("wasm-ld").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some(None);
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let mut roots = Vec::new();
    if let Ok(o) = Command::new(rustc).args(["--print", "sysroot"]).output() {
        roots.push(PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    }
    let home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".rustup")));
    if let Some(dir) = home.map(|h| h.join("toolchains")).and_then(|d| std::fs::read_dir(d).ok()) {
        roots.extend(dir.flatten().map(|e| e.path()));
    }
    roots.into_iter().map(|r| r.join("lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld/wasm-ld")).find(|p| p.is_file()).map(Some)
}

#[test]
fn c_guest_compiles_with_clang_when_available() {
    let tool = |n: &str| Command::new(n).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if !tool("clang") {
        eprintln!("SKIPPED: clang is not installed; the C guest was only checked natively (bindgen.rs: c_provide_records_match_the_format)");
        return;
    }
    let Some(ld) = wasm_ld() else {
        eprintln!("SKIPPED: no wasm-ld on PATH and none in a rustup toolchain");
        return;
    };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("guest_c.wasm");
    let inc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../efe-guest/include");
    let mut cmd = Command::new("clang");
    cmd.args(["--target=wasm32", "-nostdlib", "-O2", "-Wl,--no-entry", "-Wl,--export=efe_scratch"]);
    if let Some(p) = ld {
        // `--ld-path` is ignored by clang's wasm driver; an absolute `-fuse-ld` path is not.
        cmd.arg(format!("-fuse-ld={}", p.display()));
    }
    let o = cmd.arg("-I").arg(&inc).arg("-I").arg(fixture("."))
        .arg(fixture("guest_c/mod.c")).arg("-o").arg(&out)
        .output().unwrap();
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        if err.contains("unknown target") || err.contains("No available targets") {
            eprintln!("SKIPPED: this clang cannot target wasm32: {err}");
            return;
        }
        panic!("clang failed:\n{err}");
    }
    let w = parse(&std::fs::read(&out).unwrap());
    let want = expected_imports();
    for (k, v) in &w.imports {
        assert_eq!(want.get(k), Some(v), "unexpected import {}::{}", k.0, k.1);
    }
    for name in ["spawn", "is_alive", "set_transform"] {
        assert!(w.imports.contains_key(&("engine.world@2".to_string(), name.to_string())), "{name}");
    }
    // `efe.api` is a real custom section (inline assembly in the generated header). The records
    // of EFE_PROVIDE are not emitted for wasm: clang + lld turn `.custom_section.*` variables
    // into data segments, so the provides reach the translator through its fallback (mod.toml).
    let api = w.customs.get("efe.api").expect("custom section `efe.api` (inline asm in the generated header)");
    assert_eq!(api.len(), 1);
    assert_eq!(api[0], fixture_index().hash().to_le_bytes());
    assert!(!w.customs.contains_key("efe.provide"));
    for name in ["efe_provide_add", "efe_provide_weigh", "efe_provide_checked", "efe_provide_run", "efe_provide_noop"] {
        assert_eq!(w.exports.get(name).map(|e| e.0), Some(ExternalKind::Func), "export {name}");
    }
    assert_eq!(w.exports.get("efe_scratch").map(|e| e.0), Some(ExternalKind::Global));
}
