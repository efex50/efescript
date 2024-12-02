pub(super)  mod handle_syscall;
pub(super)  mod handle_opcodes;
pub(super)  mod data_funs;
pub(super)  mod program;

use crate::*;

use efearena::Arena;
use data_funs::get_inner_ptr;
use nasm_efe::{parse_register_type_to_op, Instuction, OperandType, Operands, SimpleOperands,};
use num_traits::FromPrimitive;
use ops::OpCodes;



#[derive(Debug)]
pub struct ProgramRuntime{
    pub program:Arena,
    pub stack:Arena,
}
impl ProgramRuntime{
    pub fn new() -> Self{
        Self{
            program:Arena::new(),
            stack:Arena::new()
        }
    }
    pub fn load_from_file<S:Into<String>>(&mut self,a:S){
        let path:String = a.into();
        let path = std::path::Path::new(&path);
        let v = std::fs::read(path).unwrap();
        self.program.write(0, v);
    }
    pub fn load_from_vec(&mut self,a:Vec<u8>){
        self.program.write(0, a);
    }

    pub fn load_from_vec_new(&mut self){

    }

    pub fn print_slice(&mut self,start:usize,len:usize) -> String{
        let v = self.program.read(start, len);
        format!("{:?}",v)
    }


    fn get_operand(&mut self,mut counter:&mut usize) -> Option<Operands>{

        let l = self.program.read(*counter, 1).try_into().unwrap();
        let l = u8::from_be_bytes(l);

        

        let lo = OperandType::from_u8(l).unwrap();
        

        let l = SimpleOperands::from_operand(lo.clone());
        let op = match l {
            SimpleOperands::Reg => {
                let ll = parse_register_type_to_op(lo).unwrap();
                ll
            }, 
            SimpleOperands::Static => {
                *counter +=1;

                let len = self.program.read(*counter, 1).try_into().unwrap();
                let len = u8::from_be_bytes(len);
    
                if len == 0 {
                    Operands::Static(0)
                }else{
                    *counter +=1;
                    let a = self.program.read(*counter, len as usize);
                    let a = as_be_bytes!(usize a);
                    let a = usize::from_be_bytes(a);
                    *counter += len as usize -1;
                    Operands::Static(a)
                }
            },
            SimpleOperands::String => todo!(),
            SimpleOperands::Ptr => {
                let innerptr = get_inner_ptr(&mut counter,&mut self.program);
                
                
                innerptr                    
            },
            SimpleOperands::Nop => {Operands::Null},
            SimpleOperands::PushPop => {
                todo!()
            },
        };
        Some(op)

    } 

    fn get_instruction(&mut self,counter:&mut usize) -> Option<Instuction>{
        let op = self.program.read(*counter, 1).try_into().unwrap();
        let op = u8::from_be_bytes(op);
    
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
            OpCodes::Cmp  => {
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
            OpCodes::Test |
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

    pub fn start(&mut self){
        let mut counter = 0;
        loop {
            let i = self.get_instruction(&mut counter);
            let i = match i {
                Some(some) => {
                    some
                },
                None => break,
            };
            handle_opcodes::handle_opcodes(&i,&mut counter,&mut self.stack);
        }

    } 
}

