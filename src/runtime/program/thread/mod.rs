pub mod runtime;
pub mod syscall;
pub mod opcode;
pub mod opcode_funs;

use efepages::page::Page;

use super::{PFlags, PRegisters};


#[derive(Debug)]
pub struct PThread{
    /// id of the thread
    pub id:u32,
    pub counter:usize,
    pub registers:PRegisters,
    pub flags:PFlags,


    pub stdin: *mut Vec<u8>,
    pub stdout:*mut Vec<u8>,
    pub stderr:*mut Vec<u8>,
    // unsafe
    pub program:*mut Page
}
impl Default for PThread {
    fn default() -> Self {
        Self { 
            id:         Default::default(),
            registers:  Default::default(),
            flags:      Default::default(),
            counter:    Default::default(), 
            stdin:   0 as _,
            stdout:  0 as _,
            program: 0 as _,
            stderr:  0 as _,
        }
    }
}



pub enum SyscallSignal{
    Ok,
    Finish(u8),
    Err(SyscallError),
}
#[repr(u8)]
pub enum SyscallError{
    Todo
}