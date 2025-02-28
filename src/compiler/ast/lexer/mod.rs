

#[derive(Clone,Debug)]
struct Position{
    start:LineColmn,
    end:LineColmn
}
#[derive(Clone,Debug)]
struct LineColmn{
    line: u64,
    colmn:u64
}

#[repr(u8)]
#[derive(Debug)]
enum LexerTokenType{
    Number(i128),
    Ident( String),
    String(String),
    
    //  operator signs

    /// sign +
    Plus,
    /// sign -
    Minus,
    /// sign *
    Star,
    /// sign /
    Slash,
    /// sign ^
    Exponent,
    /// sign %
    Percent,
    /// sign &
    And,
    /// sing |
    Or,
    /// sign !
    Exclamation,
    
    
    /// sign '
    Quote,
    /// sign "
    DQuote,
    /// sign `
    TildeQuote,
    /// sign ~
    Tilde,
    /// sign )
    RPar,
    /// sign (
    LPar,
    /// sign ,
    Punc,
    /// sign .
    Dot,
    /// sign {
    LCPar,
    /// sign }
    RCPar,
    /// sign =
    Equal,
    /// sign ;
    SemiColon,
    /// sign //
    Comment,
    /// sign /*
    MCommentStart,
    /// sign */
    MCommentEnd,


    /// end of line
    EOL,
    /// end of file
    EOF,
}
#[derive(Debug)]
pub struct LexerTokens{
    token_type:LexerTokenType,
    pos:Position
} 

impl LexerTokens {
    pub fn str_to_token<S:Into<String>>(str:S) -> Vec<Self>{
        let str:String = str.into();
        let tokens = Self::to_token_inner(&str);
        return tokens; 
    }
    pub fn string_to_token(str:&String) -> Vec<Self>{
        let tokens = Self::to_token_inner(str);
        return tokens; 
    }

    fn new(start:LineColmn,end:LineColmn,tok_type:LexerTokenType) -> Self{
        return Self {
            token_type: tok_type,
            pos: Position { start, end } 
        };
    }

    fn to_token_inner(str:&String) -> Vec<LexerTokens>{
        
        
        let mut str = str.chars();

        let mut counter = 0;
        let (mut line,mut column) = (1u64,1u64);
        let mut tokens = Vec::<LexerTokens>::new();
        let start = LineColmn { line, colmn : column };
        loop{
            let x = str.next();
            if x.is_none(){
                tokens.push(LexerTokens::new(start.clone(), start.clone(),  LexerTokenType::EOF));
                return tokens;
            }
            let char = x.unwrap();
            
            counter += 1;
            println!("counter: {counter} ={:?}",x);
            let token = match char {
                '+' => LexerTokens{token_type:LexerTokenType::Plus,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '-' => LexerTokens{token_type:LexerTokenType::Minus,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '*' => LexerTokens{token_type:LexerTokenType::Star,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},

                // should handle the comment case
                '/' => LexerTokens{token_type:LexerTokenType::Slash,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '^' => LexerTokens{token_type:LexerTokenType::Exponent,pos:Position     { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '%' => LexerTokens{token_type:LexerTokenType::Percent,pos:Position      { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '&' => LexerTokens{token_type:LexerTokenType::And,pos:Position          { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '|' => LexerTokens{token_type:LexerTokenType::Or,pos:Position           { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '!' => LexerTokens{token_type:LexerTokenType::Exclamation,pos:Position  { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                
                '(' => LexerTokens{token_type:LexerTokenType::LPar,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                ')' => LexerTokens{token_type:LexerTokenType::RPar,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '{' => LexerTokens{token_type:LexerTokenType::RCPar,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '}' => LexerTokens{token_type:LexerTokenType::LCPar,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '.' => LexerTokens{token_type:LexerTokenType::Dot,pos:Position          { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                ',' => LexerTokens{token_type:LexerTokenType::Punc,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '=' => LexerTokens{token_type:LexerTokenType::Equal,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '~' => LexerTokens{token_type:LexerTokenType::Tilde,pos:Position        { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                ';' => LexerTokens{token_type:LexerTokenType::SemiColon,pos:Position    { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                '\n' => LexerTokens{token_type:LexerTokenType::EOL,pos:Position         { start: LineColmn { line: line, colmn: column }, end: LineColmn { line: line, colmn: column+1 } }},
                
                '"' => {
                    get_str(str,'"',&mut line,&mut column);
                    todo!()
                },
                '´' => {
                    get_str(str,'`',&mut line,&mut column);
                    todo!()
                },
                '\'' => {
                    get_str(str,'\'',&mut line,&mut column);
                    todo!()
                },

                ' ' => {
                    column+=1;
                    continue;
                }
                _ => {
                    eprint!("error at :{}",char);
                    todo!("not yet")
                }  
            };
            match token.token_type {
                LexerTokenType::Plus |
                LexerTokenType::Minus |
                LexerTokenType::Star |
                LexerTokenType::Slash |
                LexerTokenType::Exponent |
                LexerTokenType::Percent |
                LexerTokenType::And |
                LexerTokenType::Or |
                LexerTokenType::Exclamation |
                LexerTokenType::Tilde |
                LexerTokenType::RPar |
                LexerTokenType::LPar |
                LexerTokenType::Punc |
                LexerTokenType::Dot |
                LexerTokenType::LCPar |
                LexerTokenType::RCPar |
                LexerTokenType::Equal |
                LexerTokenType::SemiColon =>{column+=1;tokens.push(token);},
                LexerTokenType::EOL => {column=1;line+=1;tokens.push(token);},
                
                // strings
                LexerTokenType::Quote |
                LexerTokenType::DQuote |
                LexerTokenType::TildeQuote =>{
                    panic!("strings are not yet implemented")
                }
                
                LexerTokenType::Comment |
                LexerTokenType::MCommentStart |
                LexerTokenType::MCommentEnd => (),

                LexerTokenType::EOF => break,
                _ => (),
            }
        }

        return tokens;
    }
}


fn get_str(mut chars:impl Iterator<Item = char> , end_token:char,line:&mut u64,column:&mut u64)->Option<String>{
    let mut str = String::new();
    loop {

        if let Some(c) = chars.next(){
            if c == end_token{
                return Some(str);
            }
        }else {
            return None;
        }

    }

}





mod test{
    use crate::compiler::ast::lexer::LexerTokens;
    #[test]
    fn test_tokenize(){
        let s = 
        "
        void main(){
            int a = 10;
        }
        ";
        let ss = 
        "
        ..,()^+
        .....";
        let toks = LexerTokens::str_to_token(ss);
        println!("{:#?}",toks)
    }
}