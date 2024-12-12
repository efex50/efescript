use efepages::page::Page;
use num_traits::Zero;

use crate::{flags::{BIGGER, BIGGEREQ, EQUALS, NEGATIVE, SMALLER, SMALLEREQ, ZERO}, funs::reset_flags, ESP};

use super::{ data_funs::{get_op_data, write_op_data}, handle_syscall, nasm_efe::Instuction, ops::OpCodes, reg};

pub(super)  fn handle_opcodes(i:&Instuction,counter:&mut usize,stack:&mut Page){
    match i.opcode {
        OpCodes::Mov => {
            let r = get_op_data(&i.operandr,stack);
            write_op_data(&i.operandl,r,stack);
        },
        OpCodes::Add8 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u8 + r as u8;
            write_op_data(&i.operandl, s as usize,stack);
        },
        OpCodes::Add16 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u16 + r as u16;
            write_op_data(&i.operandl, s as usize,stack);
            
        },
        OpCodes::Add32 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u32 + r as u32;
            write_op_data(&i.operandl, s as usize,stack);
        },
        OpCodes::Add64 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u64 + r as u64;
            write_op_data(&i.operandl, s as usize,stack);
        },
        OpCodes::Sub8 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u8 - r as u8;
            write_op_data(&i.operandl, s as usize,stack);
        },
        OpCodes::Sub16 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u16 - r as u16;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Sub32 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u32 - r as u32;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Sub64 => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as u64 - r as u64;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Or => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as usize | r as usize;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Xor => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as usize ^ r as usize;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::And => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = ld as usize & r as usize;
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Nand => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = !(ld as usize & r as usize);
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Nor => {
            let r = get_op_data(&i.operandr,stack);
            let ld = get_op_data(&i.operandl,stack);
            let s = !(ld as usize | r as usize);
            write_op_data(&i.operandl, s as usize,stack);

        },
        OpCodes::Push8 => {
            let d = get_op_data(&i.operandl,stack);
            stack.write(reg!(ESP), vec![d as u8]);
            reg!(ESP = ESP+1);  
        },
        OpCodes::Push16 => {
            let d = get_op_data(&i.operandl,stack) as u16;
            let d = d.to_be_bytes().to_vec();
            stack.write(reg!(ESP), d);
            reg!(ESP = ESP+2);  
        },
        OpCodes::Push32 => {
            let d = get_op_data(&i.operandl,stack) as u32;
            let d = d.to_be_bytes().to_vec();
            stack.write(reg!(ESP), d);
            reg!(ESP = ESP+4);
        },
        OpCodes::Push64 => {
            let d = get_op_data(&i.operandl,stack) as u64;
            let d = d.to_be_bytes().to_vec();
            stack.write(reg!(ESP), d);
            reg!(ESP = ESP+8);  
        },
        OpCodes::Pop8 => {
            let d = stack.read(reg!(ESP) -1, 1);
            let d = u8::from_be_bytes(d.try_into().unwrap());
            reg!(ESP = ESP-1);
            write_op_data(&i.operandl, d as usize,stack);
        },
        OpCodes::Pop16 => {
            let d = stack.read(reg!(ESP) -2, 2);
            let d = u16::from_be_bytes(d.try_into().unwrap());
            reg!(ESP = ESP-2);
            write_op_data(&i.operandl, d as usize,stack);
        },
        OpCodes::Pop32 => {
            let d = stack.read(reg!(ESP) -4, 4);
            let d = u32::from_be_bytes(d.try_into().unwrap());
            reg!(ESP = ESP-4);
            write_op_data(&i.operandl, d as usize,stack);
        },
        OpCodes::Pop64 => {
            let d = stack.read(reg!(ESP) -8, 8);
            let d = u64::from_be_bytes(d.try_into().unwrap());
            reg!(ESP = ESP-8);
            write_op_data(&i.operandl, d as usize,stack);
        },
        // set flags accordingly to operands
        OpCodes::Cmp => {
            let first = get_op_data(&i.operandl,stack);
            let sec = get_op_data(&i.operandr,stack);
            reset_flags();
            reg!(EQUALS = first.eq(&sec));
            reg!(BIGGER = first.gt(&sec));
            reg!(SMALLER = first.lt(&sec));
            reg!(BIGGEREQ = first.ge(&sec));
            reg!(SMALLEREQ = first.le(&sec));
        },
        OpCodes::Test => {
            let first = get_op_data(&i.operandl,stack);
            reset_flags();
            reg!(ZERO = first.is_zero());
            reg!(NEGATIVE = first.lt(&0));
            reg!(BIGGER = first.gt(&0));
            reg!(SMALLER  = first.lt(&0));


        },
        // nasm should handle the addr to labels
        OpCodes::Jmp => {
            let addr = get_op_data(&i.operandl,stack);
            *counter = addr;
        },
        // go to function and add current addr to stack
        OpCodes::Call => {
            let addr = get_op_data(&i.operandl, stack);

            // push usize
            let d = *counter;
            let d = d.to_be_bytes().to_vec();
            let len = d.len();
            stack.write(reg!(ESP), d);
            reg!(ESP = ESP+len);  
            *counter = addr;
        },
        // get return addr from stack and return
        OpCodes::Ret => {

            let len = *counter;
            let len = len.to_be_bytes().to_vec().len();

            let d = stack.read(reg!(ESP) - len, len);
            let d = usize::from_be_bytes(d.try_into().unwrap());
            reg!(ESP = ESP-len);
            *counter = d;
        },
        OpCodes::SysCall => {
            handle_syscall::handle_syscalls(stack);
        },
        OpCodes::Nop => (),
        OpCodes::Je     => {
            if reg!(EQUALS){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }
        },
        OpCodes::Jne    => {
            if !reg!(EQUALS){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jgt    => {
            if reg!(BIGGER){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jlt    => {
            if reg!(SMALLER){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jle    => {
            if reg!(SMALLEREQ){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jge    => {
            if reg!(BIGGEREQ){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jz     => {
            if reg!(ZERO){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Jnz    => {
            if !reg!(ZERO){
                let addr = get_op_data(&i.operandl,stack);
                *counter = addr;    
            }

        },
        OpCodes::Db => todo!(),
    }
}