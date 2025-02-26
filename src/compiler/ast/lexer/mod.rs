

struct Position{
    start:LineColmn,
    end:LineColmn
}

struct LineColmn{
    line:usize,
    colmn:usize
}

#[repr(u8)]
enum LexerTokens{
    Number(u128),
    Ident( String),
    String(String),
    
    //  operator signs

    /// sign +
    Sum,
    /// sign -
    Subtact,
    /// sign *
    Mult,
    /// sign /
    Divide,
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
    /// sign (
    RPar,
    /// sign )
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
    /// sign //
    Comment,
    /// sign /*
    MCommentStart,
    /// sign */
    MCommentEnd,
    /// sign ;
    SemiColon,

    
}

impl LexerTokens {
    pub fn from_string<S:Into<String>>(str:S){
        let str:String = str.into();
        let str:String = str.trim().parse().unwrap();
        let str = str.chars();
        let mut counter = 0;
        for x in str{
            counter += 1;
            println!("counter: {counter} ={}",x)
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
        let toks = LexerTokens::from_string(s);
    }
}