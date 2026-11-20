//! Differential fuzzing against `wasmi`: a random module (`gen.rs`) runs in wasmi, in the
//! efevm interpreter and in the efevm JIT; results, traps, memory and globals must agree.
#![allow(dead_code)]

use super::wgen::{Gen, Rng, Sig, Ty, gen_module, rand_arg};
use super::{Backend, instance};
use efe_wasm::{TranslateOptions, translate};
use efevm::{CallError, Instance, Trap};
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct Outcome {
    pub calls: u64,
    /// How the calls ended: "ok" or the trap.
    pub ends: BTreeMap<String, u64>,
    pub wasm_bytes: usize,
}

fn to_val(t: Ty, bits: u64) -> wasmi::Val {
    match t {
        Ty::I32 => wasmi::Val::I32(bits as u32 as i32),
        Ty::I64 => wasmi::Val::I64(bits as i64),
        Ty::F32 => wasmi::Val::F32(wasmi::F32::from_bits(bits as u32)),
        Ty::F64 => wasmi::Val::F64(wasmi::F64::from_bits(bits)),
    }
}

fn from_val(v: &wasmi::Val) -> u64 {
    match v {
        wasmi::Val::I32(x) => *x as u32 as u64,
        wasmi::Val::I64(x) => *x as u64,
        wasmi::Val::F32(x) => x.to_bits() as u64,
        wasmi::Val::F64(x) => x.to_bits(),
        other => panic!("unexpected value {other:?}"),
    }
}

fn zero(t: Ty) -> wasmi::Val {
    to_val(t, 0)
}

fn trap_of(e: &wasmi::Error) -> Result<Trap, String> {
    use wasmi::TrapCode as T;
    match e.as_trap_code() {
        Some(T::UnreachableCodeReached) => Ok(Trap::Unreachable),
        Some(T::MemoryOutOfBounds) => Ok(Trap::OutOfBounds),
        Some(T::TableOutOfBounds) | Some(T::IndirectCallToNull) => Ok(Trap::UndefinedElement),
        Some(T::BadSignature) => Ok(Trap::IndirectCallTypeMismatch),
        Some(T::IntegerDivisionByZero) => Ok(Trap::DivideByZero),
        Some(T::IntegerOverflow) => Ok(Trap::IntegerOverflow),
        Some(T::BadConversionToInteger) => Ok(Trap::InvalidConversion),
        Some(T::StackOverflow) => Ok(Trap::StackOverflow),
        other => Err(format!("unexpected wasmi error {other:?}: {e}")),
    }
}

fn run_efevm(inst: &mut Instance<()>, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
    match inst.call(name, args, &mut (), u64::MAX) {
        Ok(r) => Ok(r),
        Err(CallError::Trap { trap, .. }) => Err(trap),
        Err(e) => Err(Trap::Host(format!("{e:?}"))),
    }
}

fn dump(seed: u64, wat: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("efe_wasm_fuzz_{seed}.wat"));
    match std::fs::write(&path, wat) {
        Ok(()) => path.display().to_string(),
        Err(_) => "(could not write the module)".into(),
    }
}

pub fn fuzz_case(seed: u64) -> Result<Outcome, String> {
    let g: Gen = gen_module(seed);
    let fail = |msg: String| -> String { format!("seed {seed}: {msg}\n  module: {}", dump(seed, &g.wat)) };
    let wasm = wat::parse_str(&g.wat).map_err(|e| fail(format!("the generator produced invalid wat: {e}")))?;

    // wasmi
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &wasm).map_err(|e| fail(format!("wasmi rejects the generated module: {e}")))?;
    let mut store = wasmi::Store::new(&engine, ());
    let linker = wasmi::Linker::<()>::new(&engine);
    let winst = linker.instantiate_and_start(&mut store, &module).map_err(|e| fail(format!("wasmi instantiate: {e}")))?;

    // efevm
    let m = translate(&wasm, &Default::default(), &TranslateOptions::default()).map_err(|e| fail(format!("translate: {e}")))?;
    let mut interp = instance(&m, Backend::Interp);
    let mut jit = instance(&m, Backend::Jit);

    let mut out = Outcome { wasm_bytes: wasm.len(), ..Default::default() };
    let mut rng = Rng::new(seed ^ 0xA5A5_5A5A);
    for round in 0..3 {
        for (name, sig) in &g.funcs {
            let Sig { params, results } = sig;
            let args: Vec<u64> = params.iter().map(|t| rand_arg(&mut rng, *t)).collect();
            let wf = winst.get_func(&store, name).ok_or_else(|| fail(format!("no export {name}")))?;
            let wargs: Vec<wasmi::Val> = params.iter().zip(&args).map(|(t, b)| to_val(*t, *b)).collect();
            let mut wres: Vec<wasmi::Val> = results.iter().map(|t| zero(*t)).collect();
            let want: Result<Vec<u64>, Trap> = match wf.call(&mut store, &wargs, &mut wres) {
                Ok(()) => Ok(wres.iter().map(from_val).collect()),
                Err(e) => Err(trap_of(&e).map_err(|m| fail(format!("{name}{args:?}: {m}")))?),
            };
            let got_i = run_efevm(&mut interp, name, &args);
            let got_j = run_efevm(&mut jit, name, &args);
            out.calls += 1;
            *out.ends.entry(match &want { Ok(_) => "ok".to_string(), Err(t) => format!("{t}") }).or_default() += 1;
            if got_i != want {
                return Err(fail(format!("round {round}: {name}({args:x?}): wasmi says {want:x?}, the interpreter says {got_i:x?}")));
            }
            if got_j != want {
                return Err(fail(format!("round {round}: {name}({args:x?}): wasmi says {want:x?}, the JIT says {got_j:x?}")));
            }
            // State after every call: memory (and its size) and globals.
            let wmem = winst.get_memory(&store, "mem").unwrap().data(&store);
            for (what, inst) in [("interpreter", &interp), ("JIT", &jit)] {
                if inst.memory().as_slice() != wmem {
                    let pos = inst.memory().as_slice().iter().zip(wmem).position(|(a, b)| a != b);
                    return Err(fail(format!(
                        "round {round}, after {name}({args:x?}): memory differs between wasmi ({} bytes) and the {what} ({} bytes), first difference at {pos:?}",
                        wmem.len(),
                        inst.memory().as_slice().len()
                    )));
                }
            }
            for (idx, gname, t) in &g.globals {
                let w = from_val(&winst.get_global(&store, gname).unwrap().get(&store));
                for (what, inst) in [("interpreter", &interp), ("JIT", &jit)] {
                    let v = inst.global(*idx).unwrap();
                    if v != w {
                        return Err(fail(format!("round {round}, after {name}({args:x?}): global {gname} ({t:?}) is {w:#x} in wasmi, {v:#x} in the {what}")));
                    }
                }
            }
        }
    }
    Ok(out)
}
