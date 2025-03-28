use num_traits::FromPrimitive;

use crate::nasm_efe::OperandType;

use super::{as_be_bytes, SimpleOperands};
use super::instruction::{Operands, PtrInner, Registers};
use efepages::page::Page;

pub enum NumToStr{
    I(isize),
    U(usize)
}





#[derive(Debug)]
enum InnerTypes{
    REG(Registers),
    Static(usize)
}


pub(super) fn get_inner_ptr(start:&mut usize,program:&mut Page) -> Operands{
    
    
    
    let ctr = start;

    
    let endcode = program.read(*ctr, 1).try_into().unwrap();
    let endcode = u8::from_be_bytes(endcode);
    
    *ctr += 1;
    
    let operand1byte = program.read(*ctr, 1).try_into().unwrap();
    let operand1byte = u8::from_be_bytes(operand1byte);

    let operand1 = OperandType::from_u8(operand1byte.clone()).unwrap();
    let simpleoperand1 = SimpleOperands::from_operand(operand1.clone());
    let a1 = match simpleoperand1 {
        SimpleOperands::Reg => {
            *ctr +=1;
            InnerTypes::REG(Registers::from_operand(operand1))
        },
        SimpleOperands::Static => {
            *ctr +=1;
            let len = program.read(*ctr, 1).try_into().unwrap();
            let len = u8::from_be_bytes(len);
            
            if len == 0{
                *ctr += 1;
                InnerTypes::Static(0)
            }else{
                *ctr +=1;                
                let a = program.read(*ctr, 1);
            


                *ctr += len as usize;
                let a = as_be_bytes!(usize a);
                let a = usize::from_be_bytes(a);
                InnerTypes::Static(a)
            }
        },
        _ => todo!(),
    };
    let isnext = program.read(*ctr, 1).try_into().unwrap();
    let isnext = u8::from_be_bytes(isnext);

    if isnext == endcode {
        let inner = match a1 {
            InnerTypes::REG(registers) => PtrInner::Reg(registers),
            InnerTypes::Static(u) => PtrInner::Static(u),
        };
        match OperandType::from_u8(endcode).unwrap(){
            OperandType::BYTEPTR  => return Operands::BYTEPTR(inner),
            OperandType::WORDPTR  => return Operands::WORDPTR(inner),
            OperandType::DWORDPTR => return Operands::DWORDPTR(inner),
            OperandType::QWORDPTR => return Operands::QWORDPTR(inner),
            _ => todo!(),
        }
    };
    
    // second part
    let operator = program.read(*ctr, 1).try_into().unwrap();
    let operator = u8::from_be_bytes(operator);
    
    let sum = operator == b'+';
    *ctr +=1;

    let operand2byte = program.read(*ctr, 1).try_into().unwrap();
    let operand2byte = u8::from_be_bytes(operand2byte);

    let operand2 = OperandType::from_u8(operand2byte.clone()).unwrap();
    let simpleoperand2 = SimpleOperands::from_operand(operand2.clone());

    

    let a2: InnerTypes = match simpleoperand2 {
        SimpleOperands::Reg => {
            *ctr +=1;
            InnerTypes::REG(Registers::from_operand(operand2))
        },
        SimpleOperands::Static => {
            *ctr +=1;
            let len = program.read(*ctr, 1).try_into().unwrap();
            let len = u8::from_be_bytes(len);
            if len == 0{
                *ctr += 1;
                InnerTypes::Static(0)
            }else{
                *ctr +=1;
                let a = program.read(*ctr, len as usize);

                *ctr += len as usize;
                let a = as_be_bytes!(usize a);
                let a = usize::from_be_bytes(a);
                InnerTypes::Static(a)
            }        },
        SimpleOperands::Ptr =>{
            eprintln!("hataatatatataata {:?}  {:?}",a1,2);
            panic!("panik anı : hatata")
        }
        _ => todo!(),
    };
    let tuple = (a1,a2,sum);
    let inner = match tuple {
        (InnerTypes::REG(registers), InnerTypes::REG(registers2), true)  => {PtrInner::SumReg(registers, registers2)},
        (InnerTypes::REG(registers), InnerTypes::REG(registers2), false) => {PtrInner::ExtReg(registers, registers2)},
        (InnerTypes::REG(registers), InnerTypes::Static(u), true)  => {PtrInner::Sum(registers, u)},
        (InnerTypes::REG(registers), InnerTypes::Static(u), false) => {PtrInner::Ext(registers, u)},
        (InnerTypes::Static(u), InnerTypes::REG(registers), true)  => {PtrInner::Sum(registers, u)},
        (InnerTypes::Static(u), InnerTypes::REG(registers), false) => {PtrInner::Extr(u, registers)},
        (InnerTypes::Static(u1), InnerTypes::Static(u2), true)  => {PtrInner::Static(u1+u2)},
        (InnerTypes::Static(u1), InnerTypes::Static(u2), false) => {PtrInner::Static(u1-u2)},
    };
    let code = OperandType::from_u8(endcode).unwrap();
    match code {
        OperandType::BYTEPTR => Operands::BYTEPTR(inner),
        OperandType::WORDPTR => Operands::WORDPTR(inner),
        OperandType::DWORDPTR => Operands::DWORDPTR(inner),
        OperandType::QWORDPTR => Operands::QWORDPTR(inner),
        OperandType::Pointer => Operands::Pointer(inner),
        _ => panic!("wrong ptr operand code")
    }    


}
