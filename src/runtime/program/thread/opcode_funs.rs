use efepages::page::Page;
use num_traits::FromPrimitive;

use crate::{as_be_bytes, instruction::{Operands, PtrInner, Registers}, runtime::{program::PRegisters, OperandType, SimpleOperands}};

use super::PThread;

impl PThread {
    pub(super) fn get_op_data(&mut self,a: &Operands) -> usize{

        match a {
            Operands::Static(s) => *s,
            // Operands::String(_vec) => todo!(),
            Operands::EAX => self.registers.eax,
            Operands::EBX => self.registers.ebx,
            Operands::ECX => self.registers.ecx,
            Operands::EDX => self.registers.edx,
            Operands::EBP => self.registers.ebp,
            Operands::ESP => self.registers.esp,
            Operands::R1 =>  self.registers.r1,
            Operands::R2 =>  self.registers.r2,
            Operands::R3 =>  self.registers.r3,
            Operands::R4 =>  self.registers.r4,
            Operands::R5 =>  self.registers.r5,
            Operands::R6 =>  self.registers.r6,
            Operands::AL => {
                let r = self.registers.eax;
                r & 0xff
            },
            Operands::AH => {
                let r = self.registers.eax;
                (r & 0xff00) >> 8
            },
            Operands::BL => {
                let r = self.registers.ebx;
                r & 0xff
            },
            Operands::BH => {
                let r = self.registers.ebx;
                (r & 0xff00) >> 8
    
            },
            Operands::CL => {
                let r = self.registers.ecx;
                r & 0xff
            },
            Operands::CH => {
                let r = self.registers.ecx;
                (r & 0xff00) >> 8
            },
            Operands::DL => {
                let r = self.registers.edx;
                r & 0xff
            },
            Operands::DH => {
                let r = self.registers.edx;
                (r & 0xff00) >> 8
            },
    
            Operands::BYTEPTR(ptr_inner) =>  {
                let p = get_pointer_inner_addr(ptr_inner,&self.registers);
                let prg = unsafe {
                    &*self.program
                };
                let a = prg.read(p, 1)[0];
                a as usize
            },
            Operands::WORDPTR(ptr_inner) =>  {
                let p = get_pointer_inner_addr(ptr_inner,&self.registers);
                let prg = unsafe {
                    &*self.program
                };
                let v = prg.read(p, 2).try_into().unwrap();
                let a = u16::from_be_bytes(v);
                a as usize
            },
            Operands::DWORDPTR(ptr_inner) => {
                let p = get_pointer_inner_addr(ptr_inner,&self.registers);
                let prg = unsafe {
                    &*self.program
                };
                let v = prg.read(p, 4).try_into().unwrap();
                let a = u32::from_be_bytes(v);
                a as usize
            },
            Operands::QWORDPTR(ptr_inner) => {
                let p = get_pointer_inner_addr(ptr_inner,&self.registers);
                let prg = unsafe {
                    &*self.program
                };
                let v = prg.read(p, 8).try_into().unwrap();
                let a = u64::from_be_bytes(v);
                a as usize
            },
            Operands::Pointer(ptr_inner) => {
                get_pointer_inner_addr(ptr_inner,&self.registers)
    
            },
            n => {
    
                todo!("{:?}",n)
            },
        }
    
    }
    pub(super) fn write_op_data(&mut self,a:&Operands,r:usize){
        match a {
            Operands::EAX => self.registers.eax = r,
            Operands::EBX => self.registers.ebx = r,
            Operands::ECX => self.registers.ecx = r,
            Operands::EDX => self.registers.edx = r,
            Operands::EBP => self.registers.ebp = r,
            Operands::ESP => self.registers.esp = r,
            Operands::R1 =>  self.registers.r1 = r,
            Operands::R2 =>  self.registers.r2 = r,
            Operands::R3 =>  self.registers.r3 = r,
            Operands::R4 =>  self.registers.r4 = r,
            Operands::R5 =>  self.registers.r5 = r,
            Operands::R6 =>  self.registers.r6 = r,
            Operands::BYTEPTR(ptr_inner) | Operands::Pointer(ptr_inner) => {
                let addr = get_pointer_inner_addr(ptr_inner,&self.registers);

                let v: [u8; 1] = (r as u8).to_be_bytes();
                let prg = unsafe {
                    &mut *self.program
                };
                prg.write(addr, v.to_vec());
            },
            Operands::WORDPTR(ptr_inner) => {
                let addr = get_pointer_inner_addr(ptr_inner,&self.registers);
                let v: [u8; 2] = (r as u16).to_be_bytes();
                let prg = unsafe {
                    &mut *self.program
                };
                prg.write(addr, v.to_vec());
            },
            Operands::DWORDPTR(ptr_inner) => {
                let addr = get_pointer_inner_addr(ptr_inner,&self.registers);

                let v: [u8; 4] = (r as u32).to_be_bytes();
                let prg = unsafe {
                    &mut *self.program
                };
                prg.write(addr, v.to_vec());
            },
            Operands::QWORDPTR(ptr_inner) => {
                let addr = get_pointer_inner_addr(ptr_inner,&self.registers);

                let v: [u8; 8] = (r as u64).to_be_bytes();
                let prg = unsafe {
                    &mut *self.program
                };
                prg.write(addr, v.to_vec());
            },
            Operands::Null => todo!(),
            Operands::AL => {
                let r = r ;
                let reg = self.registers.eax;
                let upper = reg & 0xff;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.eax = reg3;
            },
            Operands::AH => {
                let r = r <<8;
                let reg = self.registers.eax;
                let upper = reg & 0xff00;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.eax = reg3;
            },
            Operands::BL => {
                let r = r ;
                let reg = self.registers.ebx;
                let upper = reg & 0xff;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.ebx = reg3
            },
            Operands::BH => {
                let r = r <<8;
                let reg = self.registers.ebx;
                let upper = reg & 0xff00;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.ebx = reg3;
            },
            Operands::CL => {
                let r = r ;
                let reg = self.registers.ecx;
                let upper = reg & 0xff;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.ecx = reg3;
            },
            Operands::CH => {
                let r = r <<8;
                let reg = self.registers.ecx;
                let upper = reg & 0xff00;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.ecx = reg3;
            },
            Operands::DL => {
                let r = r ;
                let reg = self.registers.edx;
                let upper = reg & 0xff;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.edx = reg3;
            },
            Operands::DH => {
                let r = r <<8;
                let reg = self.registers.edx;
                let upper = reg & 0xff00;
                let reg2 = reg - upper;
                let reg3 = reg2 + r;
                self.registers.edx = reg3;
            },
            _ => {
                println!("{:?}",a);
                panic!()
            },
        }
        
    }

}



fn get_pointer_inner_addr(a:&PtrInner,regs:&PRegisters) -> usize{
    match a {
        PtrInner::Static(a) => *a,
        PtrInner::Reg(registers) => regtype_to_reg_data(registers,regs),
        PtrInner::Sum(registers, a) => regtype_to_reg_data(registers,regs) + a,
        PtrInner::Ext(registers, a) => regtype_to_reg_data(registers,regs) - a,
        PtrInner::Extr(a, registers) => a - regtype_to_reg_data(registers,regs),
        PtrInner::SumReg(registers, registers1) => regtype_to_reg_data(registers,regs) + regtype_to_reg_data(registers1,regs),
        PtrInner::ExtReg(registers, registers1) => regtype_to_reg_data(registers,regs) + regtype_to_reg_data(registers1,regs),
    }
}


fn regtype_to_reg_data(a:&Registers,regs:&PRegisters) -> usize{
    match a {
        Registers::EAX => regs.eax,
        Registers::EBX => regs.ebx,
        Registers::ECX => regs.ecx,
        Registers::EDX => regs.edx,
        Registers::EBP => regs.ebp,
        Registers::ESP => regs.esp,
        Registers::R1 => regs.r1,
        Registers::R2 => regs.r2,
        Registers::R3 => regs.r3,
        Registers::R4 => regs.r4,
        Registers::R5 => regs.r5,
        Registers::R6 => regs.r6,
        Registers::AL => {
            let r = regs.eax;
            r | 0xff
        },
        Registers::AH => {
            let r = regs.eax;
            (r | 0xff00) >> 8
        },
        Registers::BL => {
            let r = regs.ebx;
            r | 0xff
        },
        Registers::BH => {
            let r = regs.ebx;
            (r | 0xff00) >> 8

        },
        Registers::CL => {
            let r = regs.ecx;
            r | 0xff
        },
        Registers::CH => {
            let r = regs.ecx;
            (r | 0xff00) >> 8
        },
        Registers::DL => {
            let r = regs.edx;
            r | 0xff
        },
        Registers::DH => {
            let r = regs.edx;
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
