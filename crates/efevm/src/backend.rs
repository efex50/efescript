//! The backend seam: lets another crate (a JIT) run a fiber instead of the interpreter, using
//! only safe access to the instance state (SPEC section 7, "Backends").
//!
//! # Charging rule
//!
//! A backend follows exactly the interpreter's budget rule (SPEC section 5): the budget is a
//! signed counter. Whenever control is at a block leader about to execute it (at the start of
//! the run, after a jump or branch, falling through, entering a callee, returning to a caller),
//! a counter `<= 0` stops the fiber with `Exit::OutOfBudget` positioned at that leader;
//! otherwise the block's cost is subtracted and the block runs, even if the counter goes
//! negative. Bulk memory operations subtract their extra cost the same way with no immediate
//! exit. `Instance::resume` turns a negative final counter into debt, for both paths, in one
//! place; a backend only returns the counter.

use crate::imports::{Caller, HostFunc, HostOk};
use crate::instance::FiberId;
use crate::memory::Memory;
use crate::module::Module;
use crate::trap::{Exit, Trap};
use crate::types::Limits;

/// Per-function data precomputed at instantiation (whole function index space).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuncInfo {
    /// Parameter count.
    pub np: u32,
    /// Result count.
    pub nr: u32,
    /// Register count of the function (0 for imports).
    pub nregs: u32,
    /// Structural type id: equal for equal `FuncType`s (what `call_indirect` compares).
    pub canon: u32,
}

/// A suspended caller activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The caller's function index.
    pub func: u32,
    /// Return pc in the caller: the index of the `call` instruction plus 1.
    pub pc: u32,
    /// The caller's frame base in `FiberStack::regs`.
    pub fp: u32,
    /// The caller's call window base register (where the results go).
    pub base: u32,
}

impl Frame {
    /// `(caller function, index of the call instruction)`.
    pub fn call_site(&self) -> (u32, u32) {
        (self.func, self.pc.saturating_sub(1))
    }
}

/// A fiber's execution state, shared by the interpreter and backends.
///
/// Invariants for a `Ready` or `Suspended` fiber (what a backend must leave behind on `Yield`
/// and `OutOfBudget`, and may rely on at entry):
/// - `regs` holds the registers of every activation back to back. The innermost activation
///   owns `regs[fp .. regs.len()]`; the activation of `frames[i]` owns
///   `regs[frames[i].fp .. fp_of_the_next_activation]`. Every activation owns exactly
///   `max(nregs, nparams)` registers of its function.
/// - `frames` are the suspended callers, outermost first. The entry function has no frame.
/// - `func`, `pc` and `fp` describe the innermost activation; `pc` is the next instruction to
///   execute. On `Yield` that is the instruction after the `yield` or `call`; on `OutOfBudget`
///   it is a block leader.
/// - `results` is filled on `Done`; `trace` on `Trap` (innermost first, `(func, inst index)`,
///   where outer entries are the call instructions).
/// - `debt` is owned by `Instance::resume`; a backend does not touch it.
#[derive(Debug, Default)]
pub struct FiberStack {
    pub regs: Vec<u64>,
    pub frames: Vec<Frame>,
    pub(crate) scratch: Vec<u64>,
    pub func: u32,
    pub pc: u32,
    pub fp: u32,
    pub results: Vec<u64>,
    pub trace: Vec<(u32, u32)>,
    /// Budget units still owed by an earlier overrun; paid first by the next resume.
    pub debt: u64,
}

/// Safe access to everything a backend needs to run a fiber: the module, per-function info,
/// memory, globals and host calls.
pub struct RunCx<'a, H> {
    pub module: &'a Module,
    /// One entry per function index (imports first).
    pub infos: &'a [FuncInfo],
    /// Canonical type id per type index (compare with `FuncInfo::canon` for `call_indirect`).
    pub type_canon: &'a [u32],
    pub limits: Limits,
    pub memory: &'a mut Memory,
    pub globals: &'a mut [u64],
    /// The fiber being run.
    pub fiber: FiberId,
    pub(crate) host: &'a mut H,
    pub(crate) hosts: &'a [HostFunc<H>],
    pub(crate) gmut: &'a [bool],
    pub(crate) gmask: &'a [u64],
    pub(crate) res_masks: &'a [Box<[u64]>],
}

impl<H> RunCx<'_, H> {
    /// Calls import `import` (a function index below the import count) with `args` (length =
    /// its parameter count). `results` (length = its result count) is zeroed, written by the
    /// host function and masked to the result types (32-bit results keep the low half), exactly
    /// as the interpreter does. A host `Err` is returned as is; on `Ok(HostOk::Yield)` the
    /// caller must suspend the fiber after storing the results. A bad index or a wrong
    /// `results` length is `Trap::Unreachable`.
    pub fn call_host(&mut self, import: u32, args: &[u64], results: &mut [u64]) -> Result<HostOk, Trap> {
        invoke_host(
            self.hosts,
            self.res_masks,
            self.host,
            self.memory,
            self.globals,
            self.gmut,
            self.gmask,
            self.fiber,
            import,
            args,
            results,
        )
    }
}

/// The one implementation of a host call; the interpreter calls it with the pieces of `RunCx`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_host<H>(
    hosts: &[HostFunc<H>],
    res_masks: &[Box<[u64]>],
    host: &mut H,
    memory: &mut Memory,
    globals: &mut [u64],
    gmut: &[bool],
    gmask: &[u64],
    fiber: FiberId,
    import: u32,
    args: &[u64],
    results: &mut [u64],
) -> Result<HostOk, Trap> {
    let Some(hf) = hosts.get(import as usize) else { return Err(Trap::Unreachable) };
    let Some(masks) = res_masks.get(import as usize).filter(|m| m.len() == results.len()) else {
        return Err(Trap::Unreachable);
    };
    results.fill(0);
    let k = {
        let mut caller = Caller::new(host, memory, globals, gmut, gmask, fiber);
        hf(&mut caller, args, results)?
    };
    for (d, m) in results.iter_mut().zip(masks.iter()) {
        *d &= *m;
    }
    Ok(k)
}

/// Something that can run a fiber instead of the interpreter (`Instance::set_backend`).
///
/// `run` receives a `Ready` or `Suspended` fiber positioned at a block leader (`Instance`
/// only calls it then; otherwise the interpreter runs the fiber) and a signed budget counter.
/// It follows the charging rule in the module docs and returns the exit and the final
/// counter, which may be negative; `Instance::resume` converts that to debt for backends and
/// the interpreter alike.
///
/// - `Yield` / `OutOfBudget`: `stack` must be left in exactly the interpreter's layout (see
///   `FiberStack`), so either side can resume it.
/// - `Done`: the results are in `stack.results`.
/// - `Trap`: the call stack is in `stack.trace`, innermost first, `(func, inst index)`.
///
/// A backend may delegate to `run_interpreter`. It is `Send` because `Instance` is (the engine
/// runs script lanes on worker threads); a backend that owns raw code pointers makes that
/// promise itself.
pub trait Backend<H>: Send {
    fn run(&mut self, cx: &mut RunCx<'_, H>, stack: &mut FiberStack, budget: i64) -> (Exit, i64);
}

/// Runs the fiber in the interpreter under the charging rule; same contract as `Backend::run`.
pub fn run_interpreter<H>(cx: &mut RunCx<'_, H>, stack: &mut FiberStack, budget: i64) -> (Exit, i64) {
    crate::interp::run::<H, false>(cx, stack, budget)
}
