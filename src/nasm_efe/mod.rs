
use std::collections::HashMap;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive;

use crate::{funs::{as_usize, get_db_data, trim_zeroes}, ops::OpCodes, syscalls::SysCalls};


#[derive(Debug)]
pub enum PreCompile{
    I(Instuction),
    L(String)
}
//
//
//
//
// PROGRAM CANT HANDLE LABELS FROM DİFFERENT PLACESES
// BECAUSE İT WİLL CHANGE THE LOCATİON OF THE LABEL 
//
//
//
//
//
//


#[derive(Debug)]
pub struct Instuction{
    pub opcode:OpCodes,
    pub operandl:Operands,
    pub operandr:Operands,
}
impl Instuction {
    
    pub fn from_slice(){}
    pub fn get_len(&self) -> usize {
        let mut len = 1 ;
        let b = self.operandl.to_int();
        len += b.len();
        let b = self.operandr.to_int();
        len += b.len();
        len
    }
    pub fn get_program(&self) -> Vec<u8> {
        let mut v = Vec::new() ;
        v.push(self.opcode.to_u8().unwrap());
        let mut b = self.operandl.to_int();
        v.append(&mut b);
        let mut b = self.operandr.to_int();
        v.append(&mut b);
        v
    }
    pub fn to_program(&self,labelmap:&HashMap<String,usize>) -> Vec<u8>{
        let mut p: Vec<u8> = Vec::new();
        let op = self.opcode.to_u8().unwrap();
        p.push(op);
        match &self.operandl {
            Operands::Label(l) => {

                let a = labelmap.get(l).unwrap();
                let mut l = trim_zeroes(a.to_be_bytes().to_vec());
                p.push(0);
                p.push(l.len() as u8);
                p.append(&mut l);
            },
            _ => {
                let mut a = self.operandl.to_int();
                p.append(&mut a);
            },
        };
        match &self.operandr {
            Operands::Label(l) => {

                let a = labelmap.get(l).unwrap();
                let mut l = trim_zeroes(a.to_be_bytes().to_vec());
                p.push(0);
                p.push(l.len() as u8);
                p.append(&mut l);
            },
            _ => {
                let mut a = self.operandr.to_int();
                p.append(&mut a);
            },
        };
        p


    }
}

#[derive(Debug,PartialEq)]
pub enum SimpleOperands{
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
            OperandType::EAX |
            OperandType::EBX |
            OperandType::ECX |
            OperandType::EDX |
            OperandType::EBP |
            OperandType::ESP |
            OperandType::E1 |
            OperandType::E2 |
            OperandType::E3 |
            OperandType::E4 |
            OperandType::E5 |
            OperandType::E6 |
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
            OperandType::NOP => Self::Nop,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Operands{
    Static(usize),
    String(Vec<u8>),
    Label(String),
    EAX,
    EBX,
    ECX,
    EDX,
    EBP,
    ESP,
    E1,
    E2,
    E3,
    E4,
    E5,
    E6,
    AL,
    AH,
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
            Operands::String(l)=> {
                let mut v = vec![OperandType::String.to_u8().unwrap()];
                v.push(l.len() as u8);
                let mut l = l.clone();
                v.append(&mut l);
                v
            },
            Operands::EAX => vec![OperandType::EAX.to_u8().unwrap()],
            Operands::EBX => vec![OperandType::EBX.to_u8().unwrap()],
            Operands::ECX => vec![OperandType::ECX.to_u8().unwrap()],
            Operands::EDX => vec![OperandType::EDX.to_u8().unwrap()],
            Operands::EBP => vec![OperandType::EBP.to_u8().unwrap()],
            Operands::ESP => vec![OperandType::ESP.to_u8().unwrap()],
            Operands::E1  => vec![OperandType::E1.to_u8().unwrap()],
            Operands::E2  => vec![OperandType::E2.to_u8().unwrap()],
            Operands::E3  => vec![OperandType::E3.to_u8().unwrap()],
            Operands::E4  => vec![OperandType::E4.to_u8().unwrap()],
            Operands::E5  => vec![OperandType::E5.to_u8().unwrap()],
            Operands::E6  => vec![OperandType::E6.to_u8().unwrap()],
            Operands::AL => vec![OperandType::AL.to_u8().unwrap()],
            Operands::AH => vec![OperandType::AH.to_u8().unwrap()],
            Operands::BL => vec![OperandType::BL.to_u8().unwrap()],
            Operands::BH => vec![OperandType::BH.to_u8().unwrap()],
            Operands::CL => vec![OperandType::CL.to_u8().unwrap()],
            Operands::CH => vec![OperandType::CH.to_u8().unwrap()],
            Operands::DL => vec![OperandType::DL.to_u8().unwrap()],
            Operands::DH => vec![OperandType::DH.to_u8().unwrap()],

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
            Operands::Null => vec![OperandType::NOP.to_u8().unwrap()],
            Operands::Label(_) => vec![OperandType::NOP.to_u8().unwrap()],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PtrInner{
    Static(usize),
    Reg(Registers),
    Sum(Registers,usize),
    Ext(Registers,usize),
    Extr(usize,Registers),
    SumReg(Registers,Registers),
    ExtReg(Registers,Registers),
}
impl PtrInner {
    fn from_str<S:Into<String>>(data:S) -> Option<Self>{
        let d:String = data.into();
        // girdi örnekleri
        // ebp+0xf0
        // ebp
        // çıktı örnekleri
        // i: ebp+0x0f
        // o: PtrInner::Sum(Registers::EBP,15)
        // 
        // i: 0x05-ebp
        // o: PtrInner::Extr(5,Registers::EBP)


        // sum: 10  01    00
        //      ext sum   no op
        let mut sum = 0b00;
        let mut sp:Vec<&str> = d.split("+").collect();

        if sp.len() >1{
            sum |= 0b01;
        }
        if sum & 0b01 != 1{
            sp = d.split("-").collect();
            if sp.len() >1{
                sum |= 0b10;
            }
        }
        if sum == 0b00{
            match Registers::from_str(d.clone()){
                Some(so) => return  Some(Self::Reg(so)),
                None => {
                    let num = as_usize(sp[0]);
                    return num.map(|a| Self::Static(a));
                },
            };
        }
        
        if sum == 0b01  {
            match Registers::from_str(sp[0]) {
                Some(so) => {
                    let num = as_usize(sp[1]);
                    return num.map(|a| Self::Sum(so, a));
                },
                None => {
                    match Registers::from_str(sp[1]) {
                        Some(so) => {
                            let num = as_usize(sp[0]);
                            return num.map(|a| Self::Sum(so, a));
                        },
                        None => {
                            // ???????????
                            return None;
                        },
                    };
        
                },
            }
        };
        if sum == 0b10{
            match Registers::from_str(sp[0]) {
                Some(so) => {
                    let num = as_usize(sp[1]);
                    return num.map(|a| Self::Ext(so, a));
                },
                None => {
                    match Registers::from_str(sp[1]) {
                        Some(so) => {
                            let num = as_usize(sp[0]);
                            return num.map(|a| Self::Extr(a, so));
                        },
                        None => {
                            return None;
                        },
                    };
        
                },
            }
        }
        
        println!("sum {:b}",sum);
        todo!()


    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Registers{
    EAX,
    EBX,
    ECX,
    EDX,
    EBP,
    ESP,
    E1,
    E2,
    E3,
    E4,
    E5,
    E6,
    AL,
    AH,
    BL,
    BH,
    CL,
    CH,
    DL,
    DH,
}
impl Registers {
    fn from_str<S:Into<String>>(data:S) -> Option<Self>{
        let d:String = data.into();
        match d.as_str() {
            "EAX" | "Eax" | "eax" => Some(Self::EAX),
            "EBX" | "Ebx" | "ebx" => Some(Self::EBX),
            "ECX" | "Ecx" | "ecx" => Some(Self::ECX),
            "EDX" | "Edx" | "edx" => Some(Self::EDX),
            "ESP" | "Esp" | "esp" => Some(Self::ESP),
            "EBP" | "Ebp" | "ebp" => Some(Self::EBP),
            "E1" | "e1" => Some(Self::E1),
            "E2" | "e2" => Some(Self::E2),
            "E3" | "e3" => Some(Self::E3),
            "E4" | "e4" => Some(Self::E4),
            "E5" | "e5" => Some(Self::E5),
            "E6" | "e6" => Some(Self::E6),
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
            Registers::EAX => OperandType::EAX,
            Registers::EBX => OperandType::EBX,
            Registers::ECX => OperandType::ECX,
            Registers::EDX => OperandType::EDX,
            Registers::EBP => OperandType::EBP,
            Registers::ESP => OperandType::ESP,
            Registers::E1  => OperandType::E1,
            Registers::E2  => OperandType::E2,
            Registers::E3  => OperandType::E3,
            Registers::E4  => OperandType::E4,
            Registers::E5  => OperandType::E5,
            Registers::E6  => OperandType::E6,
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
            OperandType::EAX => Self::EAX,
            OperandType::EBX => Self::EBX,
            OperandType::ECX => Self::ECX,
            OperandType::EDX => Self::EDX,
            OperandType::EBP => Self::EBP,
            OperandType::ESP => Self::ESP,
            OperandType::E1 => Self::E1,
            OperandType::E2 => Self::E2,
            OperandType::E3 => Self::E3,
            OperandType::E4 => Self::E4,
            OperandType::E5 => Self::E5,
            OperandType::E6 => Self::E6,
            _=> todo!(),
        }
    }
}

/// bytecode definition
#[repr(u8)]
#[derive(Debug, PartialEq, Eq,ToPrimitive,FromPrimitive,Clone)]
pub enum OperandType{
    Static = 0,
    EAX = 1,
    EBX,
    ECX,
    EDX,
    EBP,
    ESP = 6,
    E1,
    E2,
    E3,
    E4,
    E5,
    E6 = 12,
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




fn parse_operand_types<S:Into<String>>(data:S) -> Option<OperandType>{
    let d:String = data.into();
    match d.as_str() {
        "EAX" | "Eax" | "eax" => Some(OperandType::EAX),
        "EBX" | "Ebx" | "ebx" => Some(OperandType::EBX),
        "ECX" | "Ecx" | "ecx" => Some(OperandType::ECX),
        "EDX" | "Edx" | "edx" => Some(OperandType::EDX),
        "ESP" | "Esp" | "esp" => Some(OperandType::ESP),
        "EBP" | "Ebp" | "ebp" => Some(OperandType::EBP),
        "E1" | "e1" => Some(OperandType::E1),
        "E2" | "e2" => Some(OperandType::E2),
        "E3" | "e3" => Some(OperandType::E3),
        "E4" | "e4" => Some(OperandType::E4),
        "E5" | "e5" => Some(OperandType::E5),
        "E6" | "e6" => Some(OperandType::E6),

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
            as_usize(d).map(|_a| OperandType::Static)
            
        }
    }
}


pub fn parse_register_type_to_op(a:OperandType) -> Option<Operands>{
    match a {
        OperandType::EAX => Some(Operands::EAX),
        OperandType::EBX => Some(Operands::EBX),
        OperandType::ECX => Some(Operands::ECX),
        OperandType::EDX => Some(Operands::EDX),
        OperandType::EBP => Some(Operands::EBP),
        OperandType::ESP => Some(Operands::ESP),
        OperandType::E1 => Some(Operands::E1),
        OperandType::E2 => Some(Operands::E2),
        OperandType::E3 => Some(Operands::E3),
        OperandType::E4 => Some(Operands::E4),
        OperandType::E5 => Some(Operands::E5),
        OperandType::E6 => Some(Operands::E6),
        
        OperandType::AL => Some(Operands::AL),
        OperandType::AH => Some(Operands::AL),
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
        "SYSCALL" | "SysCall" | "syscall" | "Syscall" | "sysCall" => Some(OpCodes::SysCall),
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


        "DB" | "Db" | "db" => Some(OpCodes::Db),
        
        
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
            if iter.peek().is_none() || iter.peek().is_some_and(|x| *x == b'/'){
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
            if byte == b';'{
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
            OpCodes::Jmp |
            OpCodes::Je  |
            OpCodes::Jge |
            OpCodes::Jgt |
            OpCodes::Jle |
            OpCodes::Jlt |
            OpCodes::Jne |
            OpCodes::Jnz |
            OpCodes::Jz => {
                let mut  cc = Vec::new();
                while let Some(a) = iter.next() {
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

            OpCodes::Db => {
                let mut str :Vec<u8> = Vec::new();
                let mut a = iter;
                get_db_data(&mut a);
                todo!();
                continue;
            }
            _ => ()
        }



        let operandl: Operands = loop {
            let byte = iter.next(). unwrap();
            
            
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
                    OperandType::EAX |
                    OperandType::EBX |
                    OperandType::ECX |
                    OperandType::EDX |
                    OperandType::EBP |
                    OperandType::ESP |
                    OperandType::E1 |
                    OperandType::E2 |
                    OperandType::E3 |
                    OperandType::E4 |
                    OperandType::E5 |
                    OperandType::AL |
                    OperandType::AH |
                    OperandType::BL |
                    OperandType::BH |
                    OperandType::CL |
                    OperandType::CH |
                    OperandType::DL |
                    OperandType::DH |
                    OperandType::E6 => parse_register_type_to_op(a).unwrap(),
                    OperandType::Pointer |
                    OperandType::BYTEPTR  |
                    OperandType::WORDPTR  |
                    OperandType::DWORDPTR  |
                    OperandType::QWORDPTR  |
                    OperandType::String  |
                    OperandType::NOP => todo!(),
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
                    let ptrin = PtrInner::from_str(String::from_utf8(bbuu).unwrap().trim()).unwrap();
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
                let ptrin = PtrInner::from_str(String::from_utf8(bbuu).unwrap().trim()).unwrap();
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





        let operandr = loop {
            let byte = iter.next().unwrap();
            if byte == b';'{
                let str = String::from_utf8(buf.clone()).unwrap();
                let a = parse_operand_types(str.trim()).unwrap();
                let res = match a {
                    OperandType::Static => Operands::Static(as_usize(str.trim()).unwrap()),
                    OperandType::EAX |
                    OperandType::EBX |
                    OperandType::ECX |
                    OperandType::EDX |
                    OperandType::EBP |
                    OperandType::ESP |
                    OperandType::E1 |
                    OperandType::E2 |
                    OperandType::E3 |
                    OperandType::E4 |
                    OperandType::E5 |
                    OperandType::E6 |
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
                    OperandType::NOP => todo!(),
                    
                    
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
                    let ptrin = PtrInner::from_str(String::from_utf8(bbuu).unwrap().trim()).unwrap();
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
                let ptrin = PtrInner::from_str(String::from_utf8(bbuu).unwrap().trim()).unwrap();
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




mod nasm_compiler{
    use std::collections::HashMap;

    use num_traits::ToPrimitive;

    use crate::{funs::trim_zeroes, ops::OpCodes};

    use super::{Instuction, PreCompile};


    #[derive(Debug,)]
    enum Objects{
        Instruction{
            i:Instuction,
            len:usize
        },
        Jump{
            type_:OpCodes,
            label:String
        },
        AfterJump{
            type_:OpCodes,
            label:String,
            jump:Vec<u8>
        },
        Label(String),
        AfterLabel(String,usize)
    }
    #[derive(Debug)]
    pub struct Data{
        v :Vec<Objects>,
        labels:HashMap<String,usize>
    }
    impl Data{
        pub fn from_pre_compile(a:Vec<PreCompile>) -> Self{
            let mut v: Vec<Objects> = Vec::new();
            let mut labels = HashMap::new();
            for x in a{
                match x {
                    PreCompile::I(inst) => {
                        match inst.opcode {
                            OpCodes::Jmp |
                            OpCodes::Je |
                            OpCodes::Jne |
                            OpCodes::Jgt |
                            OpCodes::Jlt |
                            OpCodes::Jle |
                            OpCodes::Jge |
                            OpCodes::Jz |
                            OpCodes::Jnz => {
                                match inst.operandl{
                                    crate::nasm_efe::Operands::Label(l) => {
                                        v.push(Objects::Jump { type_: inst.opcode, label: l });
                                    },
                                    _ => todo!(),
                                };
                            },
                            _ => {
                                let len = inst.get_len();
                                v.push(Objects::Instruction { i: inst, len: len });
                            },
                        }
                    },
                    PreCompile::L(l) =>{
                        labels.insert(l.clone(), 0);
                        v.push(Objects::Label(l))
                    },
                }
            }
            
            Self{v,labels}
        }
        pub fn r1(&mut self){
            let mut current_len = 0;
            for x in &mut self.v{
                match x {
                    Objects::Instruction { i:_, len } => {
                        current_len += *len;
                    },
                    Objects::Jump { type_:_, label:_ } => {
                        current_len += 2;
                        
                    },
                    Objects::Label(l) => {
                        self.labels.insert(l.to_string(), current_len);
                        *x = Objects::AfterLabel(l.clone(), current_len);

                    },
                    _ => todo!(),
                }
            }
        }
        pub fn r2(&mut self){
            let mut current_len = 0;
            for x in &mut self.v{
                match x {
                    Objects::Instruction { i:_i, len } => {
                        current_len += *len;
                    },
                    Objects::Jump { type_, label } => {
                        let labeld = self.labels.get(label).unwrap();
                        let mut staticval = Vec::new();
                        staticval.push(type_.to_u8().unwrap());
                        staticval.push(0);
                        let mut addr = trim_zeroes(labeld.to_be_bytes().to_vec());
                        staticval.push(addr.len() as u8);
                        staticval.append(&mut addr);
                        current_len += staticval.len();
                        *x = Objects::AfterJump { type_: type_.clone(), label: label.clone(), jump: staticval }
                    },
                    Objects::AfterLabel(l, _addr) => {
                        let s = l.clone();
                        *x = Objects::AfterLabel(s.clone(), current_len);
                        self.labels.insert(s, current_len);
                    },
                    Objects::AfterJump { type_, label, jump: _jump } => {
                        let labeld = self.labels.get(label).unwrap();
                        let mut staticval = Vec::new();
                        staticval.push(type_.to_u8().unwrap());
                        staticval.push(0);
                        let mut addr = trim_zeroes(labeld.to_be_bytes().to_vec());
                        staticval.push(addr.len() as u8);
                        staticval.append(&mut addr);
                        current_len += staticval.len();
                        *x = Objects::AfterJump { type_: type_.clone(), label: label.clone(), jump: staticval }

                    },
                    _ => {
                        println!("panic {:?}",x);
                        panic!()

                    },
                }
            }
        }



        pub fn to_porgram(&self) -> Vec<u8>{
            let mut p = Vec::new();

            for x in &self.v{
                match x {
                    Objects::Instruction { i, len:_ } => {
                        let mut px = i.get_program();
                        p.append(&mut px);
                    },
                    Objects::Jump { type_:_, label:_ } => todo!(),
                    Objects::AfterJump { type_:_, label:_, jump } => {
                        let mut px = jump.clone();
                        p.append(&mut px);
                    },
                    Objects::Label(_) => (),
                    Objects::AfterLabel(_, _) => (),
                }
            }

            p
        } 

    }


}