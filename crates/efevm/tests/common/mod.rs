//! Shared test helper: register raw-slot host functions through `HostRegistry` (the only way to
//! make a host function callable), with generated parameter names `p0..` / `r0..`.

#![allow(dead_code)]

use efevm::*;

/// A description of `namespace.name` with the given lowered signature.
pub fn raw_desc(namespace: &str, name: &str, ty: &FuncType) -> HostFn {
    let mut d = HostFn::new(namespace, name).doc("Test function.");
    for (i, t) in ty.params.iter().enumerate() {
        d = d.param(format!("p{i}"), ApiType::from_val(*t));
    }
    for (i, t) in ty.results.iter().enumerate() {
        d = d.result(format!("r{i}"), ApiType::from_val(*t));
    }
    d
}

/// Defines `namespace` (version 1, owner `test`) if needed, then registers `namespace.name`,
/// or replaces it if it exists, with a function over the raw argument and result slots.
pub fn try_host<H: 'static>(
    reg: &mut HostRegistry<H>,
    namespace: &str,
    name: &str,
    ty: FuncType,
    f: impl Fn(&mut Caller<'_, H>, &[u64], &mut [u64]) -> Result<HostOk, Trap> + Send + Sync + 'static,
) -> Result<(), RegistryError> {
    if reg.namespace(namespace).is_none() {
        reg.define_namespace(namespace, 1, "test", "Test functions.")?;
    }
    let d = raw_desc(namespace, name, &ty);
    let w = move |c: &mut Caller<'_, H>, a: &Args<'_>, r: &mut Results<'_>| f(c, a.raw(), r.raw());
    if reg.function(namespace, name).is_some() { reg.replace(d, w) } else { reg.register(d, w) }
}

/// `try_host` that panics on a registry error.
pub fn reg_host<H: 'static>(
    reg: &mut HostRegistry<H>,
    namespace: &str,
    name: &str,
    ty: FuncType,
    f: impl Fn(&mut Caller<'_, H>, &[u64], &mut [u64]) -> Result<HostOk, Trap> + Send + Sync + 'static,
) {
    try_host(reg, namespace, name, ty, f).expect("register test host function");
}

/// `Instance::new`, and with `EFEVM_TEST_BACKEND=jit` in the environment the Cranelift JIT
/// installed on the new instance. Every test that goes through here then checks the JIT against
/// the same expectations as the interpreter: `EFEVM_TEST_BACKEND=jit cargo test -p efevm`.
/// (Tests of the backend seam itself, `backend.rs`, use `Instance::new` directly.)
pub fn new_instance<H: 'static>(m: &Module, imports: &Imports<H>, limits: Limits) -> Result<Instance<H>, LoadError> {
    let mut inst = Instance::new(m, imports, limits)?;
    if std::env::var("EFEVM_TEST_BACKEND").is_ok_and(|v| v.eq_ignore_ascii_case("jit")) {
        let opts = efevm_jit::JitOptions { verify: true, ..Default::default() };
        match efevm_jit::enable(&mut inst, &opts) {
            Ok(()) => {}
            // Documented fallback: such a module stays on the interpreter.
            Err(efevm_jit::JitError::TooManyRegisters { .. }) => {}
            Err(e) => panic!("the JIT does not compile this test module: {e}"),
        }
    }
    Ok(inst)
}
