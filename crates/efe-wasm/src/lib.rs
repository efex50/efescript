//! `efe-wasm`: translates a `.wasm` module (a mod written in Rust or C, design doc
//! `crates/efescript/docs/e7.4-wasm-design.md`) into a verified efevm [`Module`].
//!
//! Pure data, no I/O, no unsafe: the engine's package path never links a wasm parser; the
//! translator runs at pack time (or in development builds) and the result is checked by
//! `Module::verify` like any other module.
//!
//! ```ignore
//! let index = efevm::ApiIndex::from_json(&std::fs::read_to_string("game.efapi")?)?;
//! let module = efe_wasm::translate(&std::fs::read("mod.wasm")?, &index, &Default::default())?;
//! std::fs::write("mod.efvm", module.encode())?;
//! ```
#![forbid(unsafe_code)]

mod abi;
mod error;
mod func;
mod parse;

pub use error::{Loc, TranslateError};

use abi::{Rec, wasm_lower};
use efevm::api::StructDesc;
use efevm::{
    ApiIndex, ApiType, FuncType, Interface, LoadError, Module, ModuleBuilder, Param, Provide, ProvidedEvent, ValType,
};
use func::{Ctx, Emit, EvKind, Host, Init, lower_host, translate_body};
use parse::{Parsed, WExport};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Bump on every change that can alter the output for the same input (cache keys: the wasm
/// bytes, the index hash and this number).
pub const TRANSLATOR_VERSION: u32 = 1;

/// A provided function given outside the module (design doc 4.4: `mod.toml` `[[provide]]`),
/// for toolchains that cannot emit the `efe.provide` section. Same fields as a record of kind 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackProvide {
    pub name: String,
    /// The wasm export that implements it (`efe_provide_<name>`).
    pub export: String,
    pub doc: String,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
}

/// A provided event given outside the module (a record of kind 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackEvent {
    pub name: String,
    pub payload: Option<String>,
    pub doc: String,
}

/// What `efe.provide` would say, from somewhere else. Merged with the section: a provide or event
/// named by both must be identical, a scratch size given by both must be equal; otherwise the
/// translation fails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fallback {
    pub provides: Vec<FallbackProvide>,
    pub events: Vec<FallbackEvent>,
    /// Size of the scratch region (the module must export `efe_scratch`).
    pub scratch: Option<u32>,
}

/// Options of [`translate`].
#[derive(Clone, Debug)]
pub struct TranslateOptions {
    /// Provides, events and scratch size to use besides (or instead of) the `efe.provide` section.
    pub fallback: Fallback,
    /// Copy function names (the wasm `name` section, else export names) into the names section.
    pub names: bool,
    /// Fail when the module has no `efe.api` section instead of trusting the given index
    /// (design doc 4.4).
    pub require_efe_api: bool,
}

impl Default for TranslateOptions {
    fn default() -> Self {
        TranslateOptions { fallback: Fallback::default(), names: true, require_efe_api: false }
    }
}

fn validate(wasm: &[u8]) -> Result<(), TranslateError> {
    // Only the features the subset uses. Unsupported constructs have already been rejected by
    // `parse` and `scan_body` with errors that name them.
    use wasmparser::WasmFeatures as F;
    let features = F::MUTABLE_GLOBAL | F::SATURATING_FLOAT_TO_INT | F::SIGN_EXTENSION | F::REFERENCE_TYPES | F::MULTI_VALUE | F::BULK_MEMORY | F::FLOATS;
    let mut v = wasmparser::Validator::new_with_features(features);
    v.validate_all(wasm).map(|_| ()).map_err(|e| parse::invalid(e.offset(), e.message()))
}

fn sig_text(t: &FuncType) -> String {
    t.to_string()
}

/// Translates `wasm` against `index` (the merged API index of the game and the mods it
/// depends on). The result is verified.
pub fn translate(wasm: &[u8], index: &ApiIndex, opts: &TranslateOptions) -> Result<Module, TranslateError> {
    let p = parse::parse(wasm)?;
    let nimports = p.imports.len() as u32;
    for (k, body) in p.bodies.iter().enumerate() {
        let f = nimports + k as u32;
        func::scan_body(f, p.names.get(&f), body)?;
    }
    validate(wasm)?;
    let ndefined = p.bodies.len() as u32;

    // ---- efe.api ----
    let index_hash = index.hash();
    let mut module_hash = None;
    for s in &p.api_sections {
        let Ok(b) = <[u8; 8]>::try_from(*s) else {
            return Err(TranslateError::Section { section: "efe.api", message: format!("expected 8 bytes (the index hash), found {}", s.len()) });
        };
        let h = u64::from_le_bytes(b);
        if module_hash.is_some_and(|m| m != h) {
            return Err(TranslateError::Section { section: "efe.api", message: "two efe.api sections with different hashes".into() });
        }
        module_hash = Some(h);
    }
    match module_hash {
        Some(h) if h != index_hash => return Err(TranslateError::ApiHash { module: h, index: index_hash }),
        None if opts.require_efe_api => {
            return Err(TranslateError::Section { section: "efe.api", message: "missing (the module was not built against an API index)".into() });
        }
        _ => {}
    }

    // ---- import resolution (design doc, section 3.1 to 3.4) ----
    let mut host_imports: Vec<(String, String, FuncType)> = Vec::new();
    let mut host_ids: HashMap<(String, String), u32> = HashMap::new();
    let mut versions: BTreeMap<String, u32> = BTreeMap::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut event_set: BTreeSet<(String, String)> = BTreeSet::new();
    for imp in &p.imports {
        let bad = |message: String| TranslateError::Import { module: imp.module.clone(), name: imp.name.clone(), message };
        let Some((ns, ver)) = imp.module.rsplit_once('@').filter(|(_, v)| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).and_then(|(n, v)| Some((n, v.parse::<u32>().ok()?))) else {
            return Err(bad("the import module is not of the form `<namespace>@<version>` (the guest ABI)".into()));
        };
        if index.namespace(ns).is_none() {
            return Err(TranslateError::UnknownNamespace { namespace: ns.to_string(), module: imp.module.clone() });
        }
        if let Some(prev) = versions.insert(ns.to_string(), ver) {
            if prev != ver {
                return Err(bad(format!("namespace `{ns}` is imported both as version {prev} and {ver}")));
            }
        }
        let wasm_ty = &p.types[imp.type_idx as usize];
        if let Some((ev, kind)) = imp.name.rsplit_once('.') {
            let Some(e) = index.event(ns, ev) else {
                return Err(bad(format!("event `{ns}.{ev}` is not in the API index")));
            };
            let has = e.payload.is_some();
            let (kind, want) = match kind {
                "send" => (EvKind::Send, FuncType::new(if has { vec![ValType::I32] } else { vec![] }, [])),
                "count" => (EvKind::Count, FuncType::new([], [ValType::I32])),
                "read" if has => (EvKind::Read, FuncType::new([ValType::I32, ValType::I32], [ValType::I32])),
                "read" => return Err(bad(format!("event `{ns}.{ev}` has no payload, so there is no `read`"))),
                other => return Err(bad(format!("`{other}` is not an event operation (send, count, read)"))),
            };
            if *wasm_ty != want {
                return Err(bad(format!("has {}, the event ABI says {}", sig_text(wasm_ty), sig_text(&want))));
            }
            event_set.insert((ns.to_string(), ev.to_string()));
            pending.push(Pending::Event { kind, ns: ns.to_string(), ev: ev.to_string(), payload: has });
            continue;
        }
        let Some(f) = index.function(ns, &imp.name) else {
            return Err(bad(format!("no function `{}` in namespace `{ns}`", imp.name)));
        };
        let want = abi::wasm_sig(&f.params.iter().map(|q| &q.ty).collect::<Vec<_>>(), &f.results.iter().map(|q| &q.ty).collect::<Vec<_>>());
        if *wasm_ty != want {
            let ps: Vec<&ApiType> = f.params.iter().map(|q| &q.ty).collect();
            return Err(bad(format!("has {}, the index says {} = {}", sig_text(wasm_ty), abi::api_text(&ps), sig_text(&want))));
        }
        let key = (ns.to_string(), imp.name.clone());
        let idx = *host_ids.entry(key).or_insert_with(|| {
            host_imports.push((ns.to_string(), imp.name.clone(), f.func_type()));
            (host_imports.len() - 1) as u32
        });
        let wrap: Vec<bool> = f.results.iter().flat_map(|q| wasm_lower(&q.ty).iter().map(|_| q.ty == ApiType::Ptr)).collect();
        pending.push(Pending::Fn { idx, np: f.func_type().params.len() as u32, nr: f.func_type().results.len() as u32, wrap });
    }
    // engine.events functions, only the ones the event helpers use.
    let events: Vec<(String, String)> = event_set.into_iter().collect();
    let mut ev_fn = [None; 3];
    for (k, (name, hard)) in [
        ("send", FuncType::new([ValType::I32, ValType::I64], [])),
        ("count", FuncType::new([ValType::I32], [ValType::I32])),
        ("read", FuncType::new([ValType::I32, ValType::I32, ValType::I64], [ValType::I32])),
    ]
    .into_iter()
    .enumerate()
    {
        let kind = [EvKind::Send, EvKind::Count, EvKind::Read][k];
        if !pending.iter().any(|x| matches!(x, Pending::Event { kind: kk, .. } if *kk == kind)) {
            continue;
        }
        let key = ("engine.events".to_string(), name.to_string());
        let idx = *host_ids.entry(key).or_insert_with(|| {
            let ty = index.function("engine.events", name).map_or(hard, |f| f.func_type());
            host_imports.push(("engine.events".to_string(), name.to_string(), ty));
            (host_imports.len() - 1) as u32
        });
        ev_fn[k] = Some(idx);
    }
    if !events.is_empty() {
        if let Some(n) = index.namespace("engine.events") {
            versions.entry("engine.events".to_string()).or_insert(n.version);
        }
    }
    let hosts: Vec<Host> = pending
        .into_iter()
        .map(|x| match x {
            Pending::Fn { idx, np, nr, wrap } => Host::Fn { idx, np, nr, wrap },
            Pending::Event { kind, ns, ev, payload } => Host::Event {
                kind,
                id: events.iter().position(|e| e.0 == ns && e.1 == ev).unwrap_or(0) as u32,
                payload,
                idx: ev_fn[kind as usize].unwrap_or(0),
            },
        })
        .collect();

    // ---- layout of the efevm function index space ----
    let nhost = host_imports.len() as u32;
    let def_base = nhost;
    let efunc_defined = |f: u32| def_base + f - nimports;
    let mut names: HashMap<u32, String> = p.names.clone();
    for (n, e) in &p.exports {
        if let WExport::Func(f) = e {
            names.entry(*f).or_insert_with(|| n.clone());
        }
    }

    // Imports that need a function of their own: referenced from the table or exported.
    let mut wrapper_wasm: Vec<u32> = Vec::new();
    {
        let mut seen = HashSet::new();
        let table_refs = p.elems.iter().flat_map(|(_, items, _)| items.iter().flatten().copied());
        let export_refs = p.exports.iter().filter_map(|(_, e)| if let WExport::Func(f) = e { Some(*f) } else { None });
        for f in table_refs.chain(export_refs) {
            if f < nimports && seen.insert(f) {
                wrapper_wasm.push(f);
            }
        }
    }
    let wrapper_base = def_base + ndefined;
    let wrapper_of: HashMap<u32, u32> = wrapper_wasm.iter().enumerate().map(|(k, f)| (*f, wrapper_base + k as u32)).collect();
    let efunc = |f: u32| if f >= nimports { efunc_defined(f) } else { wrapper_of[&f] };

    // ---- start function and constructors ----
    let ctors = p.exports.iter().find_map(|(n, e)| match e {
        WExport::Func(f) if n == "__wasm_call_ctors" => Some(*f),
        _ => None,
    });
    let mut init_funcs: Vec<u32> = Vec::new();
    for (what, f) in [("start function", p.start), ("`__wasm_call_ctors`", ctors)] {
        let Some(f) = f else { continue };
        if f < nimports {
            return Err(unsupported_at(format!("{what} is an imported function"), f, &names));
        }
        let t = &p.types[p.func_types[f as usize] as usize];
        if !t.params.is_empty() || !t.results.is_empty() {
            return Err(unsupported_at(format!("{what} with a signature other than () -> ()"), f, &names));
        }
        if !init_funcs.contains(&f) {
            init_funcs.push(f);
        }
    }
    let init = if init_funcs.is_empty() {
        None
    } else {
        Some(Init { global: p.globals.len() as u32, target_funcs: init_funcs.clone(), targets: init_funcs.iter().map(|f| efunc_defined(*f)).collect() })
    };
    let prologue: HashSet<u32> = if init.is_some() {
        p.exports
            .iter()
            .filter_map(|(_, e)| if let WExport::Func(f) = e { Some(*f) } else { None })
            .filter(|f| *f >= nimports)
            .collect()
    } else {
        HashSet::new()
    };

    // ---- the module ----
    let mut mb = ModuleBuilder::new();
    let etypes: Vec<u32> = p.types.iter().map(|t| mb.add_type(t.clone())).collect();
    for (ns, name, ty) in &host_imports {
        mb.import(ns, name, ty.clone());
    }
    for (ns, name) in &events {
        let e = index.event(ns, name).expect("event resolved above");
        let payload = match &e.payload {
            None => None,
            Some(s) => {
                let size = index
                    .struct_type(s)
                    .map(|d| d.size)
                    .ok_or_else(|| TranslateError::Provide { name: format!("{ns}.{name}"), message: format!("payload struct `{s}` is not in the index") })?;
                Some((s.as_str(), size))
            }
        };
        mb.event_ref(ns, name, payload);
    }
    for k in 0..ndefined {
        let f = nimports + k;
        let idx = mb.declare_func(p.types[p.func_types[f as usize] as usize].clone());
        debug_assert_eq!(idx, def_base + k);
    }
    for f in &wrapper_wasm {
        let idx = mb.declare_func(p.types[p.func_types[*f as usize] as usize].clone());
        debug_assert_eq!(idx, wrapper_of[f]);
    }

    let cx = Ctx {
        types: p.types.clone(),
        etypes,
        func_types: p.func_types.clone(),
        nimports,
        hosts,
        def_base,
        init,
        prologue,
        names,
    };

    for (k, body) in p.bodies.iter().enumerate() {
        let fb = translate_body(&cx, nimports + k as u32, body)?;
        mb.define(def_base + k as u32, fb);
    }
    for f in &wrapper_wasm {
        let ty = &p.types[p.func_types[*f as usize] as usize];
        let mut e = Emit::new();
        let np = ty.params.len() as u32;
        e.fb.set_nregs(np as u16);
        let exported = p.exports.iter().any(|(_, x)| *x == WExport::Func(*f));
        if let (Some(init), true) = (&cx.init, exported) {
            func::emit_prologue(&mut e, init, np, np + 1, false);
        }
        lower_host(&mut e, &cx.hosts[*f as usize], 0);
        e.ret(0, ty.results.len() as u32);
        mb.define(wrapper_of[f], e.fb);
    }

    // ---- memory, globals, table, data ----
    if let Some((min, max)) = p.memory {
        mb.memory(min, max);
    }
    for g in &p.globals {
        mb.global(g.ty, g.mutable, g.init);
    }
    if cx.init.is_some() {
        mb.global_i32(true, 0);
    }
    if let Some(size) = p.table_size {
        let mut entries: Vec<Option<u32>> = vec![None; size as usize];
        for (off, items, at) in &p.elems {
            let end = u64::from(*off) + items.len() as u64;
            if end > u64::from(size) {
                return Err(TranslateError::Invalid { offset: *at, message: "an element segment does not fit the table".into() });
            }
            for (k, it) in items.iter().enumerate() {
                entries[*off as usize + k] = it.map(efunc);
            }
        }
        mb.table(entries);
    }
    let mem_bytes = u128::from(p.memory.map_or(0, |m| m.0)) * 65536;
    for (off, bytes) in &p.data {
        if u128::from(*off) + bytes.len() as u128 > mem_bytes {
            return Err(TranslateError::Invalid { offset: 0, message: format!("a data segment at {off} of {} bytes does not fit the initial memory", bytes.len()) });
        }
        if !bytes.is_empty() {
            mb.data(*off, bytes);
        }
    }

    // ---- exports and names ----
    for (name, e) in &p.exports {
        match e {
            WExport::Func(f) => mb.export_func(name, efunc(*f)),
            WExport::Global(g) => mb.export_global(name, *g),
        }
    }
    if opts.names {
        for (i, (ns, name, _)) in host_imports.iter().enumerate() {
            mb.func_name(i as u32, &format!("{ns}.{name}"));
        }
        for k in 0..ndefined {
            if let Some(n) = cx.names.get(&(nimports + k)) {
                mb.func_name(def_base + k, n);
            }
        }
        for f in &wrapper_wasm {
            if let Some(n) = cx.names.get(f) {
                mb.func_name(wrapper_of[f], n);
            }
        }
    }

    // ---- section 12 ----
    mb.api_ref(index_hash, versions.iter().map(|(n, v)| (n.clone(), *v)));

    // ---- section 13 ----
    let mut recs = abi::decode_provide(&p.provide_sections.concat())?;
    merge_fallback(&mut recs, &opts.fallback)?;
    if !recs.is_empty() {
        let iface = build_interface(&p, index, &recs, &efunc, &mut mb, opts.names)?;
        // Provide wrappers were declared by `build_interface`; their bodies are defined there.
        mb.interface(iface);
    }

    mb.finish().map_err(|e| verify_error(&e, &cx, nimports, ndefined))
}

enum Pending {
    Fn { idx: u32, np: u32, nr: u32, wrap: Vec<bool> },
    Event { kind: EvKind, ns: String, ev: String, payload: bool },
}

fn unsupported_at(what: String, f: u32, names: &HashMap<u32, String>) -> TranslateError {
    TranslateError::Unsupported { what, at: Loc { func: Some(f), name: names.get(&f).cloned(), offset: None } }
}

fn verify_error(e: &LoadError, cx: &Ctx, nimports: u32, ndefined: u32) -> TranslateError {
    let mut msg = e.to_string();
    if let LoadError::Verify { func: Some(f), .. } = e {
        if *f >= cx.def_base && *f < cx.def_base + ndefined {
            let w = nimports + (*f - cx.def_base);
            let name = cx.names.get(&w).map(|n| format!(" `{n}`")).unwrap_or_default();
            msg = format!("{msg} (wasm function {w}{name})");
        }
    }
    TranslateError::Verify(msg)
}

fn efevm_type(ts: &[&ApiType], rs: &[&ApiType]) -> FuncType {
    FuncType::new(
        ts.iter().flat_map(|t| t.lower().iter().copied()),
        rs.iter().flat_map(|t| t.lower().iter().copied()),
    )
}

/// Section 13 from the `efe.provide` records (design doc 4.2). Declares and defines the
/// functions that adapt wasm's 32-bit pointers to efevm's 64-bit ones.
fn build_interface(
    p: &Parsed<'_>,
    index: &ApiIndex,
    recs: &[Rec],
    efunc: &dyn Fn(u32) -> u32,
    mb: &mut ModuleBuilder,
    names: bool,
) -> Result<Interface, TranslateError> {
    let mut iface = Interface::default();
    let copy = |iface: &mut Interface, name: &str, owner_of: &str| -> Result<(), TranslateError> {
        if iface.structs.iter().any(|s| s.name == name) {
            return Ok(());
        }
        let mut d: StructDesc = index
            .struct_type(name)
            .ok_or_else(|| TranslateError::Provide { name: owner_of.to_string(), message: format!("struct `{name}` is not in the API index") })?
            .clone();
        if d.owner.is_empty() {
            d.owner = index.game.clone();
        }
        iface.structs.push(d);
        Ok(())
    };
    for rec in recs {
        match rec {
            Rec::Scratch(size) => {
                if iface.scratch.is_some() {
                    return Err(TranslateError::Section { section: "efe.provide", message: "more than one scratch record".into() });
                }
                let g = p.exports.iter().find_map(|(n, e)| match e {
                    WExport::Global(g) if n == "efe_scratch" => Some(*g),
                    _ => None,
                });
                let Some(g) = g else {
                    return Err(TranslateError::Section { section: "efe.provide", message: "a scratch record needs the export `efe_scratch` (a global holding the region's address)".into() });
                };
                let gl = p.globals.get(g as usize).ok_or_else(|| TranslateError::Section { section: "efe.provide", message: "`efe_scratch` is not a defined global".into() })?;
                if gl.ty != ValType::I32 || gl.mutable {
                    return Err(TranslateError::Section {
                        section: "efe.provide",
                        message: format!("`efe_scratch` must be an immutable i32 global (the region's address), it is a {}{}", if gl.mutable { "mutable " } else { "" }, gl.ty),
                    });
                }
                iface.scratch = Some((gl.init & 0xFFFF_FFFF, *size));
            }
            Rec::Event { name, payload, doc } => {
                if let Some(s) = payload {
                    copy(&mut iface, s, name)?;
                }
                iface.events.push(ProvidedEvent { name: name.clone(), payload: payload.clone(), doc: doc.clone() });
            }
            Rec::Func { name, export, doc, params, results } => {
                let bad = |message: String| TranslateError::Provide { name: name.clone(), message };
                let f = p
                    .exports
                    .iter()
                    .find_map(|(n, e)| match e {
                        WExport::Func(f) if n == export => Some(*f),
                        _ => None,
                    })
                    .ok_or_else(|| bad(format!("the export `{export}` is missing")))?;
                let ps: Vec<&ApiType> = params.iter().map(|(_, t)| t).collect();
                let rs: Vec<&ApiType> = results.iter().map(|(_, t)| t).collect();
                let want = abi::wasm_sig(&ps, &rs);
                let have = &p.types[p.func_types[f as usize] as usize];
                if *have != want {
                    return Err(bad(format!(
                        "export `{export}` has {}, the record {} lowers to {}",
                        sig_text(have),
                        abi::api_text(&ps),
                        sig_text(&want)
                    )));
                }
                for (_, t) in params {
                    if let ApiType::Struct(s) = t {
                        copy(&mut iface, s, name)?;
                    }
                }
                let ety = efevm_type(&ps, &rs);
                let widen = params.iter().any(|(_, t)| matches!(t, ApiType::Str | ApiType::Bytes | ApiType::Struct(_)));
                let func = if widen {
                    // wasm pointers are i32, efevm's are i64: a function with the efevm signature
                    // that wraps them and calls the real function.
                    let idx = mb.declare_func(ety);
                    let mut e = Emit::new();
                    let np = have.params.len() as u32;
                    e.fb.set_nregs(np as u16);
                    let mut slot = 0;
                    for (_, t) in params {
                        for _ in wasm_lower(t) {
                            if matches!(t, ApiType::Str | ApiType::Bytes | ApiType::Struct(_)) {
                                e.wrap(np + slot, slot);
                            } else {
                                e.mov(np + slot, slot);
                            }
                            slot += 1;
                        }
                    }
                    e.call(efunc(f), np, np.max(have.results.len() as u32));
                    e.ret(np, have.results.len() as u32);
                    mb.define(idx, e.fb);
                    if names {
                        mb.func_name(idx, &format!("{export}$abi"));
                    }
                    idx
                } else {
                    efunc(f)
                };
                iface.provides.push(Provide {
                    name: name.clone(),
                    func,
                    doc: doc.clone(),
                    params: params.iter().map(|(n, t)| Param::new(n.clone(), t.clone())).collect(),
                    results: results.iter().map(|(n, t)| Param::new(n.clone(), t.clone())).collect(),
                });
            }
        }
    }
    Ok(iface)
}

/// Merges the fallback into the records of the section (design doc 4.4): what both name must agree.
fn merge_fallback(recs: &mut Vec<Rec>, fb: &Fallback) -> Result<(), TranslateError> {
    let params = |ps: &[Param]| ps.iter().map(|p| (p.name.clone(), p.ty.clone())).collect::<Vec<_>>();
    for f in &fb.provides {
        let rec = Rec::Func { name: f.name.clone(), export: f.export.clone(), doc: f.doc.clone(), params: params(&f.params), results: params(&f.results) };
        match recs.iter().find(|r| matches!(r, Rec::Func { name, .. } if *name == f.name)) {
            Some(existing) if *existing != rec => {
                return Err(TranslateError::Provide { name: f.name.clone(), message: "the fallback and the efe.provide record of the module disagree".into() });
            }
            Some(_) => {}
            None => recs.push(rec),
        }
    }
    for e in &fb.events {
        let rec = Rec::Event { name: e.name.clone(), payload: e.payload.clone(), doc: e.doc.clone() };
        match recs.iter().find(|r| matches!(r, Rec::Event { name, .. } if *name == e.name)) {
            Some(existing) if *existing != rec => {
                return Err(TranslateError::Provide { name: e.name.clone(), message: "the fallback and the efe.provide event record of the module disagree".into() });
            }
            Some(_) => {}
            None => recs.push(rec),
        }
    }
    if let Some(size) = fb.scratch {
        match recs.iter().find_map(|r| if let Rec::Scratch(n) = r { Some(*n) } else { None }) {
            Some(n) if n != size => {
                return Err(TranslateError::Section { section: "efe.provide", message: format!("the scratch size is {n} in the module and {size} in the fallback") });
            }
            Some(_) => {}
            None => recs.push(Rec::Scratch(size)),
        }
    }
    Ok(())
}
