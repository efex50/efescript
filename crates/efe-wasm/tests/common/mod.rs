//! Shared helpers: wat to efevm, and running exports on the interpreter and the JIT.
#![allow(dead_code)]

pub mod fuzz;
pub mod wgen;
pub mod host;
pub mod prov;

use efe_wasm::{TranslateOptions, translate};
use efevm::{ApiIndex, CallError, HostRegistry, Instance, Limits, Module, Trap};

pub fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap_or_else(|e| panic!("bad wat: {e}\n{wat}"))
}

pub fn empty_index() -> ApiIndex {
    ApiIndex::default()
}

pub fn tr(wasm_bytes: &[u8]) -> Module {
    translate(wasm_bytes, &empty_index(), &TranslateOptions::default()).unwrap_or_else(|e| panic!("translate: {e}"))
}

pub fn tr_wat(wat: &str) -> Module {
    tr(&wasm(wat))
}

pub fn tr_err(wat: &str) -> String {
    match translate(&wasm(wat), &empty_index(), &TranslateOptions::default()) {
        Ok(_) => panic!("expected a translation error for\n{wat}"),
        Err(e) => e.to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Interp,
    Jit,
}

pub fn instance(m: &Module, backend: Backend) -> Instance<()> {
    let reg = HostRegistry::<()>::new();
    let mut inst = Instance::new(m, &reg.imports(), Limits::default()).expect("instantiate");
    if backend == Backend::Jit {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let opts = efevm_jit::JitOptions { verify: true, ..Default::default() };
            match efevm_jit::enable(&mut inst, &opts) {
                Ok(()) => {}
                Err(efevm_jit::JitError::TooManyRegisters { .. }) => {}
                Err(e) => panic!("jit: {e}"),
            }
        }
    }
    inst
}

pub fn call(inst: &mut Instance<()>, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
    match inst.call(name, args, &mut (), u64::MAX) {
        Ok(r) => Ok(r),
        Err(CallError::Trap { trap, .. }) => Err(trap),
        Err(e) => panic!("call {name}: {e:?}"),
    }
}

/// Runs `name(args)` on a fresh instance on both backends, asserts they agree, returns the result.
pub fn run(m: &Module, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
    let a = call(&mut instance(m, Backend::Interp), name, args);
    let b = call(&mut instance(m, Backend::Jit), name, args);
    assert_eq!(a, b, "interpreter and JIT disagree on {name}({args:?})");
    a
}

pub fn run_wat(wat: &str, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
    run(&tr_wat(wat), name, args)
}

pub fn ok1(r: Result<Vec<u64>, Trap>) -> u64 {
    let v = r.unwrap_or_else(|t| panic!("trapped: {t}"));
    assert_eq!(v.len(), 1, "one result expected, got {v:?}");
    v[0]
}

/// An interpreter instance and a JIT instance of one module; every call runs on both and the
/// results (or traps) must agree.
pub struct Pair {
    pub a: Instance<()>,
    pub b: Instance<()>,
}

impl Pair {
    pub fn new(m: &Module) -> Pair {
        Pair { a: instance(m, Backend::Interp), b: instance(m, Backend::Jit) }
    }
    pub fn call(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
        let x = call(&mut self.a, name, args);
        let y = call(&mut self.b, name, args);
        assert_eq!(x, y, "interpreter and JIT disagree on {name}({args:?})");
        x
    }
    pub fn call1(&mut self, name: &str, args: &[u64]) -> u64 {
        ok1(self.call(name, args))
    }
}

/// The fixture index of the guest tests (also used for hand-written ABI modules).
pub fn fixture_index() -> ApiIndex {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../efescript/tests/fixtures/guest_test.efapi");
    let text = std::fs::read_to_string(path).expect("fixture index");
    ApiIndex::from_json(&text).expect("valid fixture index")
}

pub fn tr_with(wasm_bytes: &[u8], ix: &ApiIndex) -> Result<Module, efe_wasm::TranslateError> {
    translate(wasm_bytes, ix, &TranslateOptions::default())
}
