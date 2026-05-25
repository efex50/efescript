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
pub(crate) enum PreCompile {
    I(Instuction),
    L(String),
}


macro_rules! ifeof {
    ($a:ident) => {{
        match if_eof(&$a) {
            Err(s) => return Err(s),
            Ok(()) => (),
        };
    }};
}


impl Instuction {
    pub fn get_len(&self) -> usize {
        let mut len = 1;
        if self.operandl != Operands::Null {
            len += self.operandl.to_int().len();
        }
        if self.operandr != Operands::Null {
            len += self.operandr.to_int().len();
        }
        len
    }

    pub fn get_program(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(self.opcode.to_u8().unwrap());
        if self.operandl != Operands::Null {
            v.append(&mut self.operandl.to_int());
        }
        if self.operandr != Operands::Null {
            v.append(&mut self.operandr.to_int());
        }
        v
    }
}


#[allow(unused)]
#[derive(Debug)]
pub enum ParseErr {
    NoData,
    WrongToken(LineColmn),
    WrongOpCode(String, LineColmn),
    WrongOperand(String, LineColmn),
    ErrLabelName(LineColmn),
    EOF,
    WrongPtrInner(String, LineColmn),
}


// ---------------------------------------------------------------------------
// Yardımcı fonksiyonlar — tekrar eden operand parse mantığı burada toplandı
// ---------------------------------------------------------------------------

/// Bir sonraki token'ı iterator'dan çeker, EOF kontrolü yapar ve `get_inner_str`
/// döndürmesi beklenen token'ın içeriğini string olarak verir.
fn next_ident<I>(iter: &mut I, pos_hint: LineColmn) -> Result<(String, LineColmn), ParseErr>
where
    I: Iterator<Item = LexerTokens>,
{
    let tok = iter.next().unwrap();
    if_eof(&tok)?;
    match tok.token_type.get_inner_str() {
        Some(s) => Ok((s.to_string(), tok.pos.start)),
        None => Err(ParseErr::WrongOpCode(format!("{:?}", tok.token_type), tok.pos.start)),
    }
}

/// `[reg + offset]` veya `[addr]` gibi pointer iç token listesini toplar ve
/// `PtrInner`'a dönüştürür.
fn parse_ptr_inner<I>(iter: &mut I, open_brl_pos: LineColmn) -> Result<PtrInner, ParseErr>
where
    I: Iterator<Item = LexerTokens>,
{
    let mut innertokens = Vec::new();
    loop {
        let tt = iter.next().unwrap();
        if_eof(&tt)?;
        if tt.token_type == LexerTokenType::RSPar && innertokens.is_empty()
            || innertokens.len() > 3
        {
            return Err(ParseErr::WrongPtrInner(
                "No address in pointer".to_string(),
                open_brl_pos,
            ));
        } else if tt.token_type == LexerTokenType::RSPar {
            break;
        } else {
            innertokens.push(tt);
        }
    }
    PtrInner::from_tokens(innertokens)
}

/// `OperandType`'a göre `Operands` üretir.
/// Pointer türleri için `[` bekleyip iç token'ları parse eder.
fn build_operand<I>(
    iter: &mut I,
    tok: &LexerTokens,
    typel: OperandType,
) -> Result<Operands, ParseErr>
where
    I: Iterator<Item = LexerTokens>,
{
    match typel {
        OperandType::Static => {
            let str = tok.token_type.get_inner_str().unwrap();
            Ok(Operands::Static(as_usize(str).unwrap()))
        }
        OperandType::RA
        | OperandType::RB
        | OperandType::RC
        | OperandType::RD
        | OperandType::RBP
        | OperandType::RSP
        | OperandType::R1
        | OperandType::R2
        | OperandType::R3
        | OperandType::R4
        | OperandType::R5
        | OperandType::R6
        | OperandType::AL
        | OperandType::AH
        | OperandType::BL
        | OperandType::BH
        | OperandType::CL
        | OperandType::CH
        | OperandType::DL
        | OperandType::DH => Ok(parse_register_type_to_op(typel).unwrap()),

        OperandType::Pointer
        | OperandType::BYTEPTR
        | OperandType::WORDPTR
        | OperandType::DWORDPTR
        | OperandType::QWORDPTR => {
            let brl = iter.next().unwrap();
            if_eof(&brl)?;
            if brl.token_type != LexerTokenType::LSPar {
                return Err(ParseErr::WrongToken(brl.pos.start));
            }
            let inner = parse_ptr_inner(iter, brl.pos.start)?;
            let operand = match typel {
                OperandType::Pointer  => Operands::Pointer(inner),
                OperandType::BYTEPTR  => Operands::BYTEPTR(inner),
                OperandType::WORDPTR  => Operands::WORDPTR(inner),
                OperandType::DWORDPTR => Operands::DWORDPTR(inner),
                OperandType::QWORDPTR => Operands::QWORDPTR(inner),
                _ => unreachable!(),
            };
            Ok(operand)
        }

        OperandType::String => todo!(),
        OperandType::NULL   => todo!(),
    }
}

/// Token'dan operand tipi çıkarır; sayısal literal'leri `Operands::Static`
/// olarak ele alır.
fn parse_operand_from_token<I>(
    iter: &mut I,
    tok: &LexerTokens,
) -> Result<Operands, ParseErr>
where
    I: Iterator<Item = LexerTokens>,
{
    // Token doğrudan sayı içeriyorsa (ör. `42`)
    if let Some(n) = tok.token_type.get_inner_num() {
        return Ok(Operands::Static(*n as usize));
    }

    let s = tok
        .token_type
        .get_inner_str()
        .ok_or_else(|| ParseErr::WrongOpCode(format!("{:?}", tok.token_type), tok.pos.start.clone()))?;

    let operand_type = parse_operand_types(s.to_lowercase())
        .map_err(|_| ParseErr::WrongOperand(s.to_string(), tok.pos.start.clone()))?;

    build_operand(iter, tok, operand_type)
}


// ---------------------------------------------------------------------------

fn parse_str_to_instructions(str: String) -> Result<Vec<PreCompile>, ParseErr> {
    let tokens = LexerTokens::trim_spaces(LexerTokens::string_to_token(&str));
    let mut program: Vec<PreCompile> = Vec::new();
    let mut iter_t = tokens.into_iter();

    loop {
        let tok = iter_t.next().unwrap();
        dbg!(&tok);
        match tok.token_type {
            LexerTokenType::EOL => {}
            LexerTokenType::EOF => return Ok(program),

            LexerTokenType::Ident(ident) => {
                let ident = ident.to_lowercase();

                if ident == "label" {
                    let label = iter_t.next().unwrap();
                    if_eof(&label)?;
                    if label.token_type.get_inner_str().is_none() {
                        return Err(ParseErr::ErrLabelName(label.pos.start));
                    }
                    program.push(PreCompile::L(
                        label.token_type.get_inner_str_owned().unwrap(),
                    ));
                } else if let Some(opcode) = parse_opcode(&ident) {
                    match opcode {
                        // İki operandlı komutlar
                        OpCodes::Mov
                        | OpCodes::Add8
                        | OpCodes::Add16
                        | OpCodes::Add32
                        | OpCodes::Add64
                        | OpCodes::Sub8
                        | OpCodes::Sub16
                        | OpCodes::Sub32
                        | OpCodes::Sub64
                        | OpCodes::Or
                        | OpCodes::Xor
                        | OpCodes::And
                        | OpCodes::Lea
                        | OpCodes::Cmp
                        | OpCodes::Nand
                        | OpCodes::Nor => {
                            // Sol operand
                            let ltok = iter_t.next().unwrap();
                            if_eof(&ltok)?;
                            if ltok.token_type.get_inner_str().is_none() {
                                return Err(ParseErr::WrongOpCode(
                                    format!("{:?}", ltok.token_type),
                                    ltok.pos.start,
                                ));
                            }
                            let operandl = parse_operand_from_token(&mut iter_t, &ltok)?;

                            // Virgül / ayraç token'ını tüket
                            if_eof(&iter_t.next().unwrap())?;

                            // Sağ operand
                            let rtok = iter_t.next().unwrap();
                            if_eof(&rtok)?;
                            let operandr = parse_operand_from_token(&mut iter_t, &rtok)?;

                            program.push(PreCompile::I(Instuction {
                                opcode,
                                operandl,
                                operandr,
                            }));
                        }

                        // Tek operandlı komutlar
                        OpCodes::Push8
                        | OpCodes::Push16
                        | OpCodes::Push32
                        | OpCodes::Push64
                        | OpCodes::Pop8
                        | OpCodes::Pop16
                        | OpCodes::Pop32
                        | OpCodes::Pop64
                        | OpCodes::Test => {
                            let ltok = iter_t.next().unwrap();
                            if_eof(&ltok)?;
                            if ltok.token_type.get_inner_str().is_none() {
                                return Err(ParseErr::ErrLabelName(ltok.pos.start));
                            }
                            dbg!("sa");
                            let operandl = parse_operand_from_token(&mut iter_t, &ltok)?;

                            program.push(PreCompile::I(Instuction {
                                opcode,
                                operandl,
                                operandr: Operands::Null,
                            }));
                        }

                        // Etiket alan jump/call komutları
                        OpCodes::Jmp
                        | OpCodes::Je
                        | OpCodes::Jne
                        | OpCodes::Jgt
                        | OpCodes::Jlt
                        | OpCodes::Jle
                        | OpCodes::Jge
                        | OpCodes::Jz
                        | OpCodes::Jnz
                        | OpCodes::Js
                        | OpCodes::Call => {
                            let label = iter_t.next().unwrap();
                            if_eof(&label)?;
                            if label.token_type.get_inner_str().is_none() {
                                return Err(ParseErr::ErrLabelName(label.pos.start));
                            }
                            program.push(PreCompile::I(Instuction {
                                opcode,
                                operandl: Operands::Label(
                                    label.token_type.get_inner_str_owned().unwrap(),
                                ),
                                operandr: Operands::Null,
                            }));
                        }

                        // Operandsız komutlar
                        OpCodes::Ret | OpCodes::SysCall | OpCodes::Nop => {
                            program.push(PreCompile::I(Instuction {
                                opcode,
                                operandl: Operands::Null,
                                operandr: Operands::Null,
                            }));
                        }
                    }
                } else {
                    return Err(ParseErr::WrongOpCode(ident, tok.pos.start));
                }
            }

            LexerTokenType::SemiColon => {
                'ine: loop {
                    let a = iter_t.next().unwrap();
                    if a.token_type == LexerTokenType::EOF {
                        return Ok(program);
                    }
                    if a.token_type == LexerTokenType::EOL {
                        break 'ine;
                    }
                }
            }

            LexerTokenType::Number(_) | _ => {
                return Err(ParseErr::WrongToken(tok.pos.start));
            }
        };
    }
}


// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, ToPrimitive, FromPrimitive, Clone)]
pub(crate) enum OperandType {
    Static   = 0,
    RA       = 1,
    RB,
    RC,
    RD,
    RBP,
    RSP      = 6,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6       = 12,
    AL,
    AH,
    BL,
    BH,
    CL,
    CH,
    DL,
    DH,
    Pointer  = 0x16,
    BYTEPTR,
    WORDPTR,
    DWORDPTR,
    QWORDPTR,
    String,
    NULL     = 0xff,
}


fn parse_operand_types<S: Into<String>>(data: S) -> Result<OperandType, ()> {
    let d: String = data.into();
    match d.as_str() {
        "ra"       => Ok(OperandType::RA),
        "rb"       => Ok(OperandType::RB),
        "rc"       => Ok(OperandType::RC),
        "rd"       => Ok(OperandType::RD),
        "rsp"      => Ok(OperandType::RSP),
        "rbp"      => Ok(OperandType::RBP),
        "r1"       => Ok(OperandType::R1),
        "r2"       => Ok(OperandType::R2),
        "r3"       => Ok(OperandType::R3),
        "r4"       => Ok(OperandType::R4),
        "r5"       => Ok(OperandType::R5),
        "r6"       => Ok(OperandType::R6),
        "al"       => Ok(OperandType::AL),
        "ah"       => Ok(OperandType::AH),
        "bl"       => Ok(OperandType::BL),
        "bh"       => Ok(OperandType::BH),
        "cl"       => Ok(OperandType::CL),
        "ch"       => Ok(OperandType::CH),
        "dl"       => Ok(OperandType::DL),
        "dh"       => Ok(OperandType::DH),
        "byteptr"  => Ok(OperandType::BYTEPTR),
        "wordptr"  => Ok(OperandType::WORDPTR),
        "dwordptr" => Ok(OperandType::DWORDPTR),
        "qwordptr" => Ok(OperandType::QWORDPTR),
        _          => as_usize(d).map(|_| OperandType::Static),
    }
}


fn if_eof(tok: &LexerTokens) -> Result<(), ParseErr> {
    if tok.token_type == LexerTokenType::EOF {
        Err(ParseErr::EOF)
    } else {
        Ok(())
    }
}


// ---------------------------------------------------------------------------

pub fn parse_str<S: Into<String>>(str: S) -> Result<Vec<u8>, ParseErr> {
    let inst = parse_str_to_instructions(str.into())?;

    let mut data = nasm_compiler::Data::from_pre_compile(inst);
    data.r1();
    data.r2();
    data.r2();

    Ok(data.to_porgram())
}

pub fn parse_from_file<S: Into<String>>(f: S) -> Result<Vec<u8>, ParseErr> {
    let path_str = f.into();
    let path = std::path::Path::new(&path_str);
    let v = std::fs::read(path).unwrap();
    parse_str(String::from_utf8(v).unwrap())
}


#[cfg(test)]
mod tests {
    use super::parse_str_to_instructions;

    #[test]
    fn label_test() {
        let s = "label sadfsafasf".to_string();
        parse_str_to_instructions(s);
    }
}