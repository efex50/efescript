pub mod prelude;

pub(crate)mod ops;
pub(crate)mod macros;
pub(crate)mod funs;
pub(crate)mod syscalls;
pub mod nasm_efe;
pub(crate)mod runtime;
pub mod compiler;
pub mod instruction;



proc_macros::max_addr!();
proc_macros::stack_page_count!();


