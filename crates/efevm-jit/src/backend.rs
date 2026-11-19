//! `JitBackend`: runs fibers in native code and rebuilds the interpreter's `FiberStack` on every
//! suspension.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use efevm::backend::{Backend, FiberStack, Frame, RunCx, run_interpreter};
use efevm::memory::Memory;
use efevm::{Exit, Instance, Trap};

use crate::compile::{JitCode, JitError, JitOptions, compile};
use crate::ctx::*;
use crate::helpers::h_host_call;

static NATIVE_RUNS: AtomicU64 = AtomicU64::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);
static KEEP_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Number of `run` calls that entered native code (process-wide; for tests and diagnostics).
pub fn native_runs() -> u64 {
    NATIVE_RUNS.load(Ordering::Relaxed)
}

/// Number of native-stack suspensions that were finished by the interpreter (process-wide).
pub fn native_stack_fallbacks() -> u64 {
    FALLBACKS.load(Ordering::Relaxed)
}

/// Number of those that happened while re-entering a suspended fiber (the untouched-tail case).
pub fn native_stack_resume_fallbacks() -> u64 {
    KEEP_FALLBACKS.load(Ordering::Relaxed)
}

/// A backend running the module the `JitCode` was compiled from. Any other module is
/// delegated to the interpreter.
pub struct JitBackend<H> {
    code: Arc<JitCode>,
    _h: PhantomData<fn() -> H>,
}

impl<H: 'static> JitBackend<H> {
    pub fn new(code: Arc<JitCode>) -> Self {
        JitBackend { code, _h: PhantomData }
    }
}

/// Compiles the instance's module and installs the JIT as its backend.
pub fn enable<H: 'static>(inst: &mut Instance<H>, opts: &JitOptions) -> Result<(), JitError> {
    let code = compile(inst.module(), opts)?;
    inst.set_backend(Some(Box::new(JitBackend::<H>::new(code))));
    Ok(())
}

unsafe fn mem_of<H>(p: *mut c_void) -> *mut Memory {
    // SAFETY: `p` is the `RunCx<H>` of the run (set in `run`).
    let cx = unsafe { &mut *(p as *mut RunCx<'_, H>) };
    &mut *cx.memory as *mut Memory
}

unsafe fn globals_of<H>(p: *mut c_void) -> *mut u64 {
    // SAFETY: as above.
    let cx = unsafe { &mut *(p as *mut RunCx<'_, H>) };
    cx.globals.as_mut_ptr()
}

type Entry = unsafe extern "C" fn(*mut JitCtx, *mut u64, i32) -> i32;

impl<H: 'static> Backend<H> for JitBackend<H> {
    fn run(&mut self, cx: &mut RunCx<'_, H>, stack: &mut FiberStack, budget: i64) -> (Exit, i64) {
        let code = &*self.code;
        let nimports = cx.module.data().imports.len() as u32;
        let outer = stack.frames.first().map_or(stack.func, |f| f.func);
        let ok = std::ptr::eq(cx.module.data(), code.module.data())
            && cx.module.is_leader(stack.func, stack.pc)
            && outer >= nimports
            && (outer as usize) < code.entries.len();
        if !ok {
            return run_interpreter(cx, stack, budget);
        }
        NATIVE_RUNS.fetch_add(1, Ordering::Relaxed);
        let info = cx.infos[outer as usize];
        let size0 = (info.nregs as u64).max(info.np as u64);
        let mut window = vec![0u64; (info.np.max(info.nr) as usize).max(1)];
        let here = 0u8;
        let stack_limit = (&here as *const u8 as usize).saturating_sub(code.max_native_stack);

        let infos: &[efevm::backend::FuncInfo] = cx.infos;
        let limits = cx.limits;
        let mut rt = Rt {
            cx: cx as *mut RunCx<'_, H> as *mut c_void,
            get_mem: mem_of::<H>,
            get_globals: globals_of::<H>,
            stack: stack as *mut FiberStack,
            infos: infos as *const [efevm::backend::FuncInfo],
            saved: Vec::new(),
            saved_regs: Vec::new(),
            cursor: 0,
            keep: false,
            trace: Vec::new(),
            pending: None,
            scratch: Vec::new(),
        };
        // SAFETY: `get_mem`/`get_globals` read the `RunCx` just stored in `rt.cx`.
        let (mb, ml, gp) = unsafe {
            let m = &mut *(rt.get_mem)(rt.cx);
            let ml = m.len();
            (m.as_mut_slice().as_mut_ptr(), ml, (rt.get_globals)(rt.cx))
        };
        let mut jc = JitCtx {
            mem_base: mb,
            mem_len: ml,
            globals: gp,
            budget,
            depth: 1,
            max_depth: limits.max_call_depth,
            regs_used: size0,
            max_regs: limits.max_registers as u64,
            stack_limit,
            table: code.table.as_ptr(),
            table_len: code.table.len() as u64,
            host_call: h_host_call::<H> as *const () as usize,
            rt: &mut rt as *mut Rt as *mut c_void,
            resume_inner: 0,
            suspend_kind: 0,
            trap_kind: 0,
            trap_func: 0,
            trap_inst: 0,
        };
        // SAFETY: `entries[outer]` is a function compiled with the signature of `Entry` for
        // this module (checked above); `jc` and `rt` outlive the call and `window` holds
        // `max(np, nr)` u64s. The code only touches memory through `jc`'s pointers, which are
        // refreshed after every operation that can move memory.
        let status = unsafe {
            let f: Entry = std::mem::transmute::<*const u8, Entry>(code.entries[outer as usize]);
            f(&mut jc, window.as_mut_ptr(), 1)
        };
        let budget = jc.budget;
        let kind = jc.suspend_kind;
        // From here on the raw pointers inside `rt` are no longer used.
        match status {
            0 => {
                stack.results.clear();
                stack.results.extend_from_slice(&window[..info.nr as usize]);
                (Exit::Done, budget)
            }
            2 => {
                stack.trace = std::mem::take(&mut rt.trace);
                let t = rt.pending.take().unwrap_or(Trap::Unreachable);
                (Exit::Trap(t), budget)
            }
            _ => {
                rebuild(stack, &rt);
                match kind {
                    SUSPEND_YIELD => (Exit::Yield, budget),
                    SUSPEND_BUDGET => (Exit::OutOfBudget, budget),
                    _ => {
                        FALLBACKS.fetch_add(1, Ordering::Relaxed);
                        if rt.keep {
                            KEEP_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                        }
                        run_interpreter(cx, stack, budget)
                    }
                }
            }
        }
    }
}

/// Writes the saved native activations back into the fiber stack in the interpreter's layout.
fn rebuild(stack: &mut FiberStack, rt: &Rt) {
    let m = rt.saved.len();
    let regs_of = |s: &Saved| &rt.saved_regs[s.start..s.start + s.len];
    if rt.keep {
        // Activations `m..` were never entered natively and stay as they are; `0..m` are
        // updated in place (same functions, same sizes, so the same `fp`s).
        for i in 0..m {
            let s = &rt.saved[m - 1 - i];
            let fr = &mut stack.frames[i];
            fr.pc = s.pc;
            fr.base = s.base;
            let fp = fr.fp as usize;
            stack.regs[fp..fp + s.len].copy_from_slice(regs_of(s));
        }
        return;
    }
    stack.regs.clear();
    stack.frames.clear();
    for i in 0..m {
        let s = &rt.saved[m - 1 - i];
        let fp = stack.regs.len() as u32;
        stack.regs.extend_from_slice(regs_of(s));
        if i + 1 < m {
            stack.frames.push(Frame { func: s.func, pc: s.pc, fp, base: s.base });
        } else {
            stack.func = s.func;
            stack.pc = s.pc;
            stack.fp = fp;
        }
    }
}
