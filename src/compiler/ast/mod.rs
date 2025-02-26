pub mod calculations;
pub mod lexer;

pub struct Ast {
    pub head: Header,
    pub body: Body,
}

pub struct Header {}

pub struct Body {}

pub enum AstObj {
    AstCalc,
}
    
pub enum Primitives{
    Int{
        ident:String,
        value:i32,
    },
    Unt{
        ident:String,
        value:u32,
    },
    Array{
        ident:String,
        value:Vec<u32>// todo 
    },
}