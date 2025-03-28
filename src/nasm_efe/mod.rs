use crate::funs::as_usize;
use crate::instruction::{Instuction, Operands, PtrInner};
use crate::compiler::ast::lexer::{LexerTokenType, LexerTokens, LineColmn};
use crate::nasm_efe::nasm_funs::{parse_opcode, parse_register_type_to_op};
use crate::ops::OpCodes;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive;

pub mod nasm_compiler;
pub mod nasm_funs;




#[derive(Debug)]
pub(crate) enum PreCompile{
    I(Instuction),
    L(String)
}


macro_rules! ifeof {
    ($a:ident) => {
        {
            // handle the unexpected EOF
            match if_eof(&$a){
                Err(s) => return Err(s),
                Ok(()) => (),
            };
        }
    };
}


impl Instuction {
    
    pub fn get_len(&self) -> usize {
        let mut len = 1 ;
        if self.operandl != Operands::Null{
            let b = self.operandl.to_int();
            len += b.len();
        }
        if self.operandr != Operands::Null{
            let b = self.operandr.to_int();
            len += b.len();
        }
        len
    }
    pub fn get_program(&self) -> Vec<u8> {
        let mut v = Vec::new() ;
        v.push(self.opcode.to_u8().unwrap());
        if self.operandl != Operands::Null{
            let mut b = self.operandl.to_int();
            v.append(&mut b);
        }
        if self.operandr != Operands::Null{
            let mut b = self.operandr.to_int();
            v.append(&mut b);            
        }
        v
    }
}

#[allow(unused)]
#[derive(Debug)]
pub enum ParseErr{
    NoData,
    WrongToken(LineColmn),
    WrongOpCode(String,LineColmn),
    WrongOperand(String,LineColmn),
    ErrLabelName(LineColmn),
    EOF,
    WrongPtrInner(LineColmn),

}


fn parse_str_to_instructions(str:String) -> Result<Vec<PreCompile>,ParseErr> {
    let tokens = LexerTokens::trim_spaces(LexerTokens::string_to_token(&str));
    let mut program: Vec<PreCompile> = Vec::new();
    let mut iter_t = tokens.into_iter();
    loop {
        let tok = iter_t.next().unwrap();
        dbg!(&tok);
        match tok.token_type {
            LexerTokenType::EOL => {dbg!("eol");} ,
            LexerTokenType::EOF => {return Ok(program);},
            LexerTokenType::Ident(ident) => {
                let ident = ident.to_lowercase();
                if ident == "label"{
                    let label = iter_t.next().unwrap();
                    // handle the unexpected EOF
                    if_eof(&label)?;
                    
                    if label.token_type.get_inner_str().is_none(){
                        return Err(ParseErr::ErrLabelName(label.pos.start));
                    }
                    let tag = label.token_type.get_inner_str_owned().unwrap();
                    program.push(PreCompile::L(tag));
                
                }else if let Some(opcode) = parse_opcode(&ident) {
                    match opcode {
                        // two operand instruction
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
                        OpCodes::Lea |
                        OpCodes::Cmp |
                        OpCodes::Nand |
                        OpCodes::Nor => {
                            let label = iter_t.next().unwrap();
                            // handle the unexpected EOF
                            if_eof(&label)?;

                            
                            
                            if label.token_type.get_inner_str().is_none(){
                                return Err(ParseErr::WrongOpCode(format!("{:?}",label.token_type),label.pos.start));
                            }
                            
                            let typel = parse_operand_types(label.token_type.get_inner_str().unwrap().to_lowercase())
                                .map_err(|_| ParseErr::WrongOperand(label.token_type.get_inner_str_owned().unwrap(), label.pos.start))?;
                            
                            
                            let operandl = match typel {
                                OperandType::Static => {
                                    Operands::Static(as_usize(&ident).unwrap())
                                },
                                OperandType::RA |
                                OperandType::RB |
                                OperandType::RC |
                                OperandType::RD |
                                OperandType::RBP |
                                OperandType::RSP |
                                OperandType::R1 |
                                OperandType::R2 |
                                OperandType::R3 |
                                OperandType::R4 |
                                OperandType::R5 |
                                OperandType::R6 |
                                OperandType::AL |
                                OperandType::AH |
                                OperandType::BL |
                                OperandType::BH |
                                OperandType::CL |
                                OperandType::CH |
                                OperandType::DL |
                                OperandType::DH => {
                                    parse_register_type_to_op(typel).unwrap()
                                }
                                OperandType::Pointer |
                                OperandType::BYTEPTR |
                                OperandType::WORDPTR |
                                OperandType::DWORDPTR |
                                OperandType::QWORDPTR => {
                                    let brl = iter_t.next().unwrap();
                                    // handle the unexpected EOF
                                    if_eof(&brl)?;
                                
                                    if brl.token_type != LexerTokenType::LSPar{
                                        return Err(ParseErr::WrongToken(brl.pos.start));
                                    }
                                    let mut innertokens = Vec::new();
                                    loop {
                                        let tt = iter_t.next().unwrap();
                                        // handle the unexpected EOF
                                        if_eof(&tt)?;
                                        if tt.token_type == LexerTokenType::RSPar && innertokens.len() < 1 || innertokens.len() > 3{
                                            return Err(ParseErr::WrongPtrInner(brl.pos.start));
                                        }else if tt.token_type == LexerTokenType::RSPar {
                                            break;
                                        }else {
                                            innertokens.push(tt);
                                        }
                                    }
                                    let inner = PtrInner::from_tokens(innertokens)?;
                                    match typel {
                                        OperandType::Pointer => {
                                            Operands::Pointer(inner)
                                        },
                                        OperandType::BYTEPTR => {
                                            Operands::BYTEPTR(inner)
                                        },
                                        OperandType::WORDPTR => {
                                            Operands::WORDPTR(inner)                                            
                                        },
                                        OperandType::DWORDPTR => {
                                            Operands::DWORDPTR(inner)
                                        },
                                        OperandType::QWORDPTR => {
                                            Operands::QWORDPTR(inner)
                                        },
                                        _=> panic!()
                                    }
                                }
                                OperandType::String => todo!(),
                                OperandType::NULL => todo!(),
                            };
                            if_eof(&iter_t.next().unwrap())?;
                            let label = iter_t.next().unwrap();
                            // handle the unexpected EOF
                            if_eof(&label)?;
                            let operandr = if label.token_type.get_inner_num().is_some(){
                                Operands::Static(*label.token_type.get_inner_num().unwrap() as usize)
                            }else if label.token_type.get_inner_str().is_none(){
                                return Err(ParseErr::WrongOpCode(format!("{:?}",label.token_type),label.pos.start));
                            }else{

                            
                                let typer = parse_operand_types(label.token_type.get_inner_str().unwrap().to_lowercase())
                                    .map_err(|_| ParseErr::WrongOperand(label.token_type.get_inner_str_owned().unwrap(), label.pos.start))?;

                                match typer {
                                    OperandType::Static => Operands::Static(as_usize(ident).unwrap()),
                                    OperandType::RA |
                                    OperandType::RB |
                                    OperandType::RC |
                                    OperandType::RD |
                                    OperandType::RBP |
                                    OperandType::RSP |
                                    OperandType::R1 |
                                    OperandType::R2 |
                                    OperandType::R3 |
                                    OperandType::R4 |
                                    OperandType::R5 |
                                    OperandType::R6 |
                                    OperandType::AL |
                                    OperandType::AH |
                                    OperandType::BL |
                                    OperandType::BH |
                                    OperandType::CL |
                                    OperandType::CH |
                                    OperandType::DL |
                                    OperandType::DH => {
                                        parse_register_type_to_op(typer).unwrap()
                                    }
                                    OperandType::Pointer |
                                    OperandType::BYTEPTR |
                                    OperandType::WORDPTR |
                                    OperandType::DWORDPTR |
                                    OperandType::QWORDPTR => {
                                        let brl = iter_t.next().unwrap();
                                        // handle the unexpected EOF
                                        if_eof(&brl)?;
                                    
                                        if brl.token_type != LexerTokenType::LSPar{
                                            return Err(ParseErr::WrongToken(brl.pos.start));
                                        }
                                        let mut innertokens = Vec::new();
                                        loop {
                                            let tt = iter_t.next().unwrap();
                                            // handle the unexpected EOF
                                            if_eof(&tt)?;
                                            if tt.token_type == LexerTokenType::RSPar && innertokens.len() < 1 || innertokens.len() > 3{
                                                return Err(ParseErr::WrongPtrInner(brl.pos.start));
                                            }else if tt.token_type == LexerTokenType::RSPar {
                                                break;
                                            }else {
                                                innertokens.push(tt);
                                            }
                                        }
                                        let inner = PtrInner::from_tokens(innertokens)?;
                                        match typer {
                                            OperandType::Pointer => {
                                                Operands::Pointer(inner)
                                            },
                                            OperandType::BYTEPTR => {
                                                Operands::BYTEPTR(inner)
                                            },
                                            OperandType::WORDPTR => {
                                                Operands::WORDPTR(inner)                                            
                                            },
                                            OperandType::DWORDPTR => {
                                                Operands::DWORDPTR(inner)
                                            },
                                            OperandType::QWORDPTR => {
                                                Operands::QWORDPTR(inner)
                                            },
                                            _=> panic!()
                                        }
                                    }
                                    OperandType::String => todo!(),
                                    OperandType::NULL => todo!(),
                                }
                            };
                                    
                            
                            let i = Instuction{
                                opcode:opcode,
                                operandl,
                                operandr,
                            };
                            program.push(PreCompile::I(i));

                        }


                        
                        // one operand instruction                        
                        OpCodes::Push8 |
                        OpCodes::Push16 |
                        OpCodes::Push32 |
                        OpCodes::Push64 |
                        OpCodes::Pop8 |
                        OpCodes::Pop16 |
                        OpCodes::Pop32 |
                        OpCodes::Pop64 |
                        OpCodes::Test => {
                            let label = iter_t.next().unwrap();
                            dbg!(&label);
                            // handle the unexpected EOF
                            if_eof(&label)?;
                            
                            
                            if label.token_type.get_inner_str().is_none(){
                                return Err(ParseErr::ErrLabelName(label.pos.start));
                            }
                            let operandtag = label.token_type.get_inner_str().unwrap().clone();
                            
                            let typel = parse_operand_types(&operandtag.to_lowercase())
                                .map_err(|_| ParseErr::WrongOperand(label.token_type.get_inner_str_owned().unwrap(), label.pos.start))?;

                            dbg!(&typel,&ident);
                            let operandl = match typel {
                                OperandType::Static => Operands::Static(as_usize(operandtag).unwrap()),
                                OperandType::RA |
                                OperandType::RB |
                                OperandType::RC |
                                OperandType::RD |
                                OperandType::RBP |
                                OperandType::RSP |
                                OperandType::R1 |
                                OperandType::R2 |
                                OperandType::R3 |
                                OperandType::R4 |
                                OperandType::R5 |
                                OperandType::R6 |
                                OperandType::AL |
                                OperandType::AH |
                                OperandType::BL |
                                OperandType::BH |
                                OperandType::CL |
                                OperandType::CH |
                                OperandType::DL |
                                OperandType::DH => {
                                    parse_register_type_to_op(typel).unwrap()
                                }
                                OperandType::Pointer |
                                OperandType::BYTEPTR |
                                OperandType::WORDPTR |
                                OperandType::DWORDPTR |
                                OperandType::QWORDPTR => {
                                    let brl = iter_t.next().unwrap();
                                    // handle the unexpected EOF
                                    if_eof(&brl)?;

                                    if brl.token_type != LexerTokenType::LSPar{
                                        return Err(ParseErr::WrongToken(brl.pos.start));
                                    }
                                    let mut innertokens = Vec::new();
                                    loop {
                                        let tt = iter_t.next().unwrap();
                                        // handle the unexpected EOF
                                        if_eof(&tt)?;
                                        if tt.token_type == LexerTokenType::RSPar && innertokens.len() < 1 || innertokens.len() > 3{
                                            return Err(ParseErr::WrongPtrInner(brl.pos.start));
                                        }else if tt.token_type == LexerTokenType::RSPar {
                                            break;
                                        }else {
                                            innertokens.push(tt);
                                        }
                                    }
                                    let inner = PtrInner::from_tokens(innertokens)?;
                                    match typel {
                                        OperandType::Pointer => {
                                            Operands::Pointer(inner)
                                        },
                                        OperandType::BYTEPTR => {
                                            Operands::BYTEPTR(inner)
                                        },
                                        OperandType::WORDPTR => {
                                            Operands::WORDPTR(inner)                                            
                                        },
                                        OperandType::DWORDPTR => {
                                            Operands::DWORDPTR(inner)
                                        },
                                        OperandType::QWORDPTR => {
                                            Operands::QWORDPTR(inner)
                                        },
                                        _=> panic!()
                                    }
                                }
                                OperandType::String => todo!(),
                                OperandType::NULL => todo!(),
                            };
                            
                            
                            let i = Instuction{
                                opcode:opcode,
                                operandl,
                                operandr:Operands::Null
                            };
                            program.push(PreCompile::I(i));
                        }


                        OpCodes::Jmp |
                        OpCodes::Je  |
                        OpCodes::Jne |
                        OpCodes::Jgt |
                        OpCodes::Jlt |
                        OpCodes::Jle |
                        OpCodes::Jge |
                        OpCodes::Jz  |
                        OpCodes::Jnz |
                        OpCodes::Js  |
                        OpCodes::Call => {
                            let label = iter_t.next().unwrap();
                            // handle the unexpected EOF
                            if_eof(&label)?;

                            
                            if label.token_type.get_inner_str().is_none(){
                                return Err(ParseErr::ErrLabelName(label.pos.start));
                            }
                            
                            let i = Instuction{
                                opcode:opcode,
                                operandl:Operands::Label(label.token_type.get_inner_str_owned().unwrap())
                                ,operandr:Operands::Null
                            };
                            program.push(PreCompile::I(i));
                        },
                        OpCodes::Ret |
                        OpCodes::SysCall |
                        OpCodes::Nop => {
                            let i = Instuction{
                                opcode:opcode,
                                operandl:Operands::Null,
                                operandr:Operands::Null,
                            };
                            program.push(PreCompile::I(i));
                            
                        },
                    }
                }else {
                    return Err(ParseErr::WrongOpCode(ident,tok.pos.start));
                }
            },
            LexerTokenType::SemiColon => {
                'ine :loop {
                    let a= iter_t.next().unwrap();
                    if a.token_type == LexerTokenType::EOF{
                        return Ok(program);
                    }
                    if a.token_type == LexerTokenType::EOL{
                        break 'ine;
                    }

                }
            },
            LexerTokenType::Number(_) |
            _ => {
                return Err(ParseErr::WrongToken(tok.pos.start));
            },
        };
    }

}




/// bytecode definition
#[repr(u8)]
#[derive(Debug, PartialEq, Eq,ToPrimitive,FromPrimitive,Clone)]
pub(crate) enum OperandType{
    Static = 0,
    RA = 1,
    RB,
    RC,
    RD,
    RBP,
    RSP = 6,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6 = 12,
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
    NULL = 0xff,
}


fn parse_operand_types<S:Into<String>>(data:S) -> Result<OperandType,()>{
    let d:String = data.into();
    match d.as_str() {
        "ra" => Ok(OperandType::RA),
        "rb" => Ok(OperandType::RB),
        "rc" => Ok(OperandType::RC),
        "rd" => Ok(OperandType::RD),
        "rsp" => Ok(OperandType::RSP),
        "rbp" => Ok(OperandType::RBP),
        "r1" => Ok(OperandType::R1),
        "r2" => Ok(OperandType::R2),
        "r3" => Ok(OperandType::R3),
        "r4" => Ok(OperandType::R4),
        "r5" => Ok(OperandType::R5),
        "r6" => Ok(OperandType::R6),
        "al" => Ok(OperandType::AL),
        "ah" => Ok(OperandType::AH),
        "bl" => Ok(OperandType::BL),
        "bh" => Ok(OperandType::BH),
        "cl" => Ok(OperandType::CL),
        "ch" => Ok(OperandType::CH),
        "dl" => Ok(OperandType::DL),
        "dh" => Ok(OperandType::DH),



        "byteptr" => Ok(OperandType::BYTEPTR), 
        "wordptr" => Ok(OperandType::WORDPTR), 
        "dwordptr" => Ok(OperandType::DWORDPTR), 
        "qwordptr" => Ok(OperandType::QWORDPTR), 
        _ => {
            as_usize(d)
                .map(|_a| OperandType::Static)
        }
    }
}


fn if_eof(tok:&LexerTokens) -> Result<(),ParseErr>{
    if tok.token_type == LexerTokenType::EOF{
        return Err(ParseErr::EOF) ;
    }
    else {
        return Ok(());
    }
}


// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
pub fn parse_str<S:Into<String>>(str:S) -> Result<Vec<u8>,ParseErr> {
    let str:String = str.into();
    let inst = parse_str_to_instructions(str)?;
    dbg!(&inst);


    let mut data = nasm_compiler::Data::from_pre_compile(inst);
    data.r1();
    data.r2();
    data.r2();

    let p = data.to_porgram();


    // let p = program.iter()
    //                 .filter_map(|a| if *a == 255{None} else{Some(*a)} )
    //                 .collect();
    // program;
    Ok(p)
}

pub fn parse_from_file<S:Into<String>>(f:S) -> Result<Vec<u8>,ParseErr> {
    let f = f.into();
    let path = std::path::Path::new(&f);
    let v = std::fs::read(path).unwrap();
    parse_str(String::from_utf8(v).unwrap())

}



#[cfg(test)]
mod tests{
    use super::parse_str_to_instructions;

    #[test]
    fn label_test(){
        let s = "label sadfsafasf".to_string();
        parse_str_to_instructions(s);
    }
}