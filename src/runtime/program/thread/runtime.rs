use crate::{as_be_bytes, instruction::Operands, runtime::{data_funs::get_inner_ptr,Instuction}};
use num_traits::FromPrimitive;

use crate::{ops::OpCodes, runtime::{parse_register_type_to_op, OperandType, SimpleOperands}};

use super::PThread;


impl PThread{
    fn get_operand(&mut self) -> Option<Operands>{

        let prg = unsafe {
            &mut *self.program
        };

        let l = prg.read(self.counter, 1).try_into().unwrap();
        let l = u8::from_be_bytes(l);

        

        let lo = OperandType::from_u8(l).unwrap();
        

        let l = SimpleOperands::from_operand(lo.clone());
        let op = match l {
            SimpleOperands::Reg => {
                let ll = parse_register_type_to_op(lo).unwrap();
                ll
            }, 
            SimpleOperands::Static => {
                self.counter +=1;

                let len = prg.read(self.counter, 1).try_into().unwrap();
                let len = u8::from_be_bytes(len);
    
                if len == 0 {
                    Operands::Static(0)
                }else{
                    self.counter +=1;
                    let a = prg.read(self.counter, len as usize);
                    let a = as_be_bytes!(usize a);
                    let a = usize::from_be_bytes(a);
                    self.counter += len as usize -1;
                    Operands::Static(a)
                }
            },
            SimpleOperands::String => todo!(),
            SimpleOperands::Ptr => {
                let innerptr = get_inner_ptr(&mut self.counter,prg);
                
                
                innerptr                    
            },
            SimpleOperands::Nop => {Operands::Null},
            SimpleOperands::PushPop => {
                todo!()
            },
        };
        Some(op)

    } 

    fn get_instruction(&mut self) -> Option<Instuction>{
        let prg = unsafe {
            &mut *self.program
        };
        let op = prg.read(self.counter, 1).try_into().unwrap();
        

        let op = u8::from_be_bytes(op);
    
        if op == 0 {return None;}
        let code = match OpCodes::from_u8(op){
            Some(some) => some,
            None => {
                dbg!(op,self.counter);
                eprintln!("{:?}",self.program);
                panic!("none çıktı usta");
            },
        };
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
                self.counter += 1;
                //left
                let lop = self.get_operand().unwrap();
                
                self.counter += 1;
                //left
                let rop = self.get_operand().unwrap();
                self.counter += 1;
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
                self.counter += 1;
                //left
                let lop = self.get_operand().unwrap();
                self.counter += 1;
                return Some(Instuction{
                    opcode:code,
                    operandl:lop,
                    operandr:Operands::Null
                })        
            },
            OpCodes::Ret |
            OpCodes::Nop |
            OpCodes::SysCall => {
                self.counter +=1;
                return Some(Instuction{
                    opcode:code,
                    operandl:Operands::Null,
                    operandr:Operands::Null,
                });
    
            },
            OpCodes::Lea => todo!(),
        }
    }

    /// runs the program untill end
    pub fn start(&mut self){
        loop {
            match self.tick() {
                Some(_) => (),
                None => break,
            }
        }

    } 

    /// process 1 instruction
    pub fn tick(&mut self) -> Option<()>{
        let i = self.get_instruction();
        let i = match i {
            Some(some) => {
                some
            },
            None => return None,
        };
        self.handle_opcodes(&i);
        return Some(());
    }
}

