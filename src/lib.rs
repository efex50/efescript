pub mod prelude;

pub(crate)mod ops;
pub(crate)mod macros;
pub(crate)mod funs;
pub(crate)mod syscalls;
pub mod nasm_efe;
pub(crate)mod runtime;
pub mod compiler;



proc_macros::max_addr!();



