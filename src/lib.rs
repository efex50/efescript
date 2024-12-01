pub mod prelude;

pub(crate)mod ops;
pub(crate)mod macros;
pub(crate)mod funs;
pub(crate)mod syscalls;
pub mod nasm_efe;
pub(crate)mod runtime;





/// accumulator     d:1
pub(crate)static mut EAX:usize = 0; 
/// base            d:2
pub(crate)static mut EBX:usize = 0;   
/// counter         d:3
pub(crate)static mut ECX:usize = 0;
/// data            d:4
pub(crate)static mut EDX:usize = 0;
/// stack frame pointer     d:5 
pub(crate)static mut EBP:usize = 0;
/// stack pointer           d:6
pub(crate)static mut ESP:usize = 0;
//other registers
pub(crate)static mut E1:usize = 0;
pub(crate)static mut E2:usize = 0;
pub(crate)static mut E3:usize = 0;
pub(crate)static mut E4:usize = 0;
pub(crate)static mut E5:usize = 0;
pub(crate)static mut E6:usize = 0;


pub(crate)mod flags{
    pub static mut CARRY    :bool = false;
    pub static mut EQUALS   :bool = false;
    pub static mut BIGGER   :bool = false;
    pub static mut SMALLER  :bool = false;
    pub static mut BIGGEREQ :bool = false;
    pub static mut SMALLEREQ:bool = false;
    pub static mut ZERO     :bool = false;
    pub static mut NEGATIVE :bool = false;
}

