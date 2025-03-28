use num_traits::ToPrimitive;

use crate::{compiler::ast::lexer::{LexerTokenType, LexerTokens}, funs::{as_usize, trim_zeroes}, instruction::{Operands, PtrInner, Registers}, ops::OpCodes};

use super::{OperandType, ParseErr};



enum PtrInnerState{
    Single,
    Sum,
    Ext,
}
impl PtrInner {
    pub fn from_tokens(t:Vec<LexerTokens>) -> Result<Self,ParseErr>{
        let state = if t.len() == 1 {
            PtrInnerState::Single
        }else if t[1].token_type == LexerTokenType::Plus {
            PtrInnerState::Sum
        }else if t[1].token_type == LexerTokenType::Minus{
            PtrInnerState::Ext
        }else{
            return Err(ParseErr::WrongPtrInner(t[1].pos.start.clone()));
        };

        match state {
            PtrInnerState::Single => {
                let tag = t[0].token_type.get_inner_str().unwrap();
                match Registers::from_str(tag){
                    Some(so) => return  Ok(Self::Reg(so)),
                    None => {
                        let num = as_usize(tag);
                        return num
                            .map(|a| Self::Static(a))
                            .map_err(|_| ParseErr::WrongPtrInner(t[0].pos.start.clone()))
                            ;
                    },
                };
    

            },
            PtrInnerState::Sum => {
                let tokl = t[0].token_type.get_inner_str();
                if tokl.is_none(){return Err(ParseErr::WrongPtrInner(t[0].pos.start.clone()));}
                let tokl = t[0].token_type.get_inner_str().unwrap();
                
                let tokr = t[2].token_type.get_inner_str();
                if tokr.is_none(){return Err(ParseErr::WrongPtrInner(t[2].pos.start.clone()));}
                let tokr = t[2].token_type.get_inner_str().unwrap();
                
                match Registers::from_str(tokl) {
                    Some(sol) => {
                        match Registers::from_str(tokr) {
                            Some(sor) => {
                                return Ok(Self::SumReg(sol, sor));
                            },
                            None => {
                                let num = as_usize(tokr).unwrap();
                                return Ok(Self::Sum(sol, num));
                            },
                        };
                    },
                    None => {
                        match Registers::from_str(tokr) {
                            Some(so) => {
                                let num = as_usize(tokl);
                                return num
                                    .map(|a| Self::Sum(so, a))
                                    .map_err(|_| ParseErr::WrongPtrInner(t[2].pos.start.clone()));
                            },
                            None => {
                                let numl = as_usize(tokl)
                                    .map_err(|_| ParseErr::WrongPtrInner(t[0].pos.start.clone()))?;
                                let numr = as_usize(tokr)
                                .map_err(|_| ParseErr::WrongPtrInner(t[0].pos.start.clone()))?;
                                
                                return Ok(Self::Static(numl + numr))
                            },
                        };
                    },
                }
    

            },
            PtrInnerState::Ext => {
                let tokl = t[0].token_type.get_inner_str();
                if tokl.is_none(){return Err(ParseErr::WrongPtrInner(t[0].pos.start.clone()));}
                let tokl = t[0].token_type.get_inner_str().unwrap();
                
                let tokr = t[2].token_type.get_inner_str();
                if tokr.is_none(){return Err(ParseErr::WrongPtrInner(t[2].pos.start.clone()));}
                let tokr = t[2].token_type.get_inner_str().unwrap();
                
                match Registers::from_str(tokl) {
                    Some(sol) => {
                        match Registers::from_str(tokr) {
                            Some(sor) => {
                                return Ok(Self::ExtReg(sol, sor));
                            },
                            None => {
                                let num = as_usize(tokr).unwrap();
                                return Ok(Self::Ext(sol, num));
                            },
                        };
                    },
                    None => {
                        match Registers::from_str(tokr) {
                            Some(so) => {
                                let num = as_usize(tokl);
                                return num
                                    .map(|a| Self::Extr(a, so))
                                    .map_err(|_| ParseErr::WrongPtrInner(t[2].pos.start.clone()));
                            },
                            None => {
                                let numl = as_usize(tokl)
                                    .map_err(|_| ParseErr::WrongPtrInner(t[0].pos.start.clone()))?;
                                let numr = as_usize(tokr)
                                .map_err(|_| ParseErr::WrongPtrInner(t[0].pos.start.clone()))?;
                                
                                return Ok(Self::Static(numl - numr))
                            },
                        };
                    },
                }

            },
        }
    }


}

impl Registers {
    pub fn from_str<S:Into<String>>(data:S) -> Option<Self>{
        let d:String = data.into();
        let d = d.to_lowercase();
        match d.as_str() {
            "ra"  => Some(Self::RA),
            "rb"  => Some(Self::RB),
            "rc"  => Some(Self::RC),
            "rd"  => Some(Self::RD),
            "rsp" => Some(Self::RSP),
            "rbp" => Some(Self::RBP),
            "r1"  => Some(Self::R1),
            "r2"  => Some(Self::R2),
            "r3"  => Some(Self::R3),
            "r4"  => Some(Self::R4),
            "r5"  => Some(Self::R5),
            "r6"  => Some(Self::R6),
            "al"  => Some(Self::AL),
            "ah"  => Some(Self::AH),
            "bl"  => Some(Self::BL),
            "bh"  => Some(Self::BH),
            "cl"  => Some(Self::CL),
            "ch"  => Some(Self::CH),
            "dl"  => Some(Self::DL),
            "dh"  => Some(Self::DH),

            _ => None
        }
    
    }
    pub fn to_operand(&self) -> OperandType{
        match self {
            Registers::RA  => OperandType::RA,
            Registers::RB  => OperandType::RB,
            Registers::RC  => OperandType::RC,
            Registers::RD  => OperandType::RD,
            Registers::RBP => OperandType::RBP,
            Registers::RSP => OperandType::RSP,
            Registers::R1  => OperandType::R1,
            Registers::R2  => OperandType::R2,
            Registers::R3  => OperandType::R3,
            Registers::R4  => OperandType::R4,
            Registers::R5  => OperandType::R5,
            Registers::R6  => OperandType::R6,
            Registers::AL  => OperandType::AL,
            Registers::AH  => OperandType::AH,
            Registers::BL  => OperandType::BL,
            Registers::BH  => OperandType::BH,
            Registers::CL  => OperandType::CL,
            Registers::CH  => OperandType::CH,
            Registers::DL  => OperandType::DL,
            Registers::DH  => OperandType::DH,
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


impl Operands{
    pub fn to_int(&self) -> Vec<u8>{
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


pub fn parse_operand_type(a:OperandType) -> Option<Operands>{
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



pub fn parse_opcode<S:Into<String>>(a:S) -> Option<OpCodes>{
    let a:String = a.into();
    match a.as_str() {
        "mov" => Some(OpCodes::Mov),
        "cmp" => Some(OpCodes::Cmp),
        "test" => Some(OpCodes::Test),
        "add8" => Some(OpCodes::Add8),
        "add16" => Some(OpCodes::Add16),
        "add32" => Some(OpCodes::Add32),
        "add64" => Some(OpCodes::Add64),
        "sub8" => Some(OpCodes::Sub8),
        "sub16" => Some(OpCodes::Sub16),
        "sub32" => Some(OpCodes::Sub32),
        "sub64" => Some(OpCodes::Sub64),
        "push8" => Some(OpCodes::Push8),
        "push16" => Some(OpCodes::Push16),
        "push32" => Some(OpCodes::Push32),
        "push64" => Some(OpCodes::Push64),
        "pop8" => Some(OpCodes::Pop8),
        "pop16" => Some(OpCodes::Pop16),
        "pop32" => Some(OpCodes::Pop32),
        "pop64" => Some(OpCodes::Pop64),
        "syscall" => Some(OpCodes::SysCall),
        "call" => Some(OpCodes::Call),
        "ret" => Some(OpCodes::Ret),
        "jmp" => Some(OpCodes::Jmp),
        "jz" => Some(OpCodes::Jz),
        "jnz" => Some(OpCodes::Jnz),
        "je" => Some(OpCodes::Je),
        "jne" => Some(OpCodes::Jne),
        "jgt" => Some(OpCodes::Jgt),
        "jlt" => Some(OpCodes::Jlt),
        "jge" => Some(OpCodes::Jge),
        "jle" => Some(OpCodes::Jle),
        "or" => Some(OpCodes::Or),
        "nor" => Some(OpCodes::Nor),
        "xor" => Some(OpCodes::Xor),
        "and" => Some(OpCodes::And),
        "nand" => Some(OpCodes::Nand),
        "nop" => Some(OpCodes::Nop),


        
        
        _ => None
    }
}

