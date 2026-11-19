//! Instances: memory, globals, resolved imports and fibers.

use crate::error::{CallError, LinkError, LoadError, SpawnError};
use crate::imports::{HostFunc, Imports};
use crate::backend::{Backend, FiberStack, FuncInfo, RunCx, run_interpreter};
use crate::interp;
use crate::memory::Memory;
use crate::module::{ExportKind, Module};
use crate::trap::{Exit, Trap};
use crate::types::{FuncType, Limits, ValType};
use std::collections::HashMap;

/// Handle to a fiber. Stale handles (after `remove_fiber`) are rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FiberId {
    index: u32,
    generation: u32,
}

/// Lifecycle state of a fiber.
///
/// `Ready`: spawned, or stopped by `OutOfBudget`, resumable without having yielded.
/// `Suspended`: stopped by `yield` (guest or host).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FiberStatus {
    Ready,
    Suspended,
    Done,
    Trapped(Trap),
}

struct Fiber {
    status: FiberStatus,
    stack: FiberStack,
    /// Call stack at the trap, innermost first (only for trapped fibers).
    trace: Vec<(u32, u32)>,
}

/// Mask keeping the low 32 bits for i32/f32, everything for i64/f64.
fn val_mask(t: ValType) -> u64 {
    match t {
        ValType::I32 | ValType::F32 => 0xFFFF_FFFF,
        ValType::I64 | ValType::F64 => u64::MAX,
    }
}

struct Slot {
    generation: u32,
    fiber: Option<Fiber>,
}

/// A module instance: memory, globals and fibers, parameterised by the embedder's host state.
pub struct Instance<H> {
    module: Module,
    memory: Memory,
    globals: Vec<u64>,
    gmut: Vec<bool>,
    gmask: Vec<u64>,
    res_masks: Vec<Box<[u64]>>,
    infos: Vec<FuncInfo>,
    type_canon: Vec<u32>,
    hosts: Vec<HostFunc<H>>,
    limits: Limits,
    slots: Vec<Slot>,
    free: Vec<u32>,
    live: u32,
    backend: Option<Box<dyn Backend<H>>>,
}

impl<H> Instance<H> {
    /// Links imports by (namespace, name) and exact type, checks the namespace versions of
    /// the module's `api` section (`Imports::check_module`; the first problem is returned),
    /// allocates memory, copies data segments and initialises globals.
    pub fn new(module: &Module, imports: &Imports<H>, limits: Limits) -> Result<Self, LoadError> {
        let data = module.data();
        if let Some(e) = imports.check_module(module).into_iter().next() {
            return Err(LoadError::Link(e));
        }
        let mut hosts = Vec::with_capacity(data.imports.len());
        for imp in &data.imports {
            let (_, f) = imports.get(&imp.namespace, &imp.name).ok_or_else(|| {
                LoadError::Link(LinkError::MissingImport { namespace: imp.namespace.clone(), name: imp.name.clone() })
            })?;
            hosts.push(f.clone());
        }

        let too_large = LoadError::Link(LinkError::MemoryTooLarge {
            min_pages: data.memory.min_pages,
            cap: limits.max_memory_pages,
        });
        if data.memory.min_pages > limits.max_memory_pages {
            return Err(too_large);
        }
        let max = data.memory.max_pages.unwrap_or(u32::MAX).min(limits.max_memory_pages);
        let mut memory = Memory::new(data.memory.min_pages, max).ok_or_else(|| too_large.clone())?;
        for seg in &data.data {
            memory.write(seg.offset, &seg.bytes).map_err(|_| too_large.clone())?;
        }

        let mut type_canon = Vec::with_capacity(data.types.len());
        let mut first: HashMap<&FuncType, u32> = HashMap::with_capacity(data.types.len());
        for (i, t) in data.types.iter().enumerate() {
            type_canon.push(*first.entry(t).or_insert(i as u32));
        }
        let res_masks: Vec<Box<[u64]>> = data
            .imports
            .iter()
            .map(|imp| {
                data.types
                    .get(imp.type_idx as usize)
                    .map(|t| t.results.iter().map(|r| val_mask(*r)).collect())
                    .unwrap_or_default()
            })
            .collect();
        let gmask: Vec<u64> = data.globals.iter().map(|g| val_mask(g.ty)).collect();
        let mut infos = Vec::new();
        for i in 0..data.num_funcs() {
            let (ty, nregs): (Option<&FuncType>, u32) = if (i as usize) < data.imports.len() {
                (data.func_type(i), 0)
            } else {
                let fun = &data.functions[i as usize - data.imports.len()];
                (data.func_type(i), fun.nregs as u32)
            };
            let (np, nr) = ty.map_or((0, 0), |t| (t.params.len() as u32, t.results.len() as u32));
            let tidx = if (i as usize) < data.imports.len() {
                data.imports[i as usize].type_idx
            } else {
                data.functions[i as usize - data.imports.len()].type_idx
            };
            let canon = type_canon.get(tidx as usize).copied().unwrap_or(u32::MAX);
            infos.push(FuncInfo { np, nr, nregs, canon });
        }

        Ok(Instance {
            module: module.clone(),
            memory,
            globals: data.globals.iter().map(|g| g.init & val_mask(g.ty)).collect(),
            gmut: data.globals.iter().map(|g| g.mutable).collect(),
            gmask,
            res_masks,
            infos,
            type_canon,
            hosts,
            limits,
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
            backend: None,
        })
    }

    /// Sets (or clears) the backend that runs fibers instead of the interpreter. `resume` uses
    /// it only when the fiber is positioned at a block leader (`Module::is_leader`); a fiber
    /// stopped in the middle of a block (by `step`) is run by the interpreter, and so is every
    /// `step`. Fibers suspended by either side can be resumed by the other.
    pub fn set_backend(&mut self, backend: Option<Box<dyn Backend<H>>>) {
        self.backend = backend;
    }

    /// Whether a backend is set.
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }

    /// Spawns a fiber at an exported function. `args` are raw register bits; the upper 32 bits
    /// of `i32`/`f32` arguments are ignored (cleared).
    pub fn spawn(&mut self, export: &str, args: &[u64]) -> Result<FiberId, SpawnError> {
        let data = self.module.data();
        let found = data.exports.iter().find(|e| e.name == export);
        match found.map(|e| e.kind) {
            None => Err(SpawnError::NoSuchExport),
            Some(ExportKind::Global(_)) => Err(SpawnError::NotAFunction),
            Some(ExportKind::Func(f)) => self.spawn_func(f, args),
        }
    }

    /// Spawns a fiber at a function index (module functions only, not imports). The upper 32
    /// bits of `i32`/`f32` arguments are ignored (cleared). An index outside the function index
    /// space is `SpawnError::NotAFunction`.
    pub fn spawn_func(&mut self, func: u32, args: &[u64]) -> Result<FiberId, SpawnError> {
        let nimports = self.module.data().imports.len();
        if (func as usize) < nimports {
            return Err(SpawnError::ImportEntry);
        }
        let info = *self.infos.get(func as usize).ok_or(SpawnError::NotAFunction)?;
        if args.len() != info.np as usize {
            return Err(SpawnError::ArgCount { expected: info.np, got: args.len() as u32 });
        }
        if self.live >= self.limits.max_fibers {
            return Err(SpawnError::TooManyFibers);
        }
        let size = (info.nregs as usize).max(args.len());
        if size as u64 > self.limits.max_registers as u64 || self.limits.max_call_depth < 1 {
            return Err(SpawnError::StackOverflow);
        }
        let mut stack = FiberStack { func, ..FiberStack::default() };
        if stack.regs.try_reserve_exact(size).is_err() {
            return Err(SpawnError::StackOverflow);
        }
        stack.regs.resize(size, 0);
        if let Some(t) = self.module.data().func_type(func) {
            for ((d, s), p) in stack.regs.iter_mut().zip(args).zip(&t.params) {
                *d = *s & val_mask(*p);
            }
        }
        let fiber = Fiber { status: FiberStatus::Ready, stack, trace: Vec::new() };
        self.live += 1;
        let id = if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.fiber = Some(fiber);
            FiberId { index, generation: slot.generation }
        } else {
            self.slots.push(Slot { generation: 0, fiber: Some(fiber) });
            FiberId { index: (self.slots.len() - 1) as u32, generation: 0 }
        };
        Ok(id)
    }

    fn fiber(&self, id: FiberId) -> Option<&Fiber> {
        let s = self.slots.get(id.index as usize)?;
        if s.generation != id.generation {
            return None;
        }
        s.fiber.as_ref()
    }

    /// Runs `fiber` for at most about `budget` budget units, charged per basic block.
    ///
    /// The budget is a signed counter during the run. Whenever control is at a block leader
    /// about to execute it (the start of the run, after a jump or branch, falling through,
    /// entering a callee, returning to a caller), a counter `<= 0` stops the fiber with
    /// `OutOfBudget` at that leader; otherwise the block's cost (its instruction count, see
    /// `Module::block_cost`) is subtracted and the whole block runs, even if the counter goes
    /// negative. `memory.copy` and `memory.fill` also subtract `len / 256`, and `memory.grow`
    /// `pages * 65536 / 256` (saturating, also when the grow fails), with no immediate exit.
    /// When the run ends with a negative counter, the overrun becomes a per-fiber debt that
    /// the next resume pays first: if `budget <= debt` the debt shrinks by `budget` and
    /// nothing executes (`OutOfBudget`), otherwise the rest of the budget runs. So a budget
    /// smaller than a block still makes progress, and budget 0 at a leader with no debt
    /// executes nothing.
    ///
    /// - Resuming a `Done` fiber returns `Exit::Done` again; a `Trapped` fiber returns the same
    ///   `Exit::Trap` again; an unknown or removed id returns `Exit::Trap(Trap::Host("invalid fiber"))`.
    /// - `OutOfBudget` leaves the fiber resumable at a block leader (`fiber_location`). To run
    ///   one instruction, use `step`.
    /// - `yield` returns `Exit::Yield` with the pc after the yield. A host function returning
    ///   `HostOk::Yield` has its results written first, then `Exit::Yield`.
    /// - A host `Err(trap)` traps the fiber; other fibers and the instance stay usable.
    pub fn resume(&mut self, fiber: FiberId, host: &mut H, budget: u64) -> Exit {
        self.resume_inner(fiber, host, budget, false).0
    }

    /// Like `resume`, also returning the budget units consumed by this call: block costs plus
    /// the extra units of bulk memory operations plus debt paid. Never more than `budget` (an
    /// overrun is not counted; it becomes debt), so an embedder can subtract it from a total
    /// budget across several resumes. A resume that executes nothing and pays no debt (budget
    /// 0 at a leader, done, trapped, invalid fiber) reports 0.
    pub fn resume_units(&mut self, fiber: FiberId, host: &mut H, budget: u64) -> (Exit, u64) {
        self.resume_inner(fiber, host, budget, false)
    }

    /// Executes exactly one instruction of a `Ready` or `Suspended` fiber, charging no budget
    /// and paying no debt (for debuggers).
    ///
    /// Returns `Exit::OutOfBudget` when the instruction ran and the fiber is still runnable
    /// (read it as "paused; resumable"; the status is `Ready`), or `Done`, `Yield` or
    /// `Trap` as the instruction produces. A `Done`, `Trapped` or invalid fiber answers as
    /// `resume` does.
    ///
    /// A fiber stepped into the middle of a block and then resumed normally continues from
    /// there without being charged for the rest of that block; it is charged again at the
    /// next leader. Steps always run in the interpreter, and so does the resume of such a
    /// fiber (a backend only runs fibers at leaders).
    pub fn step(&mut self, fiber: FiberId, host: &mut H) -> Exit {
        self.resume_inner(fiber, host, 0, true).0
    }

    /// Returns `(exit, budget units consumed)`.
    fn resume_inner(&mut self, fiber: FiberId, host: &mut H, budget: u64, step: bool) -> (Exit, u64) {
        let Instance {
            module,
            memory,
            globals,
            gmut,
            gmask,
            res_masks,
            infos,
            type_canon,
            hosts,
            limits,
            slots,
            backend,
            ..
        } = self;
        let fib = match slots.get_mut(fiber.index as usize) {
            Some(s) if s.generation == fiber.generation => s.fiber.as_mut(),
            _ => None,
        };
        let Some(fib) = fib else {
            return (Exit::Trap(Trap::Host("invalid fiber".into())), 0);
        };
        match &fib.status {
            FiberStatus::Done => return (Exit::Done, 0),
            FiberStatus::Trapped(t) => return (Exit::Trap(t.clone()), 0),
            _ => {}
        }
        // Pay budget debt from earlier overruns before executing anything.
        let mut budget = budget;
        let mut paid = 0;
        if !step {
            paid = fib.stack.debt;
            if paid > 0 {
                if budget <= paid {
                    fib.stack.debt = paid - budget;
                    return (Exit::OutOfBudget, budget);
                }
                fib.stack.debt = 0;
                budget -= paid;
            }
        }
        let start = budget.min(i64::MAX as u64) as i64;
        let mut cx = RunCx {
            module,
            infos,
            type_canon,
            limits: *limits,
            memory,
            globals,
            fiber,
            host,
            hosts,
            gmut,
            gmask,
            res_masks,
        };
        let (exit, counter) = if step {
            interp::run::<H, true>(&mut cx, &mut fib.stack, 0)
        } else {
            match backend {
                Some(b) if module.is_leader(fib.stack.func, fib.stack.pc) => b.run(&mut cx, &mut fib.stack, start),
                _ => run_interpreter(&mut cx, &mut fib.stack, start),
            }
        };
        // The one place a negative counter (an overrun) becomes debt, for every backend.
        let used = if step {
            0
        } else if counter < 0 {
            fib.stack.debt = fib.stack.debt.saturating_add(counter.unsigned_abs());
            start as u64
        } else {
            (start - counter) as u64
        };
        fib.status = match &exit {
            Exit::Done => FiberStatus::Done,
            Exit::Yield => FiberStatus::Suspended,
            Exit::OutOfBudget => FiberStatus::Ready,
            Exit::Trap(t) => FiberStatus::Trapped(t.clone()),
        };
        if matches!(exit, Exit::Trap(_)) {
            fib.trace = std::mem::take(&mut fib.stack.trace);
            if fib.trace.is_empty() {
                fib.trace.push((fib.stack.func, fib.stack.pc));
            }
        }
        if matches!(exit, Exit::Done | Exit::Trap(_)) {
            // Free the register stack; results stay.
            fib.stack.regs = Vec::new();
            fib.stack.frames = Vec::new();
            fib.stack.scratch = Vec::new();
        }
        (exit, paid + used)
    }

    /// Spawns `export`, resumes it once with `budget`, and removes the fiber again. Returns the
    /// results on `Done`. The fiber slot is freed in every case; for a trap the location and
    /// backtrace are copied into the error first.
    pub fn call(
        &mut self,
        export: &str,
        args: &[u64],
        host: &mut H,
        budget: u64,
    ) -> Result<Vec<u64>, CallError> {
        let id = self.spawn(export, args).map_err(CallError::Spawn)?;
        let exit = self.resume(id, host, budget);
        let out = match exit {
            Exit::Done => Ok(self.results(id).map(<[u64]>::to_vec).unwrap_or_default()),
            Exit::Trap(trap) => Err(CallError::Trap {
                trap,
                location: self.trap_location(id),
                backtrace: self.backtrace(id).map(<[(u32, u32)]>::to_vec).unwrap_or_default(),
            }),
            Exit::OutOfBudget => Err(CallError::OutOfBudget),
            Exit::Yield => Err(CallError::Yield),
        };
        self.remove_fiber(id);
        out
    }

    /// Total capacity (in elements) of the register, frame and scratch stacks a fiber still
    /// holds; 0 once it is `Done` or `Trapped`. For tests and diagnostics.
    #[doc(hidden)]
    pub fn fiber_stack_capacity(&self, fiber: FiberId) -> Option<usize> {
        let f = self.fiber(fiber)?;
        Some(f.stack.regs.capacity() + f.stack.frames.capacity() + f.stack.scratch.capacity())
    }

    /// Results of a `Done` fiber.
    pub fn results(&self, fiber: FiberId) -> Option<&[u64]> {
        let f = self.fiber(fiber)?;
        (f.status == FiberStatus::Done).then_some(&f.stack.results[..])
    }

    /// `(function index, instruction index)` of the instruction that trapped a `Trapped` fiber
    /// (full function index space). A host-function trap reports the calling instruction.
    pub fn trap_location(&self, fiber: FiberId) -> Option<(u32, u32)> {
        self.backtrace(fiber)?.first().copied()
    }

    /// Call stack of a `Trapped` fiber at the moment of the trap, innermost first, as
    /// `(function index, instruction index)`; outer entries are the call instructions.
    pub fn backtrace(&self, fiber: FiberId) -> Option<&[(u32, u32)]> {
        let f = self.fiber(fiber)?;
        matches!(f.status, FiberStatus::Trapped(_)).then_some(&f.trace[..])
    }

    /// The paused stack of a `Ready` or `Suspended` fiber (debugger access).
    fn paused(&self, fiber: FiberId) -> Option<&FiberStack> {
        let f = self.fiber(fiber)?;
        matches!(f.status, FiberStatus::Ready | FiberStatus::Suspended).then_some(&f.stack)
    }

    /// `(function index, instruction index)` of the next instruction a `Ready` or `Suspended`
    /// fiber will execute; `None` for `Done`, `Trapped` or invalid fibers. After `OutOfBudget`
    /// or a `yield` it is a block leader or the instruction after a `yield`/`call`; after
    /// `step` it can be anywhere. `step` executes exactly that instruction.
    pub fn fiber_location(&self, fiber: FiberId) -> Option<(u32, u32)> {
        self.paused(fiber).map(|s| (s.func, s.pc))
    }

    /// The registers of the current (innermost) frame of a `Ready` or `Suspended` fiber,
    /// `r0` first.
    pub fn fiber_registers(&self, fiber: FiberId) -> Option<&[u64]> {
        let s = self.paused(fiber)?;
        s.regs.get(s.fp as usize..)
    }

    /// The call stack of a `Ready` or `Suspended` fiber, innermost first: the next
    /// instruction (as `fiber_location`), then for each caller the `call` instruction it is
    /// waiting in. Empty for `Done`, `Trapped` or invalid fibers (see `backtrace` for those).
    pub fn fiber_backtrace(&self, fiber: FiberId) -> Vec<(u32, u32)> {
        let Some(s) = self.paused(fiber) else { return Vec::new() };
        let mut v = Vec::with_capacity(s.frames.len() + 1);
        v.push((s.func, s.pc));
        v.extend(s.frames.iter().rev().map(|fr| fr.call_site()));
        v
    }

    pub fn status(&self, fiber: FiberId) -> Option<FiberStatus> {
        self.fiber(fiber).map(|f| f.status.clone())
    }

    /// Frees the fiber's slot; the id (and copies of it) become invalid. Done and trapped fibers
    /// count towards `max_fibers` until removed.
    pub fn remove_fiber(&mut self, fiber: FiberId) -> bool {
        if self.fiber(fiber).is_none() {
            return false;
        }
        let slot = &mut self.slots[fiber.index as usize];
        slot.fiber = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(fiber.index);
        self.live -= 1;
        true
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }

    pub fn global(&self, idx: u32) -> Option<u64> {
        self.globals.get(idx as usize).copied()
    }

    /// Sets any global (embedder access ignores the mutable flag); false on a bad index. The
    /// upper 32 bits of the value are cleared when the global is `i32` or `f32`.
    pub fn set_global(&mut self, idx: u32, v: u64) -> bool {
        let mask = self.gmask.get(idx as usize).copied().unwrap_or(u64::MAX);
        match self.globals.get_mut(idx as usize) {
            Some(g) => {
                *g = v & mask;
                true
            }
            None => false,
        }
    }

    pub fn export_global(&self, name: &str) -> Option<u64> {
        self.global(self.module.export_global(name)?)
    }

    pub fn module(&self) -> &Module {
        &self.module
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{FunctionBuilder, ModuleBuilder};
    use crate::api::HostRegistry;
    use crate::imports::HostOk;
    use crate::op::Op;
    use crate::types::ValType::{F32, F64, I32, I64};

    fn ty(p: &[crate::ValType], r: &[crate::ValType]) -> FuncType {
        FuncType::new(p.iter().copied(), r.iter().copied())
    }

    fn inst(m: crate::Module) -> Instance<()> {
        Instance::new(&m, &Imports::new(), Limits::default()).unwrap()
    }

    fn run_to_end(i: &mut Instance<()>, name: &str, args: &[u64]) -> Result<Vec<u64>, Trap> {
        let f = i.spawn(name, args).unwrap();
        match i.resume(f, &mut (), u64::MAX) {
            Exit::Done => Ok(i.results(f).unwrap().to_vec()),
            Exit::Trap(t) => Err(t),
            e => panic!("unexpected {e:?}"),
        }
    }

    /// One-function module `main(params) -> results` with the given body.
    fn single(
        p: &[crate::ValType],
        r: &[crate::ValType],
        nregs: u16,
        body: impl FnOnce(&mut FunctionBuilder),
    ) -> Instance<()> {
        let mut mb = ModuleBuilder::new();
        let mut fb = FunctionBuilder::new(nregs);
        body(&mut fb);
        let f = mb.add_func(ty(p, r), fb);
        mb.export_func("main", f);
        mb.memory(1, Some(2));
        inst(mb.finish().unwrap())
    }

    #[test]
    fn factorial() {
        let mut mb = ModuleBuilder::new();
        let f = mb.declare_func(ty(&[I64], &[I64]));
        let mut fb = FunctionBuilder::new(5);
        let l = fb.new_label();
        fb.const_i64(1, 1);
        fb.op3(Op::I64LeS, 2, 0, 1);
        fb.br_ifz(2, l);
        fb.ret(1);
        fb.bind(l);
        fb.op3(Op::I64Sub, 3, 0, 1);
        fb.call(f, 3);
        fb.op3(Op::I64Mul, 4, 0, 3);
        fb.ret(4);
        mb.define(f, fb);
        mb.export_func("fact", f);
        let mut i = inst(mb.finish().unwrap());
        assert_eq!(run_to_end(&mut i, "fact", &[10]).unwrap(), vec![3628800]);
        assert_eq!(run_to_end(&mut i, "fact", &[0]).unwrap(), vec![1]);
        assert_eq!(run_to_end(&mut i, "fact", &[20]).unwrap(), vec![2432902008176640000]);
    }

    #[test]
    fn counting_loop_budget() {
        let mut i = single(&[I32], &[I32], 4, |fb| {
            let l = fb.new_label();
            fb.const_i32(1, 0);
            fb.const_i32(2, 1);
            fb.bind(l);
            fb.op3(Op::I32Add, 1, 1, 2);
            fb.op3(Op::I32LtU, 3, 1, 0);
            fb.br_if(3, l);
            fb.ret(1);
        });
        // Blocks: [0,1] (cost 2), loop body [2,3,4] (cost 3, br_if ends it), [5] ret (cost 1).
        let f = i.spawn("main", &[100]).unwrap();
        assert_eq!(i.resume_units(f, &mut (), 0), (Exit::OutOfBudget, 0));
        let mut total = 0;
        let mut calls = 0;
        loop {
            let (e, n) = i.resume_units(f, &mut (), 7);
            total += n;
            calls += 1;
            match e {
                Exit::OutOfBudget => assert!(n <= 7),
                Exit::Done => break,
                e => panic!("{e:?}"),
            }
        }
        // Every unit is paid exactly once, whether as a block cost or as debt.
        assert_eq!(total, 2 + 100 * 3 + 1);
        assert!(calls > 40);
        assert_eq!(i.results(f).unwrap(), &[100]);
        assert_eq!(i.status(f), Some(FiberStatus::Done));
        assert_eq!(i.resume(f, &mut (), 5), Exit::Done);
    }

    #[test]
    fn host_call_writes_memory() {
        let mut mb = ModuleBuilder::new();
        let h = mb.import("env", "fill", ty(&[I64, I32], &[I32]));
        let mut fb = FunctionBuilder::new(4);
        fb.const_i64(0, 100);
        fb.const_i32(1, 4);
        fb.call(h, 0);
        fb.ret(0);
        let f = mb.add_func(ty(&[], &[I32]), fb);
        mb.export_func("main", f);
        mb.memory(1, None);
        let m = mb.finish().unwrap();
        let mut imports = HostRegistry::<u32>::new();
        imports.set_raw("env", "fill", ty(&[I64, I32], &[I32]), |cx, args, res| {
            *cx.host() += 1;
            cx.write(args[0], &vec![7u8; args[1] as usize])?;
            res[0] = 99;
            Ok(HostOk::Continue)
        });
        let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
        let mut host = 0u32;
        let fid = i.spawn("main", &[]).unwrap();
        assert_eq!(i.resume(fid, &mut host, 100), Exit::Done);
        assert_eq!(i.results(fid).unwrap(), &[99]);
        assert_eq!(host, 1);
        assert_eq!(&i.memory().as_slice()[99..105], &[0, 7, 7, 7, 7, 0]);
    }

    #[test]
    fn host_yield_and_trap_and_link_errors() {
        let mut mb = ModuleBuilder::new();
        let h = mb.import("env", "y", ty(&[], &[I32]));
        let t = mb.import("env", "t", ty(&[], &[]));
        let mut fb = FunctionBuilder::new(2);
        fb.call(h, 0);
        fb.const_i32(1, 5);
        fb.op3(Op::I32Add, 0, 0, 1);
        fb.call(t, 1);
        fb.ret(0);
        let f = mb.add_func(ty(&[], &[I32]), fb);
        mb.export_func("main", f);
        let m = mb.finish().unwrap();
        let mut imports = HostRegistry::<()>::new();
        imports.set_raw("env", "y", ty(&[], &[I32]), |_, _, r| {
            r[0] = 37;
            Ok(HostOk::Yield)
        });
        assert!(matches!(
            Instance::new(&m, &imports.imports(), Limits::default()),
            Err(LoadError::Link(LinkError::MissingImport { .. }))
        ));
        imports.set_raw("env", "t", ty(&[I32], &[]), |_, _, _| Ok(HostOk::Continue));
        assert!(matches!(
            Instance::new(&m, &imports.imports(), Limits::default()),
            Err(LoadError::Link(LinkError::ImportTypeMismatch { .. }))
        ));
        imports.set_raw("env", "t", ty(&[], &[]), |_, _, _| Err(Trap::Host("boom".into())));
        let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
        let fid = i.spawn("main", &[]).unwrap();
        assert_eq!(i.resume(fid, &mut (), 100), Exit::Yield);
        assert_eq!(i.status(fid), Some(FiberStatus::Suspended));
        let e = i.resume(fid, &mut (), 100);
        assert_eq!(e, Exit::Trap(Trap::Host("boom".into())));
        assert_eq!(i.status(fid), Some(FiberStatus::Trapped(Trap::Host("boom".into()))));
        assert_eq!(i.resume(fid, &mut (), 100), e);
        assert_eq!(i.results(fid), None);
        // Instance still usable.
        let g = i.spawn("main", &[]).unwrap();
        assert_eq!(i.resume(g, &mut (), 100), Exit::Yield);
    }

    #[test]
    fn fibers_interleave() {
        // main(slot): loop 3 times { mem[slot*4] += 1; yield }
        let mut mb = ModuleBuilder::new();
        let mut fb = FunctionBuilder::new(8);
        let l = fb.new_label();
        fb.const_i32(1, 4);
        fb.op3(Op::I32Mul, 0, 0, 1); // addr
        fb.const_i32(2, 3); // counter
        fb.const_i32(3, 1);
        fb.const_i32(5, 100); // shared counter address
        fb.bind(l);
        fb.load(Op::I32Load, 4, 5, 0);
        fb.op3(Op::I32Add, 4, 4, 3);
        fb.store(Op::I32Store, 4, 5, 0);
        fb.store(Op::I32Store, 4, 0, 0);
        fb.yield_();
        fb.op3(Op::I32Sub, 2, 2, 3);
        fb.br_if(2, l);
        fb.ret(2);
        let f = mb.add_func(ty(&[I32], &[I32]), fb);
        mb.export_func("main", f);
        mb.memory(1, None);
        let mut i = inst(mb.finish().unwrap());
        let a = i.spawn("main", &[0]).unwrap();
        let b = i.spawn("main", &[1]).unwrap();
        assert_ne!(a, b);
        assert_eq!(i.resume(a, &mut (), 1000), Exit::Yield);
        assert_eq!(i.resume(b, &mut (), 1000), Exit::Yield);
        assert_eq!(i.memory().load::<4>(0, 0).unwrap(), 1u32.to_le_bytes());
        assert_eq!(i.memory().load::<4>(0, 4).unwrap(), 2u32.to_le_bytes());
        assert_eq!(i.resume(a, &mut (), 1000), Exit::Yield);
        assert_eq!(i.memory().load::<4>(0, 0).unwrap(), 3u32.to_le_bytes());
        assert_eq!(i.resume(a, &mut (), 1000), Exit::Yield);
        assert_eq!(i.resume(a, &mut (), 1000), Exit::Done);
        assert_eq!(i.resume(b, &mut (), 1000), Exit::Yield);
        assert_eq!(i.resume(b, &mut (), 1000), Exit::Yield);
        assert_eq!(i.resume(b, &mut (), 1000), Exit::Done);
        assert!(i.remove_fiber(a));
        assert!(!i.remove_fiber(a));
        assert_eq!(i.status(a), None);
        assert_eq!(i.resume(a, &mut (), 1), Exit::Trap(Trap::Host("invalid fiber".into())));
        let c = i.spawn("main", &[0]).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn call_indirect_checks() {
        let mut mb = ModuleBuilder::new();
        let t_i = mb.add_type(ty(&[I32], &[I32]));
        let mut fb = FunctionBuilder::new(1);
        fb.ret(0);
        let id = mb.add_func(ty(&[I32], &[I32]), fb);
        let mut fb = FunctionBuilder::new(0);
        fb.ret(0);
        let other = mb.add_func(ty(&[], &[]), fb);
        mb.table(vec![Some(id), None, Some(other)]);
        let mut fb = FunctionBuilder::new(3);
        fb.const_i32(1, 42);
        fb.call_indirect(t_i, 0, 1);
        fb.ret(1);
        let m = mb.add_func(ty(&[I32], &[I32]), fb);
        mb.export_func("main", m);
        let mut i = inst(mb.finish().unwrap());
        assert_eq!(run_to_end(&mut i, "main", &[0]).unwrap(), vec![42]);
        assert_eq!(run_to_end(&mut i, "main", &[1]), Err(Trap::UndefinedElement));
        assert_eq!(run_to_end(&mut i, "main", &[2]), Err(Trap::IndirectCallTypeMismatch));
        assert_eq!(run_to_end(&mut i, "main", &[3]), Err(Trap::UndefinedElement));
        assert_eq!(run_to_end(&mut i, "main", &[u32::MAX as u64]), Err(Trap::UndefinedElement));
    }

    #[test]
    fn stack_overflow() {
        let mut mb = ModuleBuilder::new();
        let f = mb.declare_func(ty(&[], &[]));
        let mut fb = FunctionBuilder::new(1);
        fb.call(f, 0);
        fb.ret(0);
        mb.define(f, fb);
        mb.export_func("main", f);
        let m = mb.finish().unwrap();
        let mut i = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
        assert_eq!(run_to_end(&mut i, "main", &[]), Err(Trap::StackOverflow));
        let lim = Limits { max_registers: 10, max_call_depth: 1_000_000, ..Limits::default() };
        let mut i = Instance::new(&m, &Imports::<()>::new(), lim).unwrap();
        assert_eq!(run_to_end(&mut i, "main", &[]), Err(Trap::StackOverflow));
        let lim = Limits { max_registers: 0, ..Limits::default() };
        let mut i = Instance::new(&m, &Imports::<()>::new(), lim).unwrap();
        assert_eq!(i.spawn("main", &[]), Err(SpawnError::StackOverflow));
    }

    #[test]
    fn spawn_errors_and_globals() {
        let mut mb = ModuleBuilder::new();
        let h = mb.import("env", "h", ty(&[], &[]));
        let g = mb.global(I64, true, 5);
        mb.export_global("g", g);
        mb.export_func("h", h);
        let mut fb = FunctionBuilder::new(1);
        fb.global_get(0, g);
        fb.const_i64(0, 9);
        fb.global_set(g, 0);
        fb.ret(0);
        let f = mb.add_func(ty(&[I32], &[]), fb);
        mb.export_func("main", f);
        let m = mb.finish().unwrap();
        let mut imports = HostRegistry::<()>::new();
        imports.set_raw("env", "h", ty(&[], &[]), |_, _, _| Ok(HostOk::Continue));
        let limits = Limits { max_fibers: 1, ..Limits::default() };
        let mut i = Instance::new(&m, &imports.imports(), limits).unwrap();
        assert_eq!(i.spawn("nope", &[]), Err(SpawnError::NoSuchExport));
        assert_eq!(i.spawn("g", &[]), Err(SpawnError::NotAFunction));
        assert_eq!(i.spawn("h", &[]), Err(SpawnError::ImportEntry));
        assert_eq!(i.spawn("main", &[]), Err(SpawnError::ArgCount { expected: 1, got: 0 }));
        let a = i.spawn("main", &[0]).unwrap();
        assert_eq!(i.spawn("main", &[0]), Err(SpawnError::TooManyFibers));
        assert_eq!(i.export_global("g"), Some(5));
        assert_eq!(i.resume(a, &mut (), 100), Exit::Done);
        assert_eq!(i.export_global("g"), Some(9));
        assert!(i.set_global(0, 1) && !i.set_global(1, 1));
        assert!(i.remove_fiber(a));
        assert!(i.spawn("main", &[0]).is_ok());
        let big = Limits { max_memory_pages: 0, ..Limits::default() };
        let mut mb = ModuleBuilder::new();
        mb.memory(1, None);
        let m2 = mb.finish().unwrap();
        assert!(matches!(
            Instance::new(&m2, &Imports::<()>::new(), big),
            Err(LoadError::Link(LinkError::MemoryTooLarge { .. }))
        ));
    }

    #[test]
    fn i32_zero_extends_and_select() {
        let mut i = single(&[], &[I64, I64, I64, I64], 8, |fb| {
            fb.const_i32(0, 0);
            fb.const_i32(1, 1);
            fb.op3(Op::I32Sub, 2, 0, 1); // 0xFFFFFFFF
            fb.const_i32(3, -8);
            fb.op2(Op::I32Extend8S, 4, 3);
            fb.const_i64(5, 0x1_0000_0001);
            fb.select(3, 5, 2, 1); // cond low 32 bits = 1 -> r2
            fb.op3(Op::I32Mul, 6, 2, 2);
            fb.ret(2);
        });
        let r = run_to_end(&mut i, "main", &[]).unwrap();
        assert_eq!(r[0], 0xFFFF_FFFF);
        assert_eq!(r[1], 0xFFFF_FFFF);
        assert_eq!(r[2], 0xFFFF_FFF8);
        assert_eq!(r[3], 0x1_0000_0001);
    }

    #[test]
    fn division_and_traps() {
        let mut i = single(&[I32, I32], &[I32], 3, |fb| {
            fb.op3(Op::I32DivS, 2, 0, 1);
            fb.ret(2);
        });
        let min = i32::MIN as u32 as u64;
        assert_eq!(run_to_end(&mut i, "main", &[7, 0]), Err(Trap::DivideByZero));
        assert_eq!(run_to_end(&mut i, "main", &[min, u32::MAX as u64]), Err(Trap::IntegerOverflow));
        assert_eq!(run_to_end(&mut i, "main", &[(-7i32) as u32 as u64, 2]).unwrap(), vec![(-3i32) as u32 as u64]);
        let mut i = single(&[I32, I32], &[I32], 3, |fb| {
            fb.op3(Op::I32RemS, 2, 0, 1);
            fb.ret(2);
        });
        assert_eq!(run_to_end(&mut i, "main", &[min, u32::MAX as u64]).unwrap(), vec![0]);
    }

    fn trunc(op: Op, bits: u64) -> Result<u64, Trap> {
        let mut i = single(&[I64], &[I64], 2, |fb| {
            fb.op2(op, 1, 0);
            fb.ret(1);
        });
        run_to_end(&mut i, "main", &[bits]).map(|v| v[0])
    }

    #[test]
    fn trunc_boundaries() {
        let f32b = |x: f32| x.to_bits() as u64;
        let f64b = |x: f64| x.to_bits();
        assert_eq!(trunc(Op::I32TruncF32S, f32b(2147483520.0)), Ok(2147483520));
        assert_eq!(trunc(Op::I32TruncF32S, f32b(2147483648.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I32TruncF32S, f32b(-2147483648.0)), Ok(0x8000_0000));
        assert_eq!(trunc(Op::I32TruncF32S, f32b(-2147483904.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I32TruncF32S, f32b(f32::NAN)), Err(Trap::InvalidConversion));
        assert_eq!(trunc(Op::I32TruncF64S, f64b(-2147483648.9)), Ok(0x8000_0000));
        assert_eq!(trunc(Op::I32TruncF64S, f64b(-2147483649.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I32TruncF64U, f64b(4294967295.9)), Ok(0xFFFF_FFFF));
        assert_eq!(trunc(Op::I32TruncF64U, f64b(4294967296.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I32TruncF64U, f64b(-0.9)), Ok(0));
        assert_eq!(trunc(Op::I32TruncF64U, f64b(-1.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I64TruncF64S, f64b(9223372036854775808.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I64TruncF64S, f64b(-9223372036854775808.0)), Ok(1 << 63));
        assert_eq!(trunc(Op::I64TruncF64U, f64b(18446744073709551616.0)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I64TruncF32U, f32b(f32::INFINITY)), Err(Trap::IntegerOverflow));
        assert_eq!(trunc(Op::I32TruncSatF32S, f32b(f32::NAN)), Ok(0));
        assert_eq!(trunc(Op::I32TruncSatF32S, f32b(-1e20)), Ok(0x8000_0000));
        assert_eq!(trunc(Op::I32TruncSatF64U, f64b(1e20)), Ok(0xFFFF_FFFF));
        assert_eq!(trunc(Op::I64TruncSatF64S, f64b(1e30)), Ok(i64::MAX as u64));
        let _ = (F32, F64);
    }

    #[test]
    fn memory_ops_in_code() {
        let mut i = single(&[], &[I64, I64, I64], 8, |fb| {
            fb.const_i64(0, 65535);
            fb.const_i32(1, 0x1234_5678);
            fb.store(Op::I32Store8, 1, 0, 0);
            fb.load(Op::I64Load8U, 2, 0, 0);
            fb.memory_size(3);
            fb.const_i64(4, 1);
            fb.memory_grow(4, 4); // old size 1
            fb.const_i64(5, 5);
            fb.memory_grow(5, 5); // fails: -1
            fb.load(Op::I32Load, 6, 0, 0); // 65535..65539 now valid after grow
            fb.ret(3);
        });
        let r = run_to_end(&mut i, "main", &[]).unwrap();
        assert_eq!(r, vec![1, 1, u64::MAX]);
        let mut i = single(&[], &[I32], 3, |fb| {
            fb.const_i64(0, 65533);
            fb.load(Op::I32Load, 1, 0, 0);
            fb.ret(1);
        });
        assert_eq!(run_to_end(&mut i, "main", &[]), Err(Trap::OutOfBounds));
    }

    #[test]
    #[ignore]
    fn bench_counting_loop() {
        let mut i = single(&[I32], &[I32], 4, |fb| {
            let l = fb.new_label();
            fb.const_i32(1, 0);
            fb.const_i32(2, 1);
            fb.bind(l);
            fb.op3(Op::I32Add, 1, 1, 2);
            fb.op3(Op::I32LtU, 3, 1, 0);
            fb.br_if(3, l);
            fb.ret(1);
        });
        let f = i.spawn("main", &[33_333_333]).unwrap();
        let t = std::time::Instant::now();
        let (e, n) = i.resume_units(f, &mut (), u64::MAX);
        let dt = t.elapsed();
        assert_eq!(e, Exit::Done);
        println!("{n} instructions, {:.3} ns/instruction", dt.as_nanos() as f64 / n as f64);
    }
}
