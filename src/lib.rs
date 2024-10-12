pub mod ops;
pub mod macros;
pub mod funs;
pub mod syscalls;
pub mod nasm_efe;
pub mod runtime;
pub mod sdl2;

pub const STACK_SIZE:usize = u16::MAX as usize;

pub static mut HEAP_SIZE:usize = u32::MAX as usize;

/// accumulator     d:1
pub static mut EAX:usize = 0; 
/// base            d:2
pub static mut EBX:usize = 0;   
/// counter         d:3
pub static mut ECX:usize = 0;
/// data            d:4
pub static mut EDX:usize = 0;
/// stack frame pointer     d:5 
pub static mut EBP:usize = 0;
/// stack pointer           d:6
pub static mut ESP:usize = 0;
pub static mut E1:usize = 0;
pub static mut E2:usize = 0;
pub static mut E3:usize = 0;
pub static mut E4:usize = 0;
pub static mut E5:usize = 0;
pub static mut E6:usize = 0;
/// program data
pub static mut PROGRAM: [u8; STACK_SIZE] = [0u8;STACK_SIZE];
/// stack data
pub static mut STACK: [u8; STACK_SIZE] = [0u8;STACK_SIZE];


pub mod flags{
    pub static mut CARRY    :bool = false;
    pub static mut EQUALS   :bool = false;
    pub static mut BIGGER   :bool = false;
    pub static mut SMALLER  :bool = false;
    pub static mut BIGGEREQ :bool = false;
    pub static mut SMALLEREQ:bool = false;
    pub static mut ZERO     :bool = false;
    pub static mut NEGATIVE :bool = false;
}

