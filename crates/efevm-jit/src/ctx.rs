//! The context structure shared between generated code and the Rust helpers.

use std::ffi::c_void;

use efevm::Trap;
use efevm::backend::{FiberStack, FuncInfo};
use efevm::memory::Memory;

/// One entry of the indirect-call table.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TableEntry {
    /// Native entry point, null for an unset slot.
    pub code: *const u8,
    /// Canonical type id (equal for equal function types).
    pub canon: u32,
    /// Activation size `max(nregs, nparams)` of the target (0 for imports).
    pub size: u32,
    /// 1 when the target is an import (its code is the import thunk).
    pub is_host: u32,
    pub _pad: u32,
}

/// Why a native run stopped with status 1.
pub const SUSPEND_YIELD: u32 = 0;
pub const SUSPEND_BUDGET: u32 = 1;
pub const SUSPEND_NATIVE_STACK: u32 = 2;

/// Trap kind codes written by generated code (`h_trap`).
pub const TRAP_OOB: u32 = 0;
pub const TRAP_DIV0: u32 = 1;
pub const TRAP_OVERFLOW: u32 = 2;
pub const TRAP_INVALID_CONV: u32 = 3;
pub const TRAP_UNREACHABLE: u32 = 4;
pub const TRAP_STACK_OVERFLOW: u32 = 5;
pub const TRAP_UNDEF_ELEM: u32 = 6;
pub const TRAP_TYPE_MISMATCH: u32 = 7;

pub fn trap_of_kind(kind: u32) -> Trap {
    match kind {
        TRAP_OOB => Trap::OutOfBounds,
        TRAP_DIV0 => Trap::DivideByZero,
        TRAP_OVERFLOW => Trap::IntegerOverflow,
        TRAP_INVALID_CONV => Trap::InvalidConversion,
        TRAP_STACK_OVERFLOW => Trap::StackOverflow,
        TRAP_UNDEF_ELEM => Trap::UndefinedElement,
        TRAP_TYPE_MISMATCH => Trap::IndirectCallTypeMismatch,
        _ => Trap::Unreachable,
    }
}

/// The context pointer every generated function receives. `#[repr(C)]`: generated code reads
/// and writes fields at the offsets given by `offset_of!`.
#[repr(C)]
pub struct JitCtx {
    pub mem_base: *mut u8,
    pub mem_len: u64,
    pub globals: *mut u64,
    /// The signed budget counter.
    pub budget: i64,
    /// Live activations (the interpreter's `frames.len() + 1`).
    pub depth: u32,
    pub max_depth: u32,
    /// Sum of `max(nregs, np)` over live activations.
    pub regs_used: u64,
    pub max_regs: u64,
    /// Native stack pointer limit.
    pub stack_limit: usize,
    pub table: *const TableEntry,
    pub table_len: u64,
    /// `extern "C" fn(*mut JitCtx, u32, *mut u64) -> i32`, monomorphized per host state type.
    pub host_call: usize,
    /// `*mut Rt`.
    pub rt: *mut c_void,
    /// Set by `h_resume_frame`: 1 when the resumed activation is the innermost.
    pub resume_inner: u32,
    /// `SUSPEND_*` of the last suspension.
    pub suspend_kind: u32,
    pub trap_kind: u32,
    pub trap_func: u32,
    pub trap_inst: u32,
}

/// A native frame's activation, saved during a suspension (innermost first in `Rt::saved`).
pub struct Saved {
    pub func: u32,
    pub pc: u32,
    pub base: u32,
    pub start: usize,
    pub len: usize,
}

type MemFn = unsafe fn(*mut c_void) -> *mut Memory;
type GlobFn = unsafe fn(*mut c_void) -> *mut u64;

/// Rust-side state of one native run.
pub struct Rt {
    /// `*mut RunCx<'_, H>`.
    pub cx: *mut c_void,
    pub get_mem: MemFn,
    pub get_globals: GlobFn,
    pub stack: *mut FiberStack,
    pub infos: *const [FuncInfo],
    pub saved: Vec<Saved>,
    pub saved_regs: Vec<u64>,
    /// Next activation of the old `FiberStack` to resume (outermost first).
    pub cursor: usize,
    /// A native-stack suspension happened while resuming: activations from `saved.len()` on
    /// are untouched in the `FiberStack`.
    pub keep: bool,
    pub trace: Vec<(u32, u32)>,
    pub pending: Option<Trap>,
    pub scratch: Vec<u64>,
}

impl JitCtx {
    pub const OFF_MEM_BASE: i32 = std::mem::offset_of!(JitCtx, mem_base) as i32;
    pub const OFF_MEM_LEN: i32 = std::mem::offset_of!(JitCtx, mem_len) as i32;
    pub const OFF_GLOBALS: i32 = std::mem::offset_of!(JitCtx, globals) as i32;
    pub const OFF_BUDGET: i32 = std::mem::offset_of!(JitCtx, budget) as i32;
    pub const OFF_DEPTH: i32 = std::mem::offset_of!(JitCtx, depth) as i32;
    pub const OFF_MAX_DEPTH: i32 = std::mem::offset_of!(JitCtx, max_depth) as i32;
    pub const OFF_REGS_USED: i32 = std::mem::offset_of!(JitCtx, regs_used) as i32;
    pub const OFF_MAX_REGS: i32 = std::mem::offset_of!(JitCtx, max_regs) as i32;
    pub const OFF_STACK_LIMIT: i32 = std::mem::offset_of!(JitCtx, stack_limit) as i32;
    pub const OFF_TABLE: i32 = std::mem::offset_of!(JitCtx, table) as i32;
    pub const OFF_TABLE_LEN: i32 = std::mem::offset_of!(JitCtx, table_len) as i32;
    pub const OFF_HOST_CALL: i32 = std::mem::offset_of!(JitCtx, host_call) as i32;
    pub const OFF_RESUME_INNER: i32 = std::mem::offset_of!(JitCtx, resume_inner) as i32;
}

impl TableEntry {
    pub const OFF_CODE: i32 = std::mem::offset_of!(TableEntry, code) as i32;
    pub const OFF_CANON: i32 = std::mem::offset_of!(TableEntry, canon) as i32;
    pub const OFF_SIZE: i32 = std::mem::offset_of!(TableEntry, size) as i32;
    pub const OFF_IS_HOST: i32 = std::mem::offset_of!(TableEntry, is_host) as i32;
    pub const SIZE: i64 = std::mem::size_of::<TableEntry>() as i64;
}
