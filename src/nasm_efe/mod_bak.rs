

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive;
use v2::OperandType;

use crate::{funs::{as_usize, get_db_data, trim_zeroes}, instruction::{Instuction, Operands, PtrInner, Registers}, ops::OpCodes};
pub mod nasm_compiler;
pub mod v2;
pub mod nasm_funs;

#[derive(Debug)]
pub(crate) enum PreCompile{
    I(Instuction),
    L(String)
}
//
//
// bu nası ingilizce amk
//
// düzeldi btw bu arada 
//
// PROGRAM CANT HANDLE LABELS FROM DİFFERENT PLACESES
// BECAUSE İT WİLL CHANGE THE LOCATİON OF THE LABEL 
//
//
// todo:
// koca tabloları başka dosyaya taşı çok kalabalık yapıyo
//
//
//


impl Operands{
    fn to_int(&self) -> Vec<u8>{
        match self {
            Operands::Static(l) => {
                let mut v = vec![OperandType::Static.to_u8().unwrap()];
                let mut l = trim_zeroes(l.to_be_bytes().to_vec());
                v.push(l.len() as u8);
                v.append(&mut l);
                v
            },
            // Operands::String(l)=> {
            //     let mut v = vec![OperandType::String.to_u8().unwrap()];
            //     v.push(l.len() as u8);
            //     let mut l = l.clone();
            //     v.append(&mut l);
            //     v
            // },
            Operands::RA => vec![OperandType::RA.to_u8().unwrap()],
            Operands::RB => vec![OperandType::RB.to_u8().unwrap()],
            Operands::RC => vec![OperandType::RC.to_u8().unwrap()],
            Operands::RD => vec![OperandType::RD.to_u8().unwrap()],
            Operands::RBP => vec![OperandType::RBP.to_u8().unwrap()],
            Operands::RSP => vec![OperandType::RSP.to_u8().unwrap()],
            Operands::R1  => vec![OperandType::R1.to_u8().unwrap()],
            Operands::R2  => vec![OperandType::R2.to_u8().unwrap()],
            Operands::R3  => vec![OperandType::R3.to_u8().unwrap()],
            Operands::R4  => vec![OperandType::R4.to_u8().unwrap()],
            Operands::R5  => vec![OperandType::R5.to_u8().unwrap()],
            Operands::R6  => vec![OperandType::R6.to_u8().unwrap()],
            Operands::AL  => vec![OperandType::AL.to_u8().unwrap()],
            Operands::AH  => vec![OperandType::AH.to_u8().unwrap()],
            Operands::BL  => vec![OperandType::BL.to_u8().unwrap()],
            Operands::BH  => vec![OperandType::BH.to_u8().unwrap()],
            Operands::CL  => vec![OperandType::CL.to_u8().unwrap()],
            Operands::CH  => vec![OperandType::CH.to_u8().unwrap()],
            Operands::DL  => vec![OperandType::DL.to_u8().unwrap()],
            Operands::DH  => vec![OperandType::DH.to_u8().unwrap()],

            Operands::Pointer(a) =>{
                let mut v = vec![OperandType::Pointer.to_u8().unwrap()];
                match a {
                    PtrInner::Static(s) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                    },
                    PtrInner::Reg(registers) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::Sum(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Ext(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Extr(s, registers) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                        v.push(b'-');
                        v.push(registers.to_operand().to_u8().unwrap());

                    },
                    PtrInner::SumReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        v.push(registers1.to_operand().to_u8().unwrap());

                    },
                    PtrInner::ExtReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        v.push(registers1.to_operand().to_u8().unwrap());
                    },
                }
                v.push(OperandType::BYTEPTR.to_u8().unwrap());
                v
            },
            // pointers end with same operand code

            Operands::BYTEPTR( _ptr_inner) => {
                let mut v = vec![OperandType::BYTEPTR.to_u8().unwrap()];
                match _ptr_inner {
                    PtrInner::Static(s) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                    },
                    PtrInner::Reg(registers) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::Sum(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Ext(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Extr(s, registers) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                        v.push(b'-');
                        v.push(registers.to_operand().to_u8().unwrap());

                    },
                    PtrInner::SumReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        v.push(registers1.to_operand().to_u8().unwrap());

                    },
                    PtrInner::ExtReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        v.push(registers1.to_operand().to_u8().unwrap());
                    },
                }
                v.push(OperandType::BYTEPTR.to_u8().unwrap());
                v
            },
            // pointers end with same operand code
            Operands::WORDPTR( _ptr_inner) => {
                let mut v = vec![OperandType::WORDPTR.to_u8().unwrap()];
                match _ptr_inner {
                    PtrInner::Static(s) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                    },
                    PtrInner::Reg(registers) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::Sum(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());

                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Ext(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Extr(s, registers) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                        v.push(b'-');
                        v.push(registers.to_operand().to_u8().unwrap());

                    },
                    PtrInner::SumReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        v.push(registers1.to_operand().to_u8().unwrap());

                    },
                    PtrInner::ExtReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        v.push(registers1.to_operand().to_u8().unwrap());
                    },
                }
                v.push(OperandType::WORDPTR.to_u8().unwrap());
                v
            },
            Operands::DWORDPTR( _ptr_inner) => {
                let mut v = vec![OperandType::DWORDPTR.to_u8().unwrap()];
                match _ptr_inner {
                    PtrInner::Static(s) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                    },
                    PtrInner::Reg(registers) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::Sum(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Ext(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());

                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Extr(s, registers) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                        v.push(b'-');
                        v.push(registers.to_operand().to_u8().unwrap());

                    },
                    PtrInner::SumReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        v.push(registers1.to_operand().to_u8().unwrap());

                    },
                    PtrInner::ExtReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        v.push(registers1.to_operand().to_u8().unwrap());
                    },
                }
                v.push(OperandType::DWORDPTR.to_u8().unwrap());
                v
            },
            Operands::QWORDPTR( _ptr_inner) => {
                let mut v = vec![OperandType::QWORDPTR.to_u8().unwrap()];
                match _ptr_inner {
                    PtrInner::Static(s) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                    },
                    PtrInner::Reg(registers) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::Sum(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Ext(registers, s) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);

                    },
                    PtrInner::Extr(s, registers) => {
                        let mut l = trim_zeroes(s.to_be_bytes().to_vec());
                        // static opcode
                        v.push(OperandType::Static.to_u8().unwrap());
                        v.push(l.len() as u8);
                        v.append(&mut l);
                        v.push(b'-');
                        v.push(registers.to_operand().to_u8().unwrap());
                    },
                    PtrInner::SumReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'+');
                        v.push(registers1.to_operand().to_u8().unwrap());

                    },
                    PtrInner::ExtReg(registers, registers1) => {
                        v.push(registers.to_operand().to_u8().unwrap());
                        v.push(b'-');
                        v.push(registers1.to_operand().to_u8().unwrap());
                    },
                }
                v.push(OperandType::QWORDPTR.to_u8().unwrap());
                v
            },
            Operands::Null => vec![OperandType::NULL.to_u8().unwrap()],
            Operands::Label(_) => vec![OperandType::NULL.to_u8().unwrap()],
        }
    }
}










impl PtrInner {
}


/*impl Registers {
    fn from_str<S:Into<String>>(data:S) -> Option<Self>{
        let d:String = data.into();
        match d.as_str() {
            "RA" | "Ra" | "ra" => Some(Self::RA),
            "RB" | "Rb" | "rb" => Some(Self::RB),
            "RC" | "Rc" | "rc" => Some(Self::RC),
            "RD" | "Rd" | "rd" => Some(Self::RD),
            "RSP" | "Rsp" | "rsp" => Some(Self::RSP),
            "RBP" | "Rbp" | "rbp" => Some(Self::RBP),
            "R1" | "r1" => Some(Self::R1),
            "R2" | "r2" => Some(Self::R2),
            "R3" | "r3" => Some(Self::R3),
            "R4" | "r4" => Some(Self::R4),
            "R5" | "r5" => Some(Self::R5),
            "R6" | "r6" => Some(Self::R6),
            "AL" | "Al" |"al" => Some(Self::AL),
            "AH" | "Ah" |"ah" => Some(Self::AH),
            "BL" | "Bl" |"bl" => Some(Self::BL),
            "BH" | "Bh" |"bh" => Some(Self::BH),
            "CL" | "Cl" |"cl" => Some(Self::CL),
            "CH" | "Ch" |"ch" => Some(Self::CH),
            "DL" | "Dl" |"dl" => Some(Self::DL),
            "DH" | "Dh" |"dh" => Some(Self::DH),

            _ => None
        }
    
    }
    pub fn to_operand(&self) -> OperandType{
        match self {
            Registers::RA => OperandType::RA,
            Registers::RB => OperandType::RB,
            Registers::RC => OperandType::RC,
            Registers::RD => OperandType::RD,
            Registers::RBP => OperandType::RBP,
            Registers::RSP => OperandType::RSP,
            Registers::R1  => OperandType::R1,
            Registers::R2  => OperandType::R2,
            Registers::R3  => OperandType::R3,
            Registers::R4  => OperandType::R4,
            Registers::R5  => OperandType::R5,
            Registers::R6  => OperandType::R6,
            Registers::AL => OperandType::AL,
            Registers::AH => OperandType::AH,
            Registers::BL => OperandType::BL,
            Registers::BH => OperandType::BH,
            Registers::CL => OperandType::CL,
            Registers::CH => OperandType::CH,
            Registers::DL => OperandType::DL,
            Registers::DH => OperandType::DH,
        }
    }
    pub fn from_operand(o:OperandType) -> Self{
        match o {
            OperandType::RA => Self::RA,
            OperandType::RB => Self::RB,
            OperandType::RC => Self::RC,
            OperandType::RD => Self::RD,
            OperandType::RBP => Self::RBP,
            OperandType::RSP => Self::RSP,
            OperandType::R1 => Self::R1,
            OperandType::R2 => Self::R2,
            OperandType::R3 => Self::R3,
            OperandType::R4 => Self::R4,
            OperandType::R5 => Self::R5,
            OperandType::R6 => Self::R6,
            OperandType::AL => Self::AL,
            OperandType::AH => Self::AH,
            OperandType::BL => Self::BL,
            OperandType::BH => Self::BH,
            OperandType::CL => Self::CL,
            OperandType::CH => Self::CH,
            OperandType::DL => Self::DL,
            OperandType::DH => Self::DH,
            _=> todo!(),
        }
    }
}*/

/*
/// bytecode definition
#[repr(u8)]
#[derive(Debug, PartialEq, Eq,ToPrimitive,FromPrimitive,Clone)]
pub enum OperandType{
    Static = 0,
    RA = 1,
    RB,
    RC,
    RD,
    RBP,
    RSP = 6,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6 = 12,
    AL,
    AH,
    BL,
    BH,
    CL,
    CH,
    DL,
    DH,
    Pointer = 0x16,
    BYTEPTR,
    WORDPTR,
    DWORDPTR,
    QWORDPTR,
    String,
    NOP = 0xff,
}
    */



fn parse_operand_types<S:Into<String>>(data:S) -> Option<OperandType>{
    let d:String = data.into();
    match d.as_str() {
        "RA" | "Eax" | "eax" => Some(OperandType::RA),
        "RB" | "Ebx" | "ebx" => Some(OperandType::RB),
        "RC" | "Ecx" | "ecx" => Some(OperandType::RC),
        "RD" | "Edx" | "edx" => Some(OperandType::RD),
        "RSP" | "Esp" | "esp" => Some(OperandType::RSP),
        "RBP" | "Ebp" | "ebp" => Some(OperandType::RBP),
        "R1" | "r1" => Some(OperandType::R1),
        "R2" | "r2" => Some(OperandType::R2),
        "R3" | "r3" => Some(OperandType::R3),
        "R4" | "r4" => Some(OperandType::R4),
        "R5" | "r5" => Some(OperandType::R5),
        "R6" | "r6" => Some(OperandType::R6),

        "AL" | "Al" |"al" => Some(OperandType::AL),
        "AH" | "Ah" |"ah" => Some(OperandType::AH),
        "BL" | "Bl" |"bl" => Some(OperandType::BL),
        "BH" | "Bh" |"bh" => Some(OperandType::BH),
        "CL" | "Cl" |"cl" => Some(OperandType::CL),
        "CH" | "Ch" |"ch" => Some(OperandType::CH),
        "DL" | "Dl" |"dl" => Some(OperandType::DL),
        "DH" | "Dh" |"dh" => Some(OperandType::DH),



        "BYTEPTR" | "Byteptr" | "byteptr" => Some(OperandType::BYTEPTR), 
        "WORDPTR" | "Wordptr" | "wordptr" => Some(OperandType::WORDPTR), 
        "DWORDPTR" | "Dwordptr" | "dwordptr" => Some(OperandType::DWORDPTR), 
        "QWORDPTR" | "Qwordptr" | "qwordptr" => Some(OperandType::QWORDPTR), 
        _ => {
            as_usize(d).ok().map(|_a| OperandType::Static)
            
        }
    }
}


pub(crate) fn parse_register_type_to_op(a:OperandType) -> Option<Operands>{
    match a {
        OperandType::RA => Some(Operands::RA),
        OperandType::RB => Some(Operands::RB),
        OperandType::RC => Some(Operands::RC),
        OperandType::RD => Some(Operands::RD),
        OperandType::RBP => Some(Operands::RBP),
        OperandType::RSP => Some(Operands::RSP),
        OperandType::R1 => Some(Operands::R1),
        OperandType::R2 => Some(Operands::R2),
        OperandType::R3 => Some(Operands::R3),
        OperandType::R4 => Some(Operands::R4),
        OperandType::R5 => Some(Operands::R5),
        OperandType::R6 => Some(Operands::R6),
        
        OperandType::AL => Some(Operands::AL),
        OperandType::AH => Some(Operands::AH),
        OperandType::BL => Some(Operands::BL),
        OperandType::BH => Some(Operands::BH),
        OperandType::CL => Some(Operands::CL),
        OperandType::CH => Some(Operands::CH),
        OperandType::DL => Some(Operands::DL),
        OperandType::DH => Some(Operands::DH),
        
        _ => None
    }
}



fn parse_opcode<S:Into<String>>(a:S) -> Option<OpCodes>{
    let a:String = a.into();
    match a.as_str() {
        "MOV" | "Mov" | "mov" => Some(OpCodes::Mov),
        "CMP" | "Cmp" | "cmp" => Some(OpCodes::Cmp),
        "TEST" | "Test" | "test" => Some(OpCodes::Test),

        "ADD8" | "Add8" | "add8" => Some(OpCodes::Add8),
        "ADD16" | "Add16" | "add16" => Some(OpCodes::Add16),
        "ADD32" | "Add32" | "add32" => Some(OpCodes::Add32),
        "ADD64" | "Add64" | "add64" => Some(OpCodes::Add64),

        "SUB8" | "Sub8" | "sub8" => Some(OpCodes::Sub8),
        "SUB16" | "Sub16" | "sub16" => Some(OpCodes::Sub16),
        "SUB32" | "Sub32" | "sub32" => Some(OpCodes::Sub32),
        "SUB64" | "Sub64" | "sub64" => Some(OpCodes::Sub64),

        "PUSH8" | "Push8" | "push8" => Some(OpCodes::Push8),
        "PUSH16" | "Push16" | "push16" => Some(OpCodes::Push16),
        "PUSH32" | "Push32" | "push32" => Some(OpCodes::Push32),
        "PUSH64" | "Push64" | "push64" => Some(OpCodes::Push64),
        "POP8" | "Pop8" | "pop8" => Some(OpCodes::Pop8),
        "POP16" | "Pop16" | "pop16" => Some(OpCodes::Pop16),
        "POP32" | "Pop32" | "pop32" => Some(OpCodes::Pop32),
        "POP64" | "Pop64" | "pop64" => Some(OpCodes::Pop64),
        // parser shit
        // needs rewrite
        // todo!
        "SYSCAL" | "SysCal" | "syscal" | "Syscal" | "sysCal" => Some(OpCodes::SysCall),
        "CALL" | "Call" | "call" => Some(OpCodes::Call),
        "RET" | "Ret" | "ret" => Some(OpCodes::Ret),


        "JMP" | "Jmp" | "jmp" => Some(OpCodes::Jmp),
        "JZ" | "Jz" | "jz" => Some(OpCodes::Jz),
        "JNZ" | "Jnz" | "jnz" => Some(OpCodes::Jnz),
        "JE" | "Je" | "je" => Some(OpCodes::Je),
        "JNE" | "Jne" | "jne" => Some(OpCodes::Jne),
        "JGT" | "Jgt" | "jgt" => Some(OpCodes::Jgt),
        "JLT" | "Jlt" | "jlt" => Some(OpCodes::Jlt),
        "JGE" | "Jge" | "jge" => Some(OpCodes::Jge),
        "JLE" | "Jle" | "jle" => Some(OpCodes::Jle),

        "OR" | "Or" | "or" => Some(OpCodes::Or),
        "NOR" | "Nor" | "nor" => Some(OpCodes::Nor),
        "XOR" | "Xor" | "xor" => Some(OpCodes::Xor),
        "AND" | "And" | "and" => Some(OpCodes::And),
        "NAND" | "Nand" | "nand" => Some(OpCodes::Nand),


        
        
        _ => None
    }
}

// OpCodes::Jmp |
// OpCodes::Je  |
// OpCodes::Jge |
// OpCodes::Jgt |
// OpCodes::Jle |
// OpCodes::Jls |
// OpCodes::Jne |
// OpCodes::Jnz |
// OpCodes::Jz => {






/// todo 
/// 
/// more error handling and error messages
fn parse_str_to_instructions(str:String) -> Vec<PreCompile> {
    
    let mut str = str;
    str.push('\n');
    let mut iter: std::slice::Iter<'_, u8> = str.as_bytes().iter();
    
    let mut program: Vec<PreCompile> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut lines:Vec<String> = Vec::new();
    loop {
        let a = iter.next();
        match a {
            Some(char) => {
                if *char == b'\n'{
                    if !buf.is_empty(){
                        let str = String::from_utf8(buf.clone()).unwrap();
                        lines.push(str.trim().to_string());
                    }
                    buf.clear();
                    continue;
                }
                buf.push(char.clone());
            },
            None => break,
        }
    }

    for x in &mut lines{
        x.push(';');
    }


    'main:for x in lines{
        buf.clear();
        let mut iter = x.bytes().into_iter().peekable();
        let opcode = loop{
            if iter.peek().is_none() || iter.peek().is_some_and(|x| *x == b';'){
                continue 'main;
            }
            let byte = iter.next().unwrap();
            if byte == b' ' {
                let str = String::from_utf8(buf.clone()).unwrap();
                
                // handle labels
                if str.trim() == "label" || str.trim() == "Label" || str.trim() == "LABEL"{
                    let mut cc = Vec::new();
                    while let Some(a) = iter.next() {
                        cc.push(a);
                    };
                    cc.pop();
                    let labl = String::from_utf8(cc).unwrap();
                    program.push(PreCompile::L(labl.trim().to_string()));
                    continue 'main;
                }


                let a = parse_opcode(str.trim()).unwrap();
                buf.clear();
                break a;
            }
            // syscall
            if byte == b'l'{
                let str = String::from_utf8(buf.clone()).unwrap();
                let op = parse_opcode(str.trim());
                
                if op.is_some(){
                    let i = Instuction{
                        opcode:op.unwrap(),
                        operandl:Operands::Null,
                        operandr:Operands::Null,
                    };
                    program.push(PreCompile::I(i));
                    continue 'main;
                }
            }
            buf.push(byte);
        };

        match opcode   
        {
            OpCodes::Jmp    |
            OpCodes::Je     |
            OpCodes::Jge    |
            OpCodes::Jgt    |
            OpCodes::Jle    |
            OpCodes::Jlt    |
            OpCodes::Jne    |
            OpCodes::Jnz    |
            OpCodes::Call   |
            OpCodes::Jz     => {
                let mut  cc = Vec::new();
                while let Some(a) = iter.next(){
                    cc.push(a);
                };
                cc.pop();
                let labl = String::from_utf8(cc).unwrap();
                let i = Instuction{
                    opcode:opcode,
                    operandl:Operands::Label(labl.trim().to_string())
                    ,operandr:Operands::Null
                };


                program.push(PreCompile::I(i));
                continue;
            },

            _ => ()
        }



        let operandl: Operands = loop {
            let byte = iter.next().unwrap();
            
            
            if byte == b','{
                let a = parse_operand_types(String::from_utf8(buf.clone()).unwrap().trim()).unwrap();
                let a = parse_register_type_to_op(a).unwrap();
                buf.clear();
                break a;
            }
            if byte == b';'{
                let str = String::from_utf8(buf.clone()).unwrap();
                let a = parse_operand_types(String::from_utf8(buf.clone()).unwrap().trim()).unwrap();
                break match a {
                    OperandType::Static => Operands::Static(as_usize(str.trim()).unwrap()),
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
                    OperandType::DH => parse_register_type_to_op(a).unwrap(),
                    OperandType::Pointer |
                    OperandType::BYTEPTR  |
                    OperandType::WORDPTR  |
                    OperandType::DWORDPTR  |
                    OperandType::QWORDPTR  |
                    OperandType::String  |
                    OperandType::NULL => todo!(),
                };

            }
            if byte == b'['{
                let str = String::from_utf8(buf.clone()).unwrap();
                if str.trim() == ""{
                    let mut bbuu: Vec<u8> = Vec::new();
                    // [x] bbuu == x
                    loop{
                        let a = iter.next().unwrap();
                        if a == b']'{break;}
                        bbuu.push(a);
                    }
                        let text = String::from_utf8(bbuu).unwrap();
                        let ptrin = PtrInner::from_str(text.trim()).unwrap();
                        let c = Operands::Pointer(ptrin);
                        break c;
                }
                
                let a = parse_operand_types(str.trim()).unwrap();
                let mut bbuu: Vec<u8> = Vec::new();
                // WORDPTR [x] -> bbuu == x
                loop{
                    let a = iter.next().unwrap();
                    if a == b']'{break;}
                    bbuu.push(a);
                }
                
                let text = String::from_utf8(bbuu).unwrap();
                println!("ptrin text {}",text);
                let ptrin = PtrInner::from_str(text.trim()).unwrap();
                println!("ptrin {:?}",ptrin);
                let c = match a {
                    OperandType::BYTEPTR  => Operands::BYTEPTR(ptrin),
                    OperandType::WORDPTR  => Operands::WORDPTR(ptrin),
                    OperandType::DWORDPTR => Operands::DWORDPTR(ptrin),
                    OperandType::QWORDPTR => Operands::QWORDPTR(ptrin),
                    _=> panic!("error in operand")
                };
                loop  {
                    if iter.next().unwrap() == b','{break;}
                }
                buf.clear();
                break c;

            }

            buf.push(byte);
        };

        if  opcode == OpCodes::Push8    ||
            opcode == OpCodes::Push16   ||
            opcode == OpCodes::Push32   ||
            opcode == OpCodes::Push64   ||
            opcode == OpCodes::Pop8     ||
            opcode == OpCodes::Pop16    ||
            opcode == OpCodes::Pop32    ||
            opcode == OpCodes::Pop64    ||
            opcode == OpCodes::Test
        {
            let i = Instuction{
                opcode,
                operandl,
                operandr:Operands::Null,
            };
            program.push(PreCompile::I(i));
            
            continue;
        };





        let operandr: Operands = loop {
            let byte = iter.next().unwrap();
            if byte == b';'{
                let str = String::from_utf8(buf.clone()).unwrap();
                let a = parse_operand_types(str.trim()).unwrap();
                let res = match a {
                    OperandType::Static => Operands::Static(as_usize(str.trim()).unwrap()),
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
                    OperandType::DH => parse_register_type_to_op(a).unwrap(),
                    OperandType::Pointer |
                    OperandType::BYTEPTR |
                    OperandType::WORDPTR |
                    OperandType::DWORDPTR |
                    OperandType::QWORDPTR |
                    OperandType::String |
                    OperandType::NULL => todo!(),
                    
                    
                };
                buf.clear();
                break res;
            }
            if byte == b'['{
                let str = String::from_utf8(buf.clone()).unwrap();
                if str.trim() == ""{
                    let mut bbuu: Vec<u8> = Vec::new();
                    // [x] bbuu == x
                    loop{
                        let a = iter.next().unwrap();
                        if a == b']'{break;}
                        bbuu.push(a);
                    }
                        let text = String::from_utf8(bbuu).unwrap();
                        let ptrin = PtrInner::from_str(text.trim()).unwrap();
                        let c = Operands::Pointer(ptrin);
                    break c;
                }
                let a = parse_operand_types(str.trim()).unwrap();
                let mut bbuu: Vec<u8> = Vec::new();
                // [x] bbuu == x
                loop{
                    let a = iter.next().unwrap();
                    if a == b']'{break;}
                    bbuu.push(a);
                }
                let text = String::from_utf8(bbuu).unwrap();
                let ptrin = PtrInner::from_str(text.trim()).unwrap();
                let c = match a {
                    OperandType::BYTEPTR  => Operands::BYTEPTR(ptrin),
                    OperandType::WORDPTR  => Operands::WORDPTR(ptrin),
                    OperandType::DWORDPTR => Operands::DWORDPTR(ptrin),
                    OperandType::QWORDPTR => Operands::QWORDPTR(ptrin),
                    _=> panic!("error in operand")
                };
                break c;

            }

            buf.push(byte);

        };
        let instuction = Instuction{
            opcode,
            operandl,
            operandr
        };
        program.push(PreCompile::I(instuction));
    };
    program
}


// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
pub fn parse_str<S:Into<String>>(str:S) -> Vec<u8> {
    let str:String = str.into();
    let inst = parse_str_to_instructions(str);



    let mut data = nasm_compiler::Data::from_pre_compile(inst);
    data.r1();
    data.r2();
    data.r2();

    let p = data.to_porgram();


    // let p = program.iter()
    //                 .filter_map(|a| if *a == 255{None} else{Some(*a)} )
    //                 .collect();
    // program;
    p
}

pub fn parse_from_file<S:Into<String>>(f:S) -> Vec<u8> {
    let f = f.into();
    let path = std::path::Path::new(&f);
    let v = std::fs::read(path).unwrap();
    parse_str(String::from_utf8(v).unwrap())

}




