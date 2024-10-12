use text_io::read;

use crate::{*,STACK_SIZE,flags::*};
use funs::{as_usize, reset_flags};
use nasm_efe::{parse_register_type_to_op, Instuction, OperandType, Operands, PtrInner, Registers, SimpleOperands,};
use num_traits::{FromPrimitive, Zero};
use ops::OpCodes;
use syscalls::SysCalls;



#[derive(Debug)]
pub struct ProgramRuntime;
impl ProgramRuntime{
    pub fn load_from_file<S:Into<String>>(&self,a:S){
        let path:String = a.into();
        let path = std::path::Path::new(&path);
        let v = std::fs::read(path).unwrap();
        write_program!(v PROGRAM,0,v);

    }
    pub fn load_from_vec(&self,a:Vec<u8>){
        write_program!(v PROGRAM ,0,a);
    }
    pub fn print_slice(&self,start:usize,len:usize) -> String{
        let v = info_program!(asv PROGRAM start,len);
        format!("{:?}",v)
    }


    fn get_operand(&self,mut counter:&mut usize) -> Option<Operands>{
        let l = mem!(PROGRAM[*counter]);
        let lo = OperandType::from_u8(l).unwrap();
        let l = SimpleOperands::from_operand(lo.clone());
        let op = match l {
            SimpleOperands::Reg => {
                let ll = parse_register_type_to_op(lo).unwrap();
                ll
            }, 
            SimpleOperands::Static => {
                *counter +=1;
                let len = mem!(PROGRAM[*counter]);
                if len == 0 {
                    Operands::Static(0)
                }else{
                    *counter +=1;
                    let a = info_program!(asv PROGRAM *counter,len as usize);
                    let a = as_be_bytes!(usize a);
                    let a = usize::from_be_bytes(a);
                    *counter += len as usize -1;
                    Operands::Static(a)
                }
            },
            SimpleOperands::String => todo!(),
            SimpleOperands::Ptr => {
                let innerptr = get_inner_ptr(&mut counter);
                
                
                innerptr                    
            },
            SimpleOperands::Nop => {Operands::Null},
            SimpleOperands::PushPop => {
                todo!()
            },
        };
        Some(op)

    } 

    fn get_instruction(&self,counter:&mut usize) -> Option<Instuction>{
        let op = mem!(PROGRAM[*counter]);
        if op == 0 {return None;}
        let code = OpCodes::from_u8(op).unwrap();
        match code {
            OpCodes::Mov |
            OpCodes::Add8 |
            OpCodes::Add16 |
            OpCodes::Add32 |
            OpCodes::Add64 |
            OpCodes::Sub8 |
            OpCodes::Sub16 |
            OpCodes::Sub32 |
            OpCodes::Sub64 |
            OpCodes::Or |
            OpCodes::Xor |
            OpCodes::And |
            OpCodes::Nand |
            OpCodes::Nor |
            OpCodes::Cmp |
            OpCodes::Test => {
                *counter += 1;
                //left
                let lop = self.get_operand(counter).unwrap();
                
                *counter += 1;
                //left
                let rop = self.get_operand(counter).unwrap();
                *counter += 1;
                return Some(Instuction{
                    opcode:code,
                    operandl:lop,
                    operandr:rop
                })        

            },
            OpCodes::Call |
            OpCodes::Push8 |
            OpCodes::Push16 |
            OpCodes::Push32 |
            OpCodes::Push64 |
            OpCodes::Pop8 |
            OpCodes::Pop16 |
            OpCodes::Pop32 |
            OpCodes::Pop64 |
            OpCodes::Jmp |
            OpCodes::Je |
            OpCodes::Jne |
            OpCodes::Jgt |
            OpCodes::Jlt |
            OpCodes::Jle |
            OpCodes::Jge |
            OpCodes::Jz |
            OpCodes::Jnz => {
                *counter += 1;
                //left
                let lop = self.get_operand(counter).unwrap();
                *counter += 1;
                return Some(Instuction{
                    opcode:code,
                    operandl:lop,
                    operandr:Operands::Null
                })        
            },
            OpCodes::Ret |
            OpCodes::Nop |
            OpCodes::SysCall => {
                *counter +=1;
                return Some(Instuction{
                    opcode:code,
                    operandl:Operands::Null,
                    operandr:Operands::Null,
                });
    
            },
            OpCodes::Db => todo!(),
        }
    }

    pub fn start(&self){
        let mut counter = 0;
        loop {
            let i = self.get_instruction(&mut counter);
            let i = match i {
                Some(some) => {
                    some
                },
                None => break,
            };
            match i.opcode {
                OpCodes::Mov => {
                    let r = get_op_data(&i.operandr);
                    write_op_data(&i.operandl,r);
                },
                OpCodes::Add8 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u8 + r as u8;
                    write_op_data(&i.operandl, s as usize);
                },
                OpCodes::Add16 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u16 + r as u16;
                    write_op_data(&i.operandl, s as usize);
                    
                },
                OpCodes::Add32 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u32 + r as u32;
                    write_op_data(&i.operandl, s as usize);
                },
                OpCodes::Add64 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u64 + r as u64;
                    write_op_data(&i.operandl, s as usize);
                },
                OpCodes::Sub8 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u8 - r as u8;
                    write_op_data(&i.operandl, s as usize);
                },
                OpCodes::Sub16 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u16 - r as u16;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Sub32 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u32 - r as u32;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Sub64 => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as u64 - r as u64;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Or => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as usize | r as usize;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Xor => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as usize ^ r as usize;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::And => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = ld as usize & r as usize;
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Nand => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = !(ld as usize & r as usize);
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Nor => {
                    let r = get_op_data(&i.operandr);
                    let ld = get_op_data(&i.operandl);
                    let s = !(ld as usize | r as usize);
                    write_op_data(&i.operandl, s as usize);

                },
                OpCodes::Push8 => {
                    let d = get_op_data(&i.operandl);
                    mem!(STACK[ESP] = d as u8);
                    reg!(ESP = ESP+1);  
                },
                OpCodes::Push16 => {
                    let d = get_op_data(&i.operandl);
                    mem!(WORD be STACK[ESP] = d as u16);
                    reg!(ESP = ESP+8);  
                },
                OpCodes::Push32 => {
                    let d = get_op_data(&i.operandl);
                    mem!(DWORD be STACK[ESP] = d as u32);
                    reg!(ESP = ESP+8);  
                },
                OpCodes::Push64 => {
                    let d = get_op_data(&i.operandl);
                    mem!(QWORD be STACK[ESP] = d as u64);
                    reg!(ESP = ESP+8);  
                },
                OpCodes::Pop8 => {
                    let d = mem!(STACK[ESP-1]);
                    reg!(ESP = ESP-1);
                    write_op_data(&i.operandl, d as usize);
                },
                OpCodes::Pop16 => {
                    let d = mem!(WORD be STACK[ESP-2]);
                    reg!(ESP = ESP-2);
                    write_op_data(&i.operandl, d as usize);
                },
                OpCodes::Pop32 => {
                    let d = mem!(DWORD be STACK[ESP-4]);
                    reg!(ESP = ESP-4);
                    write_op_data(&i.operandl, d as usize);
                },
                OpCodes::Pop64 => {
                    let d = mem!(QWORD be STACK[ESP-8]);
                    reg!(ESP = ESP-8);
                    write_op_data(&i.operandl, d as usize);
                },
                // set flags accordingly to operands
                OpCodes::Cmp => {
                    let first = get_op_data(&i.operandl);
                    let sec = get_op_data(&i.operandr);
                    reset_flags();
                    reg!(EQUALS = first.eq(&sec));
                    reg!(BIGGER = first.gt(&sec));
                    reg!(SMALLER = first.lt(&sec));
                    reg!(BIGGEREQ = first.ge(&sec));
                    reg!(SMALLEREQ = first.le(&sec));
                },
                OpCodes::Test => {
                    let first = get_op_data(&i.operandl);
                    reset_flags();
                    reg!(ZERO = first.is_zero());
                    reg!(NEGATIVE = first.lt(&0));
                    reg!(BIGGER = first.gt(&0));
                    reg!(SMALLER = first.lt(&0));


                },
                // nasm should handle the addr to labels
                OpCodes::Jmp => {
                    let addr = get_op_data(&i.operandl);
                    counter = addr;
                },
                // go to function and add current addr to stack
                OpCodes::Call => todo!(),
                // get return addr from stack and return
                OpCodes::Ret => todo!(),
                OpCodes::SysCall => {
                    handle_syscalls();
                    continue;
                },
                OpCodes::Nop => {continue;},
                OpCodes::Je     => {
                    if reg!(EQUALS){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }
                },
                OpCodes::Jne    => {
                    if !reg!(EQUALS){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jgt    => {
                    if reg!(BIGGER){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jlt    => {
                    if reg!(SMALLER){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jle    => {
                    if reg!(SMALLEREQ){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jge    => {
                    if reg!(BIGGEREQ){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jz     => {
                    if reg!(ZERO){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Jnz    => {
                    if !reg!(EQUALS){
                        let addr = get_op_data(&i.operandl);
                        counter = addr;    
                    }

                },
                OpCodes::Db => todo!(),
                
            }
        }
    } 
}


enum NumToStr{
    I(isize),
    U(usize)
}

fn handle_syscalls(){
    let eax = reg!(EAX);
    let syscall = SysCalls::from_usize(eax).unwrap();
    match syscall {
        SysCalls::Printchar => {
            let char = reg!(EBX);
            print!("{}",char as u8 as char); 
        },
        SysCalls::Print => {
            let start = reg!(EBX);
            let len = reg!(ECX);
            let str = info_program!(asv STACK start,len);
            let str = String::from_utf8(str).unwrap();
            print!("{} ",str);
        },
        SysCalls::Println => {
            let start = reg!(EBX);
            let len = reg!(ECX);
            let str = info_program!(asv STACK start,len);
            let str = String::from_utf8(str).unwrap();
            println!("{}",str);
        },
        SysCalls::Finish => {
            let code = reg!(EBX);
            std::process::exit(code as i32);
        },
        SysCalls::NumToString => {
            let r = reg!(EBX);
            let i :NumToStr;
            if reg!(ECX) == 1{
                i = NumToStr::I(r as isize);
            }else {
                i = NumToStr::U(r as usize);
            }
            let s = match reg!(EDX) {
                1 => match i {NumToStr::I(s) => format!("{:x}",s), NumToStr::U(u) => format!("{:x}",u)}
                2 => match i {NumToStr::I(s) => format!("{:b}",s), NumToStr::U(u) => format!("{:b}",u)} ,
                3 => match i {NumToStr::I(s) => format!("{:o}",s), NumToStr::U(u) => format!("{:o}",u)} ,
                _ => match i {NumToStr::I(s) => format!("{}",s), NumToStr::U(u) => format!("{}",u)} 
            };
            let s:Vec<u8> = s.as_bytes().iter().map(|a| *a).collect();
            let len = s.len();
            reg!(ECX = len);
            reg!(EBX = ESP);
            write_program!(v STACK,ESP,s);
            reg!(ESP = ESP + len)
        },
        SysCalls::StringToNum => {
            let ptr = reg!(EBX);
            let len = reg!(ECX);
            let str = info_program!(asv STACK ptr,len);
            let str = String::from_utf8(str).unwrap();
            let num = as_usize(str).unwrap();
            reg!(EAX = num);           
        },
        SysCalls::Readln => {
            

            #[cfg(target_os = "windows")]
            let str: String = read!("{}\r");
            #[cfg(target_os = "linux")]
            let str: String = read!("{}\n");
            reg!(EBX = ESP);
            reg!(ECX = str.len());
            let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
            write_program!(v STACK,ESP,s);
            reg!(ESP = ESP+str.len())
        },
        SysCalls::Read => {
            
            #[cfg(target_os = "windows")]
            let str: String = read!("{}\r");
            #[cfg(target_os = "linux")]
            let str: String = read!("{}\n");
            reg!(EBX = ESP);
            reg!(ECX = str.len());
            let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
            write_program!(v STACK,ESP,s);
            reg!(ESP = ESP+str.len())
        },
        SysCalls::SdlEx1 => {
            todo!()
        },
    }
}


fn write_op_data(a:&Operands,r:usize){
    match a {
        Operands::EAX => reg!(EAX = r),
        Operands::EBX => reg!(EBX = r),
        Operands::ECX => reg!(ECX = r),
        Operands::EDX => reg!(EDX = r),
        Operands::EBP => reg!(EBP = r),
        Operands::ESP => reg!(ESP = r),
        Operands::E1 => reg!(E1 = r),
        Operands::E2 => reg!(E2 = r),
        Operands::E3 => reg!(E3 = r),
        Operands::E4 => reg!(E4 = r),
        Operands::E5 => reg!(E5 = r),
        Operands::E6 => reg!(E6 = r),
        Operands::BYTEPTR(ptr_inner) | Operands::Pointer(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            mem!(STACK[addr] = r as u8);
        },
        Operands::WORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            mem!(WORD be STACK[addr] = r as u16);
        },
        Operands::DWORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            mem!(DWORD be STACK[addr] = r as u32);
        },
        Operands::QWORDPTR(ptr_inner) => {
            let addr = get_pointer_inner_addr(ptr_inner);
            mem!(QWORD be STACK[addr] = r as u64);
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


fn get_op_data(a:&Operands) -> usize{
    match a {
        Operands::Static(s) => *s,
        Operands::String(_vec) => todo!(),
        Operands::EAX => reg!(EAX),
        Operands::EBX => reg!(EBX),
        Operands::ECX => reg!(ECX),
        Operands::EDX => reg!(EDX),
        Operands::EBP => reg!(EBP),
        Operands::ESP => reg!(ESP),
        Operands::E1 => reg!(E1),
        Operands::E2 => reg!(E2),
        Operands::E3 => reg!(E3),
        Operands::E4 => reg!(E4),
        Operands::E5 => reg!(E5),
        Operands::E6 => reg!(E6),
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
            let a = mem!(STACK[p]);
            a as usize
        },
        Operands::WORDPTR(ptr_inner) =>  {
            let p = get_pointer_inner_addr(ptr_inner);
            let a = mem!(WORD be STACK[p]);
            a as usize
        },
        Operands::DWORDPTR(ptr_inner) => {
            let p = get_pointer_inner_addr(ptr_inner);
            let a = mem!(DWORD be STACK[p]);
            a as usize
        },
        Operands::QWORDPTR(ptr_inner) => {
            let p = get_pointer_inner_addr(ptr_inner);
            let a = mem!(QWORD be STACK[p]);
            a as usize
        },
        Operands::Pointer(ptr_inner) => {
            get_pointer_inner_addr(ptr_inner)

        },
        _ => todo!(),
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
        Registers::E1 => reg!(E1),
        Registers::E2 => reg!(E2),
        Registers::E3 => reg!(E3),
        Registers::E4 => reg!(E4),
        Registers::E5 => reg!(E5),
        Registers::E6 => reg!(E6),
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


fn get_inner_ptr(start:&mut usize) -> Operands{
    let ctr = start;

    
    let endcode = mem!(PROGRAM[*ctr]);
    *ctr += 1;
    let operand1byte= mem!(PROGRAM[*ctr]);
    let operand1 = OperandType::from_u8(operand1byte.clone()).unwrap();
    let simpleoperand1 = SimpleOperands::from_operand(operand1.clone());
    let a1 = match simpleoperand1 {
        SimpleOperands::Reg => {
            *ctr +=1;
            InnerTypes::REG(Registers::from_operand(operand1))
        },
        SimpleOperands::Static => {
            *ctr +=1;
            let len: u8 = mem!(PROGRAM[*ctr]);
            if len == 0{
                *ctr += 1;
                InnerTypes::Static(0)
            }else{
                *ctr +=1;
                let a = info_program!(asv PROGRAM *ctr,len as usize);
                *ctr += len as usize;
                let a = as_be_bytes!(usize a);
                let a = usize::from_be_bytes(a);
                InnerTypes::Static(a)
            }
        },
        _ => todo!(),
    };
    let isnext = mem!(PROGRAM[*ctr]);
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
    let operator = mem!(PROGRAM[*ctr]);
    let sum = operator == b'+';
    *ctr +=1;
    let operand2byte= mem!(PROGRAM[*ctr]);
    let operand2 = OperandType::from_u8(operand2byte.clone()).unwrap();
    let simpleoperand2 = SimpleOperands::from_operand(operand2.clone());

    

    let a2: InnerTypes = match simpleoperand2 {
        SimpleOperands::Reg => {
            *ctr +=1;
            InnerTypes::REG(Registers::from_operand(operand2))
        },
        SimpleOperands::Static => {
            *ctr +=1;
            let len: u8 = mem!(PROGRAM[*ctr]);
            if len == 0{
                *ctr += 1;
                InnerTypes::Static(0)
            }else{
                *ctr +=1;
                let a = info_program!(asv PROGRAM *ctr,len as usize);
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
