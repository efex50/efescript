//! `calc`: an expression front end (lexer, Pratt parser, evaluator, constant folder).
//!
//! ```
//! use calc::{parse, eval, Vars, Value};
//! let e = parse("(x + 2) * 3").unwrap();
//! let mut vars = Vars::new();
//! vars.set("x", 4i128);
//! assert_eq!(eval(&e, &vars).unwrap(), Value::Int(18));
//! ```

pub mod ast;
pub mod error;
pub mod eval;
pub mod lexer;
pub mod parser;

pub use ast::{BinOp, Expr, ExprKind, UnOp};
pub use error::{CalcError, CalcErrorKind, Span};
pub use eval::{eval, simplify, Env, IntWidth, NoEnv, Value, Vars};
pub use lexer::{lex, Lexer, Token, TokenIter, TokenKind, TokenSource};
pub use parser::{parse, parse_tokens, Parser};

/// Parses and evaluates `src` with no variables (handy for literals such as `-0x10` or `2.5`).
pub fn parse_value(src: &str) -> Result<Value, CalcError> {
    eval(&parse(src)?, &NoEnv)
}
