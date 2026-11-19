use crate::ast::{BinOp, Expr, ExprKind, UnOp, UNARY_PREC};
use crate::error::{CalcError, CalcErrorKind, Span};
use crate::lexer::{Lexer, Token, TokenIter, TokenKind, TokenSource};

const MAX_DEPTH: u32 = 200;

/// Parses a whole string (lexes with the built-in lexer, rejects trailing tokens).
pub fn parse(src: &str) -> Result<Expr, CalcError> {
    let mut lx = Lexer::new(src);
    let mut p = Parser::new(&mut lx);
    let e = p.parse_expr()?;
    p.finish()?;
    Ok(e)
}

/// Parses a whole token slice (e.g. produced by an external lexer).
pub fn parse_tokens(tokens: &[Token]) -> Result<Expr, CalcError> {
    let mut src = TokenIter(tokens.iter().cloned());
    let mut p = Parser::new(&mut src);
    let e = p.parse_expr()?;
    p.finish()?;
    Ok(e)
}

/// Pratt parser over any [`TokenSource`]. Use [`Parser::parse_expr`] to parse one expression
/// and stop at the first token that cannot continue it (see [`Parser::peek`]), or
/// [`Parser::finish`] to require end of input.
pub struct Parser<'s> {
    src: &'s mut dyn TokenSource,
    peeked: Option<Option<Token>>,
    last_end: usize,
    depth: u32,
}

fn bin_op(k: &TokenKind) -> Option<BinOp> {
    use TokenKind as T;
    Some(match k {
        T::Plus => BinOp::Add,
        T::Minus => BinOp::Sub,
        T::Star => BinOp::Mul,
        T::Slash => BinOp::Div,
        T::Percent => BinOp::Rem,
        T::Shl => BinOp::Shl,
        T::Shr => BinOp::Shr,
        T::Amp => BinOp::BitAnd,
        T::Pipe => BinOp::BitOr,
        T::Caret => BinOp::BitXor,
        T::EqEq => BinOp::Eq,
        T::NotEq => BinOp::Ne,
        T::Lt => BinOp::Lt,
        T::Le => BinOp::Le,
        T::Gt => BinOp::Gt,
        T::Ge => BinOp::Ge,
        T::AndAnd => BinOp::And,
        T::OrOr => BinOp::Or,
        _ => return None,
    })
}

impl<'s> Parser<'s> {
    pub fn new(src: &'s mut dyn TokenSource) -> Parser<'s> {
        Parser { src, peeked: None, last_end: 0, depth: 0 }
    }

    /// Next token without consuming it.
    pub fn peek(&mut self) -> Result<Option<Token>, CalcError> {
        if self.peeked.is_none() {
            self.peeked = Some(self.src.next_token()?);
        }
        Ok(self.peeked.clone().unwrap())
    }

    fn bump(&mut self) -> Result<Option<Token>, CalcError> {
        let t = match self.peeked.take() {
            Some(t) => t,
            None => self.src.next_token()?,
        };
        if let Some(t) = &t {
            self.last_end = t.span.end;
        }
        Ok(t)
    }

    fn end_error(&self) -> CalcError {
        CalcError::new(CalcErrorKind::UnexpectedEnd, Span::new(self.last_end, self.last_end))
    }

    fn unexpected(t: &Token) -> CalcError {
        CalcError::new(CalcErrorKind::UnexpectedToken(t.kind.describe()), t.span)
    }

    /// Errors if any token remains.
    pub fn finish(&mut self) -> Result<(), CalcError> {
        match self.peek()? {
            Some(t) => Err(Self::unexpected(&t)),
            None => Ok(()),
        }
    }

    pub fn parse_expr(&mut self) -> Result<Expr, CalcError> {
        self.expr_bp(0)
    }

    fn expr_bp(&mut self, min_bp: u8) -> Result<Expr, CalcError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let span = self.peek()?.map(|t| t.span).unwrap_or(Span::new(self.last_end, self.last_end));
            return Err(CalcError::new(CalcErrorKind::TooDeep, span));
        }
        let r = self.expr_bp_inner(min_bp);
        self.depth -= 1;
        r
    }

    fn expr_bp_inner(&mut self, min_bp: u8) -> Result<Expr, CalcError> {
        let mut lhs = self.prefix()?;
        while let Some(tok) = self.peek()? {
            let Some(op) = bin_op(&tok.kind) else { break };
            let lbp = op.precedence();
            if lbp < min_bp {
                break;
            }
            self.bump()?;
            let rhs = self.expr_bp(lbp + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr::new(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span);
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Expr, CalcError> {
        let Some(tok) = self.bump()? else { return Err(self.end_error()) };
        let leaf = match tok.kind {
            TokenKind::Int(n) => ExprKind::Int(n),
            TokenKind::Float(x) => ExprKind::Float(x),
            TokenKind::Ident(ref s) if s == "true" => ExprKind::Bool(true),
            TokenKind::Ident(ref s) if s == "false" => ExprKind::Bool(false),
            TokenKind::Ident(s) => ExprKind::Var(s),
            TokenKind::LParen => {
                let inner = self.expr_bp(0)?;
                match self.bump()? {
                    Some(Token { kind: TokenKind::RParen, span }) => {
                        let e = Expr::new(
                            ExprKind::Paren(Box::new(inner)),
                            tok.span.to(span),
                        );
                        return self.postfix(e);
                    }
                    None => return Err(CalcError::new(CalcErrorKind::UnclosedBracket, tok.span)),
                    Some(t) => return Err(Self::unexpected(&t)),
                }
            }
            TokenKind::Minus | TokenKind::Bang | TokenKind::Tilde => {
                let op = match tok.kind {
                    TokenKind::Minus => UnOp::Neg,
                    TokenKind::Bang => UnOp::Not,
                    _ => UnOp::BitNot,
                };
                let operand = self.expr_bp(UNARY_PREC)?;
                let span = tok.span.to(operand.span);
                return Ok(Expr::new(ExprKind::Unary(op, Box::new(operand)), span));
            }
            _ => return Err(Self::unexpected(&tok)),
        };
        self.postfix(Expr::new(leaf, tok.span))
    }

    /// `.field` and `name(args)` suffixes.
    fn postfix(&mut self, mut e: Expr) -> Result<Expr, CalcError> {
        loop {
            let Some(tok) = self.peek()? else { return Ok(e) };
            match tok.kind {
                TokenKind::Dot => {
                    self.bump()?;
                    match self.bump()? {
                        Some(Token { kind: TokenKind::Ident(name), span }) => {
                            let sp = e.span.to(span);
                            e = Expr::new(ExprKind::Field(Box::new(e), name), sp);
                        }
                        Some(t) => return Err(Self::unexpected(&t)),
                        None => return Err(self.end_error()),
                    }
                }
                TokenKind::LParen if matches!(e.kind, ExprKind::Var(_)) => {
                    self.bump()?;
                    let ExprKind::Var(name) = e.kind else { unreachable!() };
                    let mut args = Vec::new();
                    let close;
                    if let Some(Token { kind: TokenKind::RParen, span }) = self.peek()? {
                        self.bump()?;
                        close = span;
                    } else {
                        loop {
                            args.push(self.expr_bp(0)?);
                            match self.bump()? {
                                Some(Token { kind: TokenKind::Comma, .. }) => {}
                                Some(Token { kind: TokenKind::RParen, span }) => {
                                    close = span;
                                    break;
                                }
                                None => {
                                    return Err(CalcError::new(CalcErrorKind::UnclosedBracket, tok.span));
                                }
                                Some(t) => return Err(Self::unexpected(&t)),
                            }
                        }
                    }
                    e = Expr::new(ExprKind::Call(name, args), e.span.to(close));
                }
                _ => return Ok(e),
            }
        }
    }
}
