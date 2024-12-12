use num_traits::FromPrimitive;

use super::{ as_be_bytes, nasm_efe::{OperandType, Operands, PtrInner, Registers, SimpleOperands}, reg, R1, R2, R3, R4, R5, R6, EAX, EBP, EBX, ECX, EDX, ESP};
use efepages::page::Page;

pub enum NumToStr{
    I(isize),
    U(usize)
}


pub(super)  fn write_op_data(a:&Operands,r:usize,stack:&mut Page){
    match a {
        Operands::EAX => reg!(EAX = r),
        Operands::EBX => reg!(EBX = r),
        Operands::ECX => reg!(ECX = r),
        Operands::EDX => reg!(EDX = r),
        Operands::EBP => reg!(EBP = r),
        Operands::ESP => reg!(ESP = r),
        Operands::R1 => reg!(R1 = r),
        Operands::R2 => reg!(R2 = r),
        Operands::R3 => reg!(R3 = r),
        Operands::R4 => reg!(R4 = r),
        Operands::R5 => reg!(R5 = r),
        Operands::R6 => reg!(R6 = r),
        Operands::BYTEPTR(ptr_inner) | Operands::Pointer(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            
            let v: [u8; 1] = (r as u8).to_be_bytes();
            stack.write(addr, v.to_vec());
        },
        Operands::WORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            let v: [u8; 2] = (r as u16).to_be_bytes();
            stack.write(addr, v.to_vec());
        },
        Operands::DWORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);

            let v: [u8; 4] = (r as u32).to_be_bytes();
            stack.write(addr, v.to_vec());
        },
        Operands::QWORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);

            let v: [u8; 8] = (r as u64).to_be_bytes();
            stack.write(addr, v.to_vec());
        },
        Operands::Null => todo!(),
        Operands::AL => {
            let r = r ;
            let reg = reg!(EAX);
            let upper = reg & 0xff;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EAX = reg3)
        },
        Operands::AH => {
            let r = r <<8;
            let reg = reg!(EAX);
            let upper = reg & 0xff00;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EAX = reg3)
        },
        Operands::BL => {
            let r = r ;
            let reg = reg!(EBX);
            let upper = reg & 0xff;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EBX = reg3)
        },
        Operands::BH => {
            let r = r <<8;
            let reg = reg!(EBX);
            let upper = reg & 0xff00;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EBX = reg3)
        },
        Operands::CL => {
            let r = r ;
            let reg = reg!(ECX);
            let upper = reg & 0xff;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(ECX = reg3)
        },
        Operands::CH => {
            let r = r <<8;
            let reg = reg!(ECX);
            let upper = reg & 0xff00;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(ECX = reg3)
        },
        Operands::DL => {
            let r = r ;
            let reg = reg!(EDX);
            let upper = reg & 0xff;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EDX = reg3)
        },
        Operands::DH => {
            let r = r <<8;
            let reg = reg!(EDX);
            let upper = reg & 0xff00;
            let reg2 = reg - upper;
            let reg3 = reg2 + r;
            reg!(EDX = reg3)
        },
        _ => {
            println!("{:?}",a);
            panic!()
        },
    }
}


pub(super) fn get_op_data(a:&Operands,stack:&mut Page) -> usize{
    match a {
        Operands::Static(s) => *s,
        Operands::String(_vec) => todo!(),
        Operands::EAX => reg!(EAX),
        Operands::EBX => reg!(EBX),
        Operands::ECX => reg!(ECX),
        Operands::EDX => reg!(EDX),
        Operands::EBP => reg!(EBP),
        Operands::ESP => reg!(ESP),
        Operands::R1 => reg!(R1),
        Operands::R2 => reg!(R2),
        Operands::R3 => reg!(R3),
        Operands::R4 => reg!(R4),
        Operands::R5 => reg!(R5),
        Operands::R6 => reg!(R6),
        Operands::AL => {
            let r = reg!(EAX);
            r & 0xff
        },
        Operands::AH => {
            let r = reg!(EAX);
            (r & 0xff00) >> 8
        },
        Operands::BL => {
            let r = reg!(EBX);
            r & 0xff
        },
        Operands::BH => {
            let r = reg!(EBX);
            (r & 0xff00) >> 8

        },
        Operands::CL => {
            let r = reg!(ECX);
            r & 0xff
        },
        Operands::CH => {
            let r = reg!(ECX);
            (r & 0xff00) >> 8
        },
        Operands::DL => {
            let r = reg!(EDX);
            r & 0xff
        },
        Operands::DH => {
            let r = reg!(EDX);
            (r & 0xff00) >> 8
        },

        Operands::BYTEPTR(ptr_inner) =>  {
            let p = get_pointer_inner_addr(ptr_inner);
            let a = stack.read(p, 1)[0];
            a as usize
        },
        Operands::WORDPTR(ptr_inner) =>  {
            let p = get_pointer_inner_addr(ptr_inner);
            let v = stack.read(p, 2).try_into().unwrap();
            let a = u16::from_be_bytes(v);
            a as usize
        },
        Operands::DWORDPTR(ptr_inner) => {
            let p = get_pointer_inner_addr(ptr_inner);
            let v = stack.read(p, 4).try_into().unwrap();
            let a = u32::from_be_bytes(v);
            a as usize
        },
        Operands::QWORDPTR(ptr_inner) => {
            let p = get_pointer_inner_addr(ptr_inner);
            let v = stack.read(p, 8).try_into().unwrap();
            let a = u64::from_be_bytes(v);
            a as usize
        },
        Operands::Pointer(ptr_inner) => {
            get_pointer_inner_addr(ptr_inner)

        },
        n => {

            todo!("{:?}",n)
        },
    }
}

fn get_pointer_inner_addr(a:&PtrInner) -> usize{
    match a {
        PtrInner::Static(a) => *a,
        PtrInner::Reg(registers) => regtype_to_reg_data(registers),
        PtrInner::Sum(registers, a) => regtype_to_reg_data(registers) + a,
        PtrInner::Ext(registers, a) => regtype_to_reg_data(registers) - a,
        PtrInner::Extr(a, registers) => a - regtype_to_reg_data(registers),
        PtrInner::SumReg(registers, registers1) => regtype_to_reg_data(registers) + regtype_to_reg_data(registers1),
        PtrInner::ExtReg(registers, registers1) => regtype_to_reg_data(registers) + regtype_to_reg_data(registers1),
    }
}


fn regtype_to_reg_data(a:&Registers) -> usize{
    match a {
        Registers::EAX => reg!(EAX),
        Registers::EBX => reg!(EBX),
        Registers::ECX => reg!(ECX),
        Registers::EDX => reg!(EDX),
        Registers::EBP => reg!(EBP),
        Registers::ESP => reg!(ESP),
        Registers::R1 => reg!(R1),
        Registers::R2 => reg!(R2),
        Registers::R3 => reg!(R3),
        Registers::R4 => reg!(R4),
        Registers::R5 => reg!(R5),
        Registers::R6 => reg!(R6),
        Registers::AL => {
            let r = reg!(EAX);
            r | 0xff
        },
        Registers::AH => {
            let r = reg!(EAX);
            (r | 0xff00) >> 8
        },
        Registers::BL => {
            let r = reg!(EBX);
            r | 0xff
        },
        Registers::BH => {
            let r = reg!(EBX);
            (r | 0xff00) >> 8

        },
        Registers::CL => {
            let r = reg!(ECX);
            r | 0xff
        },
        Registers::CH => {
            let r = reg!(ECX);
            (r | 0xff00) >> 8
        },
        Registers::DL => {
            let r = reg!(EDX);
            r | 0xff
        },
        Registers::DH => {
            let r = reg!(EDX);
            (r | 0xff00) >> 8
        },
    }
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
