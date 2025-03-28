use std::rc::Rc;



#[derive(Clone,Debug)]
pub struct Position{
    pub start:LineColmn,
    pub end:LineColmn
}
#[derive(Clone,Debug)]
pub struct LineColmn{
    pub line: u64,
    pub colmn:u64
}



#[repr(u8)]
#[derive(Debug,PartialEq,Clone)]
pub enum LexerTokenType{
    Number(i64),
    Ident (String),
    
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
    /// sign [
    LSPar,
    /// sign ]
    RSPar,
    /// sign =
    Equal,
    /// sign ;
    SemiColon,

    /// space
    Space,
    /// tab space
    TabSpace,


    /// end of line
    EOL,
    /// end of file
    EOF,
}

impl LexerTokenType {
    pub fn get_inner_str(&self) -> Option<&String> {
        match &self {
            LexerTokenType::Ident(s) => Some(s),
            _ => None
        }
    }
    pub fn get_inner_str_owned(self) -> Option<String> {
        match self {
            LexerTokenType::Ident(s) => Some(s),
            _ => None
        }
    }
    pub fn get_inner_num(&self) -> Option<&i64> {
        match self {
            LexerTokenType::Number(n) => Some(n),
            _ => None
        }
    }
}


#[derive(Debug,Clone)]
pub struct LexerTokens{
    pub token_type:LexerTokenType,
    pub pos:Position
} 

impl LexerTokens {

    pub fn len(&self) -> u64{
        let len = self.pos.end.colmn - self.pos.start.colmn;
        len
    }

    pub fn str_to_token<S:Into<String>>(str:S) -> Vec<Self>{
        let str:String = str.into();
        let tokens = Self::get_tokens_merged(&str);
        return tokens; 
    }
    pub fn string_to_token(str:&String) -> Vec<Self>{
        let tokens = Self::get_tokens_merged(str);
        return tokens; 
    }

    // does the space merging
    fn get_tokens_merged(str:&String) -> Vec<Self>{
        let tokens = Self::to_token_inner(str); 
        // println!("{:?}",tokens);
        let tokens = Self::merge_spaces(tokens);
        // println!("{:?}",tokens);
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

        let (mut line,mut column) = (1u64,1u64);

        //unsafe 
        let line_ptr = &mut line as *mut u64 ;
        let column_ptr = &mut column as *mut u64;
        //unsafe 

        let mut tokens = Vec::<LexerTokens>::new();
        
        
        let start = LineColmn { line, colmn : column };
        
        loop{
            let x = str.next();
            
            
            //----
            let get_sym_token = |char:char| -> Option<LexerTokens>{
    
                let token = match char {
                    '+'  => Some(LexerTokens{token_type:LexerTokenType::Plus,pos:Position         { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '-'  => Some(LexerTokens{token_type:LexerTokenType::Minus,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '*'  => Some(LexerTokens{token_type:LexerTokenType::Star,pos:Position         { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '^'  => Some(LexerTokens{token_type:LexerTokenType::Exponent,pos:Position     { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '%'  => Some(LexerTokens{token_type:LexerTokenType::Percent,pos:Position      { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '&'  => Some(LexerTokens{token_type:LexerTokenType::And,pos:Position          { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '|'  => Some(LexerTokens{token_type:LexerTokenType::Or,pos:Position           { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '!'  => Some(LexerTokens{token_type:LexerTokenType::Exclamation,pos:Position  { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '('  => Some(LexerTokens{token_type:LexerTokenType::LPar,pos:Position         { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    ')'  => Some(LexerTokens{token_type:LexerTokenType::RPar,pos:Position         { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '{'  => Some(LexerTokens{token_type:LexerTokenType::LCPar,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '}'  => Some(LexerTokens{token_type:LexerTokenType::RCPar,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '['  => Some(LexerTokens{token_type:LexerTokenType::LSPar,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    ']'  => Some(LexerTokens{token_type:LexerTokenType::RSPar,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '.'  => Some(LexerTokens{token_type:LexerTokenType::Dot,pos:Position          { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    ','  => Some(LexerTokens{token_type:LexerTokenType::Punc,pos:Position         { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '='  => Some(LexerTokens{token_type:LexerTokenType::Equal,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '~'  => Some(LexerTokens{token_type:LexerTokenType::Tilde,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    ';'  => Some(LexerTokens{token_type:LexerTokenType::SemiColon,pos:Position    { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '\n' => Some(LexerTokens{token_type:LexerTokenType::EOL,pos:Position          { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '\t' => Some(LexerTokens{token_type:LexerTokenType::TabSpace,pos:Position     { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '/'  => Some(LexerTokens{token_type:LexerTokenType::Slash,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '"'  => Some(LexerTokens{token_type:LexerTokenType::DQuote,pos:Position       { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '\'' => Some(LexerTokens{token_type:LexerTokenType::Quote,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    ' '  => Some(LexerTokens{token_type:LexerTokenType::Space,pos:Position        { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    '`'  => Some(LexerTokens{token_type:LexerTokenType::TildeQuote,pos:Position   { start: LineColmn { line, colmn: column }, end: LineColmn { line, colmn: column+1 } }}),
                    
                    _ => None,
                };
                
                token
            };
            //----
            let handle_sym_token_line = |tok_type:&LexerTokens| {
                if tok_type.token_type == LexerTokenType::EOL{
                    unsafe {
                        *column_ptr=1;
                        *line_ptr+=1;
                    }
                }
                //else if tok_type.token_type == LexerTokenType::Space {
                //    
                //}
                else {
                    unsafe {*column_ptr+=1};
                }
            };
            //----

            if x.is_none(){
                tokens.push(LexerTokens::new(start.clone(), start.clone(),  LexerTokenType::EOF));
                return tokens;
            }
            let char = x.unwrap();
            
            
            if let Some(lt) = get_sym_token(char){
                handle_sym_token_line(&lt);
                if lt.token_type == LexerTokenType::EOF{
                    tokens.push(lt);
                    break;
                }
                tokens.push(lt);
            }
            else {
                let mut v: Vec<char> = Vec::new();
                v.push(char);
                
                let start = unsafe {LineColmn{colmn:*column_ptr,line:*line_ptr} };
                loop {
                    let x = str.next();
                    if x.is_none(){
                        if !v.is_empty(){
                            let ident_s = v.into_iter().collect::<String>();
                            //println!("{}",ident_s);

                            let end = unsafe {LineColmn{colmn:*column_ptr,line:*line_ptr} };
                            let a = if let Ok(num) = i64::from_str_radix(&ident_s, 10){
                                LexerTokens::new(start.clone(), end, LexerTokenType::Number(num))
                            }
                            else {
                                LexerTokens::new(start.clone(), end, LexerTokenType::Ident(ident_s))
                            };
                            tokens.push(a);
    
                        }
                        tokens.push(LexerTokens::new(start.clone(), start.clone(),  LexerTokenType::EOF));
                        return tokens;
                    }
                    let char = x.unwrap();
        

                    if let Some(lt) = get_sym_token(char){
                        unsafe {*column_ptr.clone() += 1;}
                        let ident_s = v.into_iter().collect::<String>();
                        //println!("{}",ident_s);
                        let end = unsafe {LineColmn{colmn:*column_ptr,line:*line_ptr} };
                        let a = if let Ok(num) = i64::from_str_radix(&ident_s, 10){
                            LexerTokens::new(start.clone(), end, LexerTokenType::Number(num))
                        }
                        else {
                            LexerTokens::new(start.clone(), end, LexerTokenType::Ident(ident_s))
                        };
                        tokens.push(a);
                        let t = get_sym_token(char).unwrap();
                        handle_sym_token_line(&lt);
                        tokens.push(t);
                        break;
                    }else{
                        unsafe { *column_ptr.clone() += 1;}
                        v.push(char);
                    }
                }
            }


        }

        return tokens;
    }

    fn merge_spaces(v:Vec<Self>) -> Vec<Self>{
        let mut t = Vec::new();
        let mut iter = v.iter();
        loop{
            let x = iter.next();
            let x = x.unwrap();
            if x.token_type == LexerTokenType::EOF{
                t.push(x.clone());
                return t;
            }

            if x.token_type == LexerTokenType::Space {
                let start = x.pos.start.clone();
                let mut prev = x;
                loop {

                    // not checking because it will hit EOF eventually
                    let next = iter.next().unwrap();
                    //if next.is_none(){
                    //    break prev.pos.end.clone();
                    //}
                    //let next = next.unwrap();

                    if next.token_type != LexerTokenType::Space{    
                        let end = next.clone();
                        
                        t.push(LexerTokens::new(start, prev.pos.end.clone(), LexerTokenType::Space));
                        t.push(end);
                        if next.token_type == LexerTokenType::EOF{
                            return t;
                        }
                        break ;

                    }else {
                        
                        prev = next;
                    }
                };
                continue;
                
            }
            else if x.token_type == LexerTokenType::TabSpace {
                let start = x.pos.start.clone();
                let mut prev = x;
                loop {

                    // not checking because it will hit EOF eventually
                    let next = iter.next().unwrap();
                    //if next.is_none(){
                    //    break prev.pos.end.clone();
                    //}
                    //let next = next.unwrap();

                    if next.token_type != LexerTokenType::TabSpace{    
                        let end = next.clone();
                        
                        t.push(LexerTokens::new(start, prev.pos.end.clone(), LexerTokenType::TabSpace));
                        t.push(end);
                        if next.token_type == LexerTokenType::EOF{
                            return t;
                        }
                        break ;

                    }else {
                        
                        prev = next;
                    }
                };
                continue;
                
            }
            else {
                t.push(x.clone());
                continue;
            }
        }
        
    }

    pub fn trim_spaces(v:Vec<Self>) -> Vec<Self>{
        v.into_iter()
            .filter(|x| !(x.token_type == LexerTokenType::Space || x.token_type == LexerTokenType::TabSpace) )
            .collect()
    }

}

/// unimplemented
/// 
/// gets the string with matching token
/// 
/// todo for more dynamic lexer
fn get_str(mut chars:impl Iterator<Item = char> , end_token:char,line:&mut u64,column:&mut u64)->Option<String>{
    let mut str = String::new();
    loop {

        if let Some(c) = chars.next(){
            if c == end_token{
                return Some(str);
            }
            str.push(c);
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
        void main(){int a = 10;}  3818  .";
        let ss = 
        "
        ..,()^+
        .....";
        let sss = ""; 
        let toks = LexerTokens::str_to_token(sss);
        
        println!("{toks:#?}");
        /*
        {
        let tok_out = format!("{toks:#?}");
        let out_path = "./test_out/lexer_tokens.ron";
        std::fs::create_dir_all("./test_out").unwrap();
        std::fs::write(out_path, tok_out).unwrap();
        }
        */
    }

    #[test]
    fn sizeofchar(){
        println!("{}",size_of_val(&'a'))
    }
    #[test]
    fn unsafe_test(){
        let 𐰦 = 21;
        println!("{}",𐰦)
    }
}
