use std::collections::HashMap;

use num_traits::ToPrimitive;

use crate::{funs::trim_zeroes, instruction::Instuction, ops::OpCodes};

use super::PreCompile;



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
    pub(crate)  fn from_pre_compile(a:Vec<PreCompile>) -> Self{
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
                        OpCodes::Call |
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
