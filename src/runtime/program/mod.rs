pub mod runtime;
pub mod init;
mod syscall;
pub mod opcode;


use efepages::page::Page;




#[derive(Debug,Default)]
pub struct PRegisters{
    /// accumulator     d:1
    pub eax:usize,
    /// base            d:2
    pub ebx:usize, 
    /// counter         d:3
    pub ecx:usize,
    /// data            d:4
    pub edx:usize,
    /// stack base pointer     d:5 
    pub ebp:usize,
    /// stack pointer           d:6
    pub esp:usize,
    //other registers
    pub r1:usize,
    pub r2:usize,
    pub r3:usize,
    pub r4:usize,
    pub r5:usize,
    pub r6:usize,
    

}

#[derive(Debug,Default)]
pub struct PFlags{
    carry    :bool,
    equals   :bool,
    greater   :bool,
    lesser  :bool,
    greatereq :bool,
    lessereq:bool,
    zero     :bool,
    negative :bool,
}
impl PFlags {
    pub fn reset(&mut self){
        *self = Self::default();
    }
}





#[derive(Debug,Default)]
pub struct ProgramRuntime{
    pub counter:usize,
    pub program:Page,
    pub registers:PRegisters,
    pub flags:PFlags,
    pub stdin:Vec<u8>,
    pub stdout:Vec<u8>,

}
