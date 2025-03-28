pub(super)  mod data_funs;

// todo
#[allow(dead_code)]
pub mod program;

use crate::{nasm_efe::OperandType, *};
use nasm_efe::nasm_funs::parse_register_type_to_op;
use crate::instruction::Instuction;


#[derive(Debug,PartialEq)]
pub(crate) enum SimpleOperands{
    Reg,
    PushPop,
    Static,
    String,
    Ptr,
    Nop
}
impl SimpleOperands {
    pub fn from_operand(a:OperandType) -> Self{
        match a {
            OperandType::Static => Self::Static,
            OperandType::RA |
            OperandType::RB |
            OperandType::RC |
            OperandType::RD |
            OperandType::RBP |
            OperandType::RSP |
            OperandType::R1 |
            OperandType::R2 |
            OperandType::R3 |
            OperandType::R4 |
            OperandType::R5 |
            OperandType::R6 |
            OperandType::AL |
            OperandType::AH |
            OperandType::BL |
            OperandType::BH |
            OperandType::CL |
            OperandType::CH |
            OperandType::DL |
            OperandType::DH => Self::Reg,

            OperandType::Pointer | 
            OperandType::BYTEPTR |
            OperandType::WORDPTR |
            OperandType::DWORDPTR |
            OperandType::QWORDPTR => Self::Ptr,
            OperandType::String => Self::String,
            OperandType::NULL => Self::Nop,
        }
    }
}