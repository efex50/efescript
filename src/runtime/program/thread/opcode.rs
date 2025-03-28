use num_traits::Zero;

use crate::{ops::OpCodes, runtime::Instuction};
use super::{PThread, SyscallSignal};


pub enum HandleSignal{
    Ok,
    Todo(Box<dyn AsRef<str>>),
    Syscall(SyscallSignal)
}


impl PThread{
    
pub(super) fn handle_opcodes(&mut self,i:&Instuction) -> HandleSignal{

    let prg = unsafe {
        &mut *self.program
    };

    
    match i.opcode {
        OpCodes::Mov => {
                        let r = self.get_op_data(&i.operandr);
                        self.write_op_data(&i.operandl,r);
            },
        OpCodes::Add8 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u8 + r as u8;
                self.write_op_data(&i.operandl, s as usize);
            },
        OpCodes::Add16 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u16 + r as u16;
                self.write_op_data(&i.operandl, s as usize);
            
            },
        OpCodes::Add32 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u32 + r as u32;
                self.write_op_data(&i.operandl, s as usize);
            },
        OpCodes::Add64 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u64 + r as u64;
                self.write_op_data(&i.operandl, s as usize);
                self.write_op_data(&i.operandl, s as usize);
            },
        OpCodes::Sub8 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u8 - r as u8;
                self.write_op_data(&i.operandl, s as usize);
            },
        OpCodes::Sub16 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u16 - r as u16;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Sub32 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u32 - r as u32;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Sub64 => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as u64 - r as u64;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Or => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as usize | r as usize;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Xor => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as usize ^ r as usize;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::And => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = ld as usize & r as usize;
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Nand => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = !(ld as usize & r as usize);
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Nor => {
                let r = self.get_op_data(&i.operandr);
                let ld = self.get_op_data(&i.operandl);
                let s = !(ld as usize | r as usize);
                self.write_op_data(&i.operandl, s as usize);

            },
        OpCodes::Push8 => {
                let d = self.get_op_data(&i.operandl);
                prg.write(self.registers.rsp, vec![d as u8]);
                self.registers.rsp += 1;
            },
        OpCodes::Push16 => {
                let d = self.get_op_data(&i.operandl) as u16;
                let d = d.to_be_bytes().to_vec();
                prg.write(self.registers.rsp, d);
                self.registers.rsp += 2;
            },
        OpCodes::Push32 => {
                let d = self.get_op_data(&i.operandl) as u32;
                let d = d.to_be_bytes().to_vec();
                prg.write(self.registers.rsp, d);
                self.registers.rsp += 4;
            },
        OpCodes::Push64 => {
                let d = self.get_op_data(&i.operandl) as u64;
                let d = d.to_be_bytes().to_vec();
                prg.write(self.registers.rsp, d);
                self.registers.rsp += 8;
            },
        OpCodes::Pop8 => {
                let d = prg.read(self.registers.rsp -1, 1);
                let d = u8::from_be_bytes(d.try_into().unwrap());
                self.registers.rsp -= 1;
                self.write_op_data(&i.operandl, d as usize);
            },
        OpCodes::Pop16 => {
                let d = prg.read(self.registers.rsp -2, 2);
                let d = u16::from_be_bytes(d.try_into().unwrap());
                self.registers.rsp -= 2;
                self.write_op_data(&i.operandl, d as usize);
            },
        OpCodes::Pop32 => {
                let d = prg.read(self.registers.rsp -4, 4);
                let d = u32::from_be_bytes(d.try_into().unwrap());
                self.registers.rsp -= 4;
                self.write_op_data(&i.operandl, d as usize);
            },
        OpCodes::Pop64 => {
                let d = prg.read(self.registers.rsp -8, 8);
                let d = u64::from_be_bytes(d.try_into().unwrap());
                self.registers.rsp -= 8;
                self.write_op_data(&i.operandl, d as usize);
            },
        OpCodes::Cmp => {
                let first = self.get_op_data(&i.operandl);
                let sec = self.get_op_data(&i.operandr);
                self.flags.reset();
                self.flags.equals = first.eq(&sec);
                self.flags.greater = first.gt(&sec);
                self.flags.lesser = first.lt(&sec);
                self.flags.greatereq = first.ge(&sec);
                self.flags.lessereq = first.le(&sec);
            },
        OpCodes::Test => {
                let first = self.get_op_data(&i.operandl);
                self.flags.reset();
                self.flags.zero = first.is_zero();
                self.flags.lesser = first.lt(&0);
                self.flags.greater = first.gt(&0);
                self.flags.negative = first.lt(&0);
            },
        OpCodes::Jmp => {
                let addr = self.get_op_data(&i.operandl);
                self.counter = addr;
            },
        OpCodes::Call => {
                let addr = self.get_op_data(&i.operandl);

                // push usize
                let d = self.counter;
                let d = d.to_be_bytes().to_vec();
                let len = d.len();
                prg.write(self.registers.rsp, d);
                self.registers.rsp += len;
                self.counter = addr;
            },
        OpCodes::Ret => {

                let len = self.counter;
                let len = len.to_be_bytes().to_vec().len();

                let d = prg.read(self.registers.rsp - len, len);
                let d = usize::from_be_bytes(d.try_into().unwrap());
                self.registers.rsp -= len;
                self.counter = d;
            },
        OpCodes::SysCall => {
                let sig = self.handle_syscalls();
                return HandleSignal::Syscall(sig);
            },
        OpCodes::Nop => (),
        OpCodes::Je  => {
                if self.flags.equals{
                    self.get_op_data(&i.operandl);
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }
            },
        OpCodes::Jne    => {
                if !self.flags.equals{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jgt    => {
                if self.flags.greater{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jlt    => {
                if self.flags.lesser{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jle    => {
                if self.flags.lessereq{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jge    => {
                if self.flags.greatereq{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jz     => {
                if self.flags.zero{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Jnz    => {
                if !self.flags.zero{
                    let addr = self.get_op_data(&i.operandl);
                    self.counter = addr;    
                }

            },
        OpCodes::Js => {
            if self.flags.sign{
                let addr = self.get_op_data(&i.operandl);
                self.counter = addr;
            }
        },
        OpCodes::Lea => return HandleSignal::Todo(Box::new("Lea not implemented")),
            };
    return HandleSignal::Ok;
    
}


}

