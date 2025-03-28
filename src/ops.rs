use num_derive::{FromPrimitive, ToPrimitive};


#[repr(u8)]
#[derive(Debug, PartialEq, Eq, FromPrimitive,ToPrimitive,Clone)]
pub enum OpCodes{
    
    Mov=1,
    Add8,
    Add16,
    Add32,
    Add64,
    Sub8,
    Sub16,
    Sub32,
    Sub64,
    Or,
    Xor,
    And,
    Nand,
    Nor,
    Lea,
    Push8 = 20,
    Push16,
    Push32,
    Push64,
    Pop8,
    Pop16,
    Pop32,
    Pop64,
    Cmp,
    Test,
    Jmp,

    /// jump equal
    Je,
    /// jump not equal
    Jne,
    /// jump greater than
    Jgt,
    /// jump lesser than
    Jlt,
    /// jump lesser equal
    Jle,
    /// jump greater equal
    Jge,
    /// jump zero
    Jz,
    /// jump not zero
    Jnz,
    /// jump signed
    Js,

    Call,
    Ret,
    SysCall,
    Nop = 255,
}

