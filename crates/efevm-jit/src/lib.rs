//! Cranelift JIT backend for `efevm` (roadmap E7, phases J1-J4).
//!
//! [`compile`] turns a verified [`efevm::Module`] into native code ([`JitCode`]);
//! [`JitBackend`] plugs it into `efevm::Instance` through `efevm::backend::Backend`, and
//! [`enable`] does both. The JIT is observably identical to the interpreter: same exits,
//! results, memory, globals, budget accounting, trap kinds, trap locations and backtraces, and
//! fibers can move between the two at any suspension point. See `structure.md`.
//!
//! Native only: on `wasm32` this crate is empty.
#![cfg(not(target_arch = "wasm32"))]

mod backend;
mod compile;
mod ctx;
mod helpers;

pub use backend::{JitBackend, enable, native_runs, native_stack_fallbacks, native_stack_resume_fallbacks};
pub use compile::{JitCode, JitError, JitOptions, MAX_JIT_REGISTERS, OptLevel, compile};
