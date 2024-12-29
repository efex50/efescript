pub mod init;
pub mod thread;
mod runtime;


use efepages::page::Page;
use thread::PThread;




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
    carry       :bool,
    equals      :bool,
    greater     :bool,
    lesser      :bool,
    greatereq   :bool,
    lessereq    :bool,
    zero        :bool,
    negative    :bool,
    sign        :bool,
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
    pub program_size:usize,
    pub threads:Vec<PThread>,
    pub stdin:Vec<u8>,
    pub stderr:Vec<u8>,
    pub stdout:Vec<u8>,

}
