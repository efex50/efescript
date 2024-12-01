use efearena::Arena;





#[derive(Debug,Default)]
pub struct PRegisters{
    /// accumulator     d:1
    EAX:usize,
    /// base            d:2
    EBX:usize, 
    /// counter         d:3
    ECX:usize,
    /// data            d:4
    EDX:usize,
    /// stack base pointer     d:5 
    EBP:usize,
    /// stack pointer           d:6
    ESP:usize,
    //other registers
    R1:usize,
    R2:usize,
    R3:usize,
    R4:usize,
    R5:usize,
    R6:usize,
    
    flags:PFlags,

}

#[derive(Debug,Default)]
pub struct PFlags{
    CARRY    :bool,
    EQUALS   :bool,
    BIGGER   :bool,
    SMALLER  :bool,
    BIGGEREQ :bool,
    SMALLEREQ:bool,
    ZERO     :bool,
    NEGATIVE :bool,
}






#[derive(Debug)]
pub struct ProgramRuntime{
    pub program:Arena,
    pub stack:Arena,
    pub registers:PRegisters
}
