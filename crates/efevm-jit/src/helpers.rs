//! `extern "C"` helpers called from generated code. They are the only place generated code
//! reaches back into Rust: trap and suspension records, the resume cursor, memory growth, bulk
//! memory operations and host calls.
//!
//! SAFETY (all helpers): generated code passes the `JitCtx` pointer it received from
//! `JitBackend::run`, whose `rt` field points at the live `Rt` of that run; both outlive the
//! native call and are not otherwise accessed while native code runs.

use std::slice;

use efevm::backend::RunCx;
use efevm::imports::HostOk;
use efevm::{PAGE_SIZE, Trap};

use crate::ctx::*;

/// Bytes per extra budget unit for bulk operations (the interpreter's `BULK_UNIT_BYTES`).
const BULK_UNIT_BYTES: u64 = 256;

unsafe fn rt<'a>(ctx: *mut JitCtx) -> &'a mut Rt {
    // SAFETY: see module docs.
    unsafe { &mut *((*ctx).rt as *mut Rt) }
}

fn size_of_func(rt: &Rt, func: u32) -> usize {
    // SAFETY: `infos` points at the instance's info slice, alive for the whole run.
    let infos = unsafe { &*rt.infos };
    infos.get(func as usize).map_or(0, |i| (i.nregs as usize).max(i.np as usize))
}

/// Refreshes `mem_base` / `mem_len` from the instance memory.
unsafe fn refresh_mem(ctx: *mut JitCtx, rt: &mut Rt) {
    // SAFETY: `get_mem` yields the memory of the `RunCx` of this run.
    let mem = unsafe { &mut *(rt.get_mem)(rt.cx) };
    let len = mem.len();
    let slice = mem.as_mut_slice();
    // SAFETY: ctx is valid (module docs).
    unsafe {
        (*ctx).mem_base = slice.as_mut_ptr();
        (*ctx).mem_len = len;
    }
}

fn charge(ctx: *mut JitCtx, units: u64) {
    // SAFETY: ctx is valid (module docs).
    unsafe {
        (*ctx).budget = (*ctx).budget.saturating_sub(units.min(i64::MAX as u64) as i64);
    }
}

/// A fresh trap: records the kind and the first (innermost) trace entry.
pub unsafe extern "C" fn h_trap(ctx: *mut JitCtx, kind: u32, func: u32, inst: u32) {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    rt.pending = Some(trap_of_kind(kind));
    rt.trace.push((func, inst));
    // SAFETY: module docs.
    unsafe {
        (*ctx).trap_kind = kind;
        (*ctx).trap_func = func;
        (*ctx).trap_inst = inst;
    }
}

/// A caller on the way out of a trap appends its call instruction.
pub unsafe extern "C" fn h_trap_frame(ctx: *mut JitCtx, func: u32, inst: u32) {
    // SAFETY: module docs.
    unsafe { rt(ctx) }.trace.push((func, inst));
}

fn push_saved(rt: &mut Rt, func: u32, pc: u32, base: u32, regs: *const u64, size: u32) {
    let start = rt.saved_regs.len();
    // SAFETY: generated code passes a stack slot of `size` u64s.
    rt.saved_regs.extend_from_slice(unsafe { slice::from_raw_parts(regs, size as usize) });
    rt.saved.push(Saved { func, pc, base, start, len: size as usize });
}

/// The innermost activation of a suspension.
pub unsafe extern "C" fn h_save_inner(ctx: *mut JitCtx, kind: u32, func: u32, pc: u32, regs: *const u64, size: u32) {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    push_saved(rt, func, pc, 0, regs, size);
    // SAFETY: module docs.
    unsafe { (*ctx).suspend_kind = kind };
}

/// A caller activation of a suspension: `pc` is the return pc, `base` the call window base.
pub unsafe extern "C" fn h_save_frame(ctx: *mut JitCtx, func: u32, pc: u32, regs: *const u64, size: u32, base: u32) {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    push_saved(rt, func, pc, base, regs, size);
}

/// The native stack is nearly used up at the entry of `func`. In normal mode (0) the callee's
/// fresh activation (parameters from `window`, other registers zero) is saved as the
/// innermost one; in resume mode (1) the activation is still in the `FiberStack`.
pub unsafe extern "C" fn h_stack_low(ctx: *mut JitCtx, func: u32, window: *const u64, mode: u32) {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    if mode == 0 {
        // SAFETY: `infos` is alive for the run.
        let np = unsafe { &*rt.infos }.get(func as usize).map_or(0, |i| i.np as usize);
        let size = size_of_func(rt, func);
        let start = rt.saved_regs.len();
        // SAFETY: the caller stored `np` arguments in its window slot.
        rt.saved_regs.extend_from_slice(unsafe { slice::from_raw_parts(window, np) });
        rt.saved_regs.resize(start + size, 0);
        rt.saved.push(Saved { func, pc: 0, base: 0, start, len: size });
    } else {
        rt.keep = true;
    }
    // SAFETY: module docs.
    unsafe { (*ctx).suspend_kind = SUSPEND_NATIVE_STACK };
}

/// Resume mode: copies the next activation's saved registers into `slot` and returns its pc.
/// `ctx.resume_inner` tells whether it is the innermost activation. A mismatch with the
/// expected function yields pc `u32::MAX`, which the generated dispatch turns into a trap.
pub unsafe extern "C" fn h_resume_frame(ctx: *mut JitCtx, func: u32, slot: *mut u64) -> u32 {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    // SAFETY: `stack` is the run's fiber stack, not otherwise accessed during the native call.
    let st = unsafe { &*rt.stack };
    let k = rt.cursor;
    rt.cursor += 1;
    let (f, pc, fp, inner) = match st.frames.get(k) {
        Some(fr) => (fr.func, fr.pc, fr.fp, false),
        None => (st.func, st.pc, st.fp, true),
    };
    // SAFETY: module docs.
    unsafe { (*ctx).resume_inner = inner as u32 };
    let size = size_of_func(rt, func);
    let fp = fp as usize;
    match st.regs.get(fp..fp + size) {
        Some(src) if f == func => {
            // SAFETY: the slot has `size` u64s.
            unsafe { slice::from_raw_parts_mut(slot, size) }.copy_from_slice(src);
            pc
        }
        _ => u32::MAX,
    }
}

/// `memory.grow`: returns the old size in pages or `u64::MAX`; charges like the interpreter.
pub unsafe extern "C" fn h_grow(ctx: *mut JitCtx, delta: u64) -> u64 {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    // SAFETY: memory of this run's `RunCx`.
    let mem = unsafe { &mut *(rt.get_mem)(rt.cx) };
    let r = mem.grow(delta).unwrap_or(u64::MAX);
    // SAFETY: refresh after a possible move.
    unsafe { refresh_mem(ctx, rt) };
    charge(ctx, delta.saturating_mul(PAGE_SIZE / BULK_UNIT_BYTES));
    r
}

/// `memory.copy`: 0 ok, 1 out of bounds.
pub unsafe extern "C" fn h_copy(ctx: *mut JitCtx, dst: u64, src: u64, len: u64) -> u32 {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    // SAFETY: memory of this run's `RunCx`.
    let mem = unsafe { &mut *(rt.get_mem)(rt.cx) };
    match mem.copy(dst, src, len) {
        Ok(()) => {
            charge(ctx, len / BULK_UNIT_BYTES);
            0
        }
        Err(_) => 1,
    }
}

/// `memory.fill` (`value` is the whole register; the low byte is used): 0 ok, 1 out of bounds.
pub unsafe extern "C" fn h_fill(ctx: *mut JitCtx, dst: u64, value: u64, len: u64) -> u32 {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    // SAFETY: memory of this run's `RunCx`.
    let mem = unsafe { &mut *(rt.get_mem)(rt.cx) };
    match mem.fill(dst, value as u8, len) {
        Ok(()) => {
            charge(ctx, len / BULK_UNIT_BYTES);
            0
        }
        Err(_) => 1,
    }
}

/// Host call: 0 continue, 3 host yield (results written), 2 trap (pending trap recorded).
pub unsafe extern "C" fn h_host_call<H: 'static>(ctx: *mut JitCtx, import: u32, window: *mut u64) -> i32 {
    // SAFETY: module docs.
    let rt = unsafe { rt(ctx) };
    // SAFETY: `cx` is the `RunCx<H>` of this run (set by `JitBackend::run`).
    let cx = unsafe { &mut *(rt.cx as *mut RunCx<'_, H>) };
    // SAFETY: `infos` is alive for the run.
    let (np, nr) = unsafe { &*rt.infos }.get(import as usize).map_or((0, 0), |i| (i.np as usize, i.nr as usize));
    let mut results = std::mem::take(&mut rt.scratch);
    results.clear();
    results.resize(nr, 0);
    // SAFETY: the caller's window slot holds `np` arguments.
    let args = unsafe { slice::from_raw_parts(window, np) };
    let r = cx.call_host(import, args, &mut results);
    // The host may have grown memory.
    // SAFETY: refresh from the instance memory.
    unsafe { refresh_mem(ctx, rt) };
    let status = match r {
        Ok(k) => {
            // SAFETY: the window slot holds `max(np, nr)` u64s.
            unsafe { slice::from_raw_parts_mut(window, nr) }.copy_from_slice(&results);
            if k == HostOk::Yield { 3 } else { 0 }
        }
        Err(t) => {
            rt.pending = Some(t);
            2
        }
    };
    rt.scratch = results;
    status
}

/// Used by tests of the trap mapping.
#[allow(dead_code)]
pub fn trap_for(kind: u32) -> Trap {
    trap_of_kind(kind)
}
