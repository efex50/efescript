use crate::error::{CalcError, CalcErrorKind, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i128),
    Float(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Shl,
    Shr,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    Comma,
    Dot,
}

impl TokenKind {
    /// Source-like text used in error messages.
    pub fn describe(&self) -> String {
        use TokenKind::*;
        match self {
            Int(n) => n.to_string(),
            Float(x) => format!("{x:?}"),
            Ident(s) => s.clone(),
            Plus => "+".into(),
            Minus => "-".into(),
            Star => "*".into(),
            Slash => "/".into(),
            Percent => "%".into(),
            Shl => "<<".into(),
            Shr => ">>".into(),
            Amp => "&".into(),
            Pipe => "|".into(),
            Caret => "^".into(),
            Tilde => "~".into(),
            Bang => "!".into(),
            EqEq => "==".into(),
            NotEq => "!=".into(),
            Lt => "<".into(),
            Le => "<=".into(),
            Gt => ">".into(),
            Ge => ">=".into(),
            AndAnd => "&&".into(),
            OrOr => "||".into(),
            LParen => "(".into(),
            RParen => ")".into(),
            Comma => ",".into(),
            Dot => ".".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, start: usize, end: usize) -> Token {
        Token { kind, span: Span { start, end } }
    }
}

/// Anything that can feed tokens to the parser (the built-in [`Lexer`], an external lexer, a token slice...).
pub trait TokenSource {
    /// Next token, `Ok(None)` at end of input.
    fn next_token(&mut self) -> Result<Option<Token>, CalcError>;
}

/// Adapts any `Iterator<Item = Token>` into a [`TokenSource`].
pub struct TokenIter<I>(pub I);

impl<I: Iterator<Item = Token>> TokenSource for TokenIter<I> {
    fn next_token(&mut self) -> Result<Option<Token>, CalcError> {
        Ok(self.0.next())
    }
}

/// Streaming char-based lexer over a `&str`; spans are byte offsets.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

/// Lexes the whole string.
pub fn lex(src: &str) -> Result<Vec<Token>, CalcError> {
    let mut lx = Lexer::new(src);
    let mut out = Vec::new();
    while let Some(t) = lx.next_token()? {
        out.push(t);
    }
    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Lexer<'a> {
        Lexer { src, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }
    fn peek2(&self) -> Option<char> {
        self.src[self.pos..].chars().nth(1)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn take_while(&mut self, f: impl Fn(char) -> bool) {
        while let Some(c) = self.peek() {
            if !f(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    fn number(&mut self, start: usize) -> Result<TokenKind, CalcError> {
        let bad = |this: &Lexer| {
            CalcError::new(
                CalcErrorKind::InvalidNumber(this.src[start..this.pos].to_string()),
                Span::new(start, this.pos),
            )
        };
        let first = self.peek().unwrap();
        let radix = match (first, self.peek2()) {
            ('0', Some('x' | 'X')) => Some(16),
            ('0', Some('b' | 'B')) => Some(2),
            ('0', Some('o' | 'O')) => Some(8),
            _ => None,
        };
        if let Some(radix) = radix {
            self.pos += 2;
            let ds = self.pos;
            self.take_while(is_ident_continue);
            let digits: String = self.src[ds..self.pos].chars().filter(|&c| c != '_').collect();
            return i128::from_str_radix(&digits, radix)
                .ok()
                .filter(|_| !digits.starts_with('+') && !digits.starts_with('-'))
                .map(TokenKind::Int)
                .ok_or_else(|| bad(self));
        }
        self.take_while(|c| c.is_ascii_digit() || c == '_');
        let mut is_float = false;
        if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.pos += 1;
            self.take_while(|c| c.is_ascii_digit() || c == '_');
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let rest = &self.src[self.pos + 1..];
            let mut it = rest.chars();
            let c1 = it.next();
            let ok = match c1 {
                Some(c) if c.is_ascii_digit() => true,
                Some('+' | '-') => it.next().is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            if ok {
                is_float = true;
                self.pos += 1;
                if matches!(self.peek(), Some('+' | '-')) {
                    self.pos += 1;
                }
                self.take_while(|c| c.is_ascii_digit());
            }
        }
        // `12abc` is one bad number, not `12` followed by an identifier.
        if self.peek().is_some_and(is_ident_start) {
            self.take_while(is_ident_continue);
            return Err(bad(self));
        }
        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        if is_float {
            text.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .map(TokenKind::Float)
                .ok_or_else(|| bad(self))
        } else {
            text.parse::<i128>().map(TokenKind::Int).map_err(|_| bad(self))
        }
    }
}

impl TokenSource for Lexer<'_> {
    fn next_token(&mut self) -> Result<Option<Token>, CalcError> {
        self.take_while(char::is_whitespace);
        let start = self.pos;
        let Some(c) = self.peek() else { return Ok(None) };
        use TokenKind::*;
        let kind = if c.is_ascii_digit() {
            self.number(start)?
        } else if is_ident_start(c) {
            self.take_while(is_ident_continue);
            Ident(self.src[start..self.pos].to_string())
        } else {
            self.bump();
            match c {
                '+' => Plus,
                '-' => Minus,
                '*' => Star,
                '/' => Slash,
                '%' => Percent,
                '^' => Caret,
                '~' => Tilde,
                '(' => LParen,
                ')' => RParen,
                ',' => Comma,
                '.' => Dot,
                '<' => {
                    if self.eat('<') {
                        Shl
                    } else if self.eat('=') {
                        Le
                    } else {
                        Lt
                    }
                }
                '>' => {
                    if self.eat('>') {
                        Shr
                    } else if self.eat('=') {
                        Ge
                    } else {
                        Gt
                    }
                }
                '=' if self.eat('=') => EqEq,
                '!' => {
                    if self.eat('=') {
                        NotEq
                    } else {
                        Bang
                    }
                }
                '&' => {
                    if self.eat('&') {
                        AndAnd
                    } else {
                        Amp
                    }
                }
                '|' => {
                    if self.eat('|') {
                        OrOr
                    } else {
                        Pipe
                    }
                }
                other => {
                    return Err(CalcError::new(
                        CalcErrorKind::UnexpectedToken(other.to_string()),
                        Span::new(start, self.pos),
                    ));
                }
            }
        };
        Ok(Some(Token { kind, span: Span::new(start, self.pos) }))
    }
}
