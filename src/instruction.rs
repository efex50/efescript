
use crate::ops::OpCodes;


#[derive(Debug)]
pub(crate) struct Instuction{
    pub opcode:OpCodes,
    pub operandl:Operands,
    pub operandr:Operands,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Operands{
    Static(usize),
    String(Vec<u8>),//todo
    Label(String),
    EAX,
    EBX,
    ECX,
    EDX,
    EBP,
    ESP,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6,
    AL,
    AH,//?
    BL,
    BH,
    CL,
    CH,
    DL,
    DH,
    Pointer(PtrInner),
    BYTEPTR (PtrInner),
    WORDPTR (PtrInner),
    DWORDPTR(PtrInner),
    QWORDPTR(PtrInner),
    Null,
}


#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PtrInner{
    Static(usize),
    Reg(Registers),
    Sum(Registers,usize),
    Ext(Registers,usize),
    Extr(usize,Registers),
    SumReg(Registers,Registers),
    ExtReg(Registers,Registers),
}


#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Registers{
    EAX,
    EBX,
    ECX,
    EDX,
    EBP,
    ESP,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6,
    AL,
    AH,
    BL,
    BH,
    CL,
    CH,
    DL,
    DH,
}