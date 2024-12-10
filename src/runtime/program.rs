use efearena::Arena;





#[derive(Debug,Default)]
pub struct PRegisters{
    /// accumulator     d:1
    eax:usize,
    /// base            d:2
    ebx:usize, 
    /// counter         d:3
    ecx:usize,
    /// data            d:4
    edx:usize,
    /// stack base pointer     d:5 
    ebp:usize,
    /// stack pointer           d:6
    esp:usize,
    //other registers
    r1:usize,
    r2:usize,
    r3:usize,
    r4:usize,
    r5:usize,
    r6:usize,
    
    flags:PFlags,

}

#[derive(Debug,Default)]
pub struct PFlags{
    carry    :bool,
    equals   :bool,
    bigger   :bool,
    smaller  :bool,
    biggereq :bool,
    smallereq:bool,
    zero     :bool,
    negative :bool,
}






#[derive(Debug)]
pub struct ProgramRuntime{
    pub program:Arena,
    pub stack:Arena,
    pub registers:PRegisters
}
