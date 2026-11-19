//! Lexer for `.efa` (see `docs/efa-syntax.md`, "Lexical rules"). Independent of the legacy
//! lexer in `src/compiler`.
//!
//! `lex` never panics: it indexes nothing that can be out of range, and every error recovers
//! by skipping the offending character or literal. The token stream always ends with `Eof`,
//! and (unless it is empty) with a `Newline` right before it.
//!
//! Newline rules: no `Newline` token at the start of the file; runs of newlines (blank and
//! comment-only lines) collapse into one token; a newline directly after a `,` is not
//! emitted (a list may continue on the next line).

use super::ast::Span;
use super::diag::Diagnostic;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// `[A-Za-z_][A-Za-z0-9_]*` segments joined by single dots: `acc`, `i64.add`, `std.println`.
    Name(String),
    /// Integer literal, sign applied.
    Int(i128),
    /// Float literal as source text (underscores removed, sign included): `1.5`, `-1e9`,
    /// `inf`, `-inf`, `nan`, `-nan`, `nan:0x7fc00001`.
    Float(String),
    /// `'a'`, `'\n'`.
    Char(char),
    /// `"..."` unescaped to bytes.
    Str(Vec<u8>),
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Equals,
    Arrow,
    /// `&`: bitwise and in a constant expression; before a struct name in a provide type
    /// (efa-syntax 16.9) it is the reference sigil.
    Amp,
    /// Constant-expression operators (efa-syntax 18). A `-` that directly starts a number
    /// is part of the literal (`-5`), the parser splits it when it follows an operand.
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    lines: Vec<&'a str>,
    file: &'a str,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
}

/// Lexes `src`. `file` is only used in diagnostics.
pub fn lex(src: &str, file: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let lines: Vec<&str> =
        src.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut lx = Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        lines,
        file,
        tokens: Vec::new(),
        diags: Vec::new(),
    };
    lx.run();
    (lx.tokens, lx.diags)
}

impl<'a> Lexer<'a> {
    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos.saturating_add(n)).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line = self.line.saturating_add(1);
            self.col = 1;
        } else {
            self.col = self.col.saturating_add(1);
        }
        Some(c)
    }

    fn error(&mut self, line: u32, col: u32, len: u32, msg: impl Into<String>) {
        let text = self.lines.get((line as usize).saturating_sub(1)).copied().unwrap_or("");
        self.diags.push(Diagnostic::with_line(self.file, text, Span::new(line, col, len), msg));
    }

    /// Error from `(line, col)` to the current position (same line).
    fn error_from(&mut self, line: u32, col: u32, msg: impl Into<String>) {
        let len = if self.line == line { self.col.saturating_sub(col).max(1) } else { 1 };
        self.error(line, col, len, msg);
    }

    fn push(&mut self, kind: TokenKind, line: u32, col: u32) {
        let len = if self.line == line { self.col.saturating_sub(col) } else { 1 };
        self.tokens.push(Token { kind, span: Span::new(line, col, len.max(1)) });
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            let (line, col) = (self.line, self.col);
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => {
                    self.bump();
                    let suppress = matches!(
                        self.tokens.last().map(|t| &t.kind),
                        None | Some(TokenKind::Newline) | Some(TokenKind::Comma)
                    );
                    if !suppress {
                        self.tokens.push(Token {
                            kind: TokenKind::Newline,
                            span: Span::new(line, col, 1),
                        });
                    }
                }
                ';' => {
                    while let Some(c) = self.peek(0) {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '{' => self.single(TokenKind::LBrace),
                '}' => self.single(TokenKind::RBrace),
                '(' => self.single(TokenKind::LParen),
                ')' => self.single(TokenKind::RParen),
                '[' => self.single(TokenKind::LBracket),
                ']' => self.single(TokenKind::RBracket),
                ',' => self.single(TokenKind::Comma),
                ':' => self.single(TokenKind::Colon),
                '=' => self.single(TokenKind::Equals),
                '&' => self.single(TokenKind::Amp),
                '+' => self.single(TokenKind::Plus),
                '*' => self.single(TokenKind::Star),
                '/' => self.single(TokenKind::Slash),
                '%' => self.single(TokenKind::Percent),
                '|' => self.single(TokenKind::Pipe),
                '^' => self.single(TokenKind::Caret),
                '~' => self.single(TokenKind::Tilde),
                '<' | '>' if self.peek(1) == Some(c) => {
                    self.bump();
                    self.bump();
                    self.push(if c == '<' { TokenKind::Shl } else { TokenKind::Shr }, line, col);
                }
                '-' => self.minus(),
                '0'..='9' => self.number(false),
                '"' => self.string(),
                '\'' => self.char_lit(),
                c if is_ident_start(c) => self.name(false),
                other => {
                    self.bump();
                    self.error(line, col, 1, format!("unexpected character `{}`", other.escape_default()));
                }
            }
        }
        let (line, col) = (self.line, self.col);
        if !matches!(self.tokens.last().map(|t| &t.kind), None | Some(TokenKind::Newline)) {
            self.tokens.push(Token { kind: TokenKind::Newline, span: Span::new(line, col, 1) });
        }
        self.tokens.push(Token { kind: TokenKind::Eof, span: Span::new(line, col, 1) });
    }

    fn single(&mut self, kind: TokenKind) {
        let (line, col) = (self.line, self.col);
        self.bump();
        self.push(kind, line, col);
    }

    /// End index (exclusive) of the dotted name starting at `from`; `from` if there is none.
    fn scan_name(&self, from: usize) -> usize {
        match self.chars.get(from) {
            Some(&c) if is_ident_start(c) => {}
            _ => return from,
        }
        let mut i = from;
        loop {
            while self.chars.get(i).map_or(false, |&c| is_ident_char(c)) {
                i += 1;
            }
            if self.chars.get(i) == Some(&'.')
                && self.chars.get(i + 1).map_or(false, |&c| is_ident_start(c))
            {
                i += 1;
            } else {
                return i;
            }
        }
    }

    fn minus(&mut self) {
        let (line, col) = (self.line, self.col);
        match self.peek(1) {
            Some('>') => {
                self.bump();
                self.bump();
                self.push(TokenKind::Arrow, line, col);
            }
            Some(d) if d.is_ascii_digit() => {
                self.bump();
                self.number(true);
            }
            Some(c) if is_ident_start(c) => {
                let end = self.scan_name(self.pos + 1);
                let text: String =
                    self.chars.get(self.pos + 1..end).unwrap_or(&[]).iter().collect();
                if text == "inf" || text == "nan" {
                    self.bump();
                    self.name(true);
                } else {
                    self.single(TokenKind::Minus);
                }
            }
            _ => self.single(TokenKind::Minus),
        }
    }

    fn name(&mut self, neg: bool) {
        let (line, col) = if neg { (self.line, self.col.saturating_sub(1)) } else { (self.line, self.col) };
        let end = self.scan_name(self.pos);
        let mut text = String::new();
        while self.pos < end {
            if let Some(c) = self.bump() {
                text.push(c);
            }
        }
        if text == "nan"
            && self.peek(0) == Some(':')
            && self.peek(1) == Some('0')
            && self.peek(2) == Some('x')
        {
            self.bump();
            self.bump();
            self.bump();
            let mut digits = String::new();
            while let Some(c) = self.peek(0) {
                if c.is_ascii_hexdigit() {
                    digits.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            if digits.is_empty() || digits.len() > 16 {
                self.error_from(line, col, "`nan:0x` needs 1 to 16 hex digits");
            } else if neg {
                self.error_from(line, col, "the sign of `nan:0x...` is part of its bits; write `nan:0x...` without `-`");
            } else {
                self.push(TokenKind::Float(format!("nan:0x{}", digits)), line, col);
            }
            return;
        }
        if text == "inf" || text == "nan" {
            let t = if neg { format!("-{}", text) } else { text };
            self.push(TokenKind::Float(t), line, col);
        } else {
            self.push(TokenKind::Name(text), line, col);
        }
    }

    /// At a digit (the sign, if any, is already consumed; `neg` says it was there).
    fn number(&mut self, neg: bool) {
        let (line, col) = if neg { (self.line, self.col.saturating_sub(1)) } else { (self.line, self.col) };
        let radix = match (self.peek(0), self.peek(1)) {
            (Some('0'), Some('x')) => 16,
            (Some('0'), Some('b')) => 2,
            (Some('0'), Some('o')) => 8,
            _ => 10,
        };
        if radix != 10 {
            let prefix = match radix {
                16 => "0x",
                2 => "0b",
                _ => "0o",
            };
            self.bump();
            self.bump();
            let mut run = String::new();
            while let Some(c) = self.peek(0) {
                if is_ident_char(c) {
                    run.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            let digits: String = run.chars().filter(|&c| c != '_').collect();
            if digits.is_empty() {
                self.error_from(line, col, format!("expected digits after `{}`", prefix));
                return;
            }
            let mut mag: u128 = 0;
            for c in digits.chars() {
                match c.to_digit(radix) {
                    Some(d) => match mag.checked_mul(radix as u128).and_then(|m| m.checked_add(d as u128)) {
                        Some(m) => mag = m,
                        None => {
                            self.error_from(line, col, "integer literal is too large");
                            return;
                        }
                    },
                    None => {
                        self.error_from(
                            line,
                            col,
                            format!("invalid digit `{}` in `{}` literal", c, prefix),
                        );
                        return;
                    }
                }
            }
            self.finish_int(mag, neg, line, col);
            return;
        }

        let mut text = String::new();
        let mut is_float = false;
        let digits = |lx: &mut Lexer, text: &mut String| {
            while let Some(c) = lx.peek(0) {
                if c.is_ascii_digit() || c == '_' {
                    if c != '_' {
                        text.push(c);
                    }
                    lx.bump();
                } else {
                    break;
                }
            }
        };
        digits(self, &mut text);
        if self.peek(0) == Some('.') && self.peek(1).map_or(false, |c| c.is_ascii_digit()) {
            is_float = true;
            text.push('.');
            self.bump();
            digits(self, &mut text);
        }
        if matches!(self.peek(0), Some('e') | Some('E')) {
            let has_exp = match self.peek(1) {
                Some(d) if d.is_ascii_digit() => true,
                Some('+') | Some('-') => self.peek(2).map_or(false, |c| c.is_ascii_digit()),
                _ => false,
            };
            if has_exp {
                is_float = true;
                text.push('e');
                self.bump();
                if let Some(s @ ('+' | '-')) = self.peek(0) {
                    text.push(s);
                    self.bump();
                }
                digits(self, &mut text);
            }
        }
        if self.peek(0).map_or(false, is_ident_char) {
            while self.peek(0).map_or(false, is_ident_char) {
                self.bump();
            }
            self.error_from(line, col, "invalid number literal");
            return;
        }
        if is_float {
            let full = if neg { format!("-{}", text) } else { text };
            self.push(TokenKind::Float(full), line, col);
        } else {
            let mut mag: u128 = 0;
            for c in text.chars() {
                let d = c.to_digit(10).unwrap_or(0) as u128;
                match mag.checked_mul(10).and_then(|m| m.checked_add(d)) {
                    Some(m) => mag = m,
                    None => {
                        self.error_from(line, col, "integer literal is too large");
                        return;
                    }
                }
            }
            self.finish_int(mag, neg, line, col);
        }
    }

    fn finish_int(&mut self, mag: u128, neg: bool, line: u32, col: u32) {
        let limit: u128 = if neg { 1u128 << 127 } else { (1u128 << 127) - 1 };
        if mag > limit {
            self.error_from(line, col, "integer literal is too large");
            return;
        }
        let v = if neg { (mag as i128).wrapping_neg() } else { mag as i128 };
        self.push(TokenKind::Int(v), line, col);
    }

    /// Reads the escape after a consumed backslash. `in_char` also allows `\'`; strings do not.
    /// Returns the value, or `None` after reporting an error (position left after the escape).
    fn escape(&mut self, line: u32, col: u32, in_char: bool) -> Option<u32> {
        let c = match self.peek(0) {
            Some(c) if c != '\n' => c,
            _ => {
                self.error_from(line, col, "unterminated escape sequence");
                return None;
            }
        };
        self.bump();
        match c {
            'n' => Some(10),
            't' => Some(9),
            'r' => Some(13),
            '0' => Some(0),
            '\\' => Some(92),
            '"' => Some(34),
            '\'' if in_char => Some(39),
            'x' => {
                let mut v = 0u32;
                for _ in 0..2 {
                    match self.peek(0).and_then(|h| h.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            self.bump();
                        }
                        None => {
                            self.error_from(line, col, "`\\x` needs two hex digits");
                            return None;
                        }
                    }
                }
                Some(v)
            }
            other => {
                self.error_from(line, col, format!("unknown escape `\\{}`", other.escape_default()));
                None
            }
        }
    }

    fn string(&mut self) {
        let (line, col) = (self.line, self.col);
        self.bump();
        let mut bytes: Vec<u8> = Vec::new();
        let mut bad = false;
        loop {
            match self.peek(0) {
                None | Some('\n') => {
                    self.error_from(line, col, "unterminated string literal");
                    return;
                }
                Some('"') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    let (el, ec) = (self.line, self.col);
                    self.bump();
                    match self.escape(el, ec, false) {
                        Some(v) => bytes.push(v as u8),
                        None => bad = true,
                    }
                }
                Some(c) => {
                    self.bump();
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
        if !bad {
            self.push(TokenKind::Str(bytes), line, col);
        }
    }

    fn char_lit(&mut self) {
        let (line, col) = (self.line, self.col);
        self.bump();
        let value: Option<u32> = match self.peek(0) {
            None | Some('\n') => {
                self.error_from(line, col, "unterminated character literal");
                return;
            }
            Some('\'') => {
                self.bump();
                self.error_from(line, col, "empty character literal");
                return;
            }
            Some('\\') => {
                let (el, ec) = (self.line, self.col);
                self.bump();
                self.escape(el, ec, true)
            }
            Some(c) => {
                self.bump();
                Some(c as u32)
            }
        };
        if self.peek(0) == Some('\'') {
            self.bump();
            if let Some(ch) = value.and_then(char::from_u32) {
                self.push(TokenKind::Char(ch), line, col);
            }
            return;
        }
        // Resynchronise: skip to the closing quote on this line, if any.
        let mut closed = false;
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                break;
            }
            self.bump();
            if c == '\'' {
                closed = true;
                break;
            }
        }
        if value.is_some() {
            let msg = if closed {
                "a character literal holds exactly one character"
            } else {
                "unterminated character literal"
            };
            self.error_from(line, col, msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (t, d) = lex(src, "t.efa");
        assert!(d.is_empty(), "unexpected diagnostics for {:?}: {:?}", src, d);
        t.into_iter().map(|t| t.kind).collect()
    }

    fn errs(src: &str) -> Vec<(u32, u32, u32, String)> {
        let (_, d) = lex(src, "t.efa");
        d.into_iter().map(|d| (d.line, d.column, d.len, d.message)).collect()
    }

    fn name(s: &str) -> TokenKind {
        Name(s.to_string())
    }

    #[test]
    fn empty_and_eof() {
        assert_eq!(kinds(""), vec![Eof]);
        assert_eq!(kinds("\n\n  \n"), vec![Eof]);
        assert_eq!(kinds("; only a comment"), vec![Eof]);
        assert_eq!(kinds("ret"), vec![name("ret"), Newline, Eof]);
        assert_eq!(kinds("ret\n"), vec![name("ret"), Newline, Eof]);
    }

    #[test]
    fn newlines_collapse() {
        assert_eq!(
            kinds("\n\na\n\n; c\n\nb ; tail\n"),
            vec![name("a"), Newline, name("b"), Newline, Eof]
        );
        assert_eq!(kinds("a\r\nb\r\n"), vec![name("a"), Newline, name("b"), Newline, Eof]);
    }

    #[test]
    fn newline_after_comma_is_continuation() {
        assert_eq!(
            kinds("a 1,\n  2,\n\n 3\n"),
            vec![name("a"), Int(1), Comma, Int(2), Comma, Int(3), Newline, Eof]
        );
        // trailing comma at end of file still ends with Newline
        assert_eq!(kinds("a 1,"), vec![name("a"), Int(1), Comma, Newline, Eof]);
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            kinds("{ } ( ) [ ] , : = ->"),
            vec![LBrace, RBrace, LParen, RParen, LBracket, RBracket, Comma, Colon, Equals, Arrow, Newline, Eof]
        );
        assert_eq!(kinds(")->i64"), vec![RParen, Arrow, name("i64"), Newline, Eof]);
    }

    #[test]
    fn dotted_names() {
        assert_eq!(
            kinds("i64.add std.println msg.len i64.trunc_f64_s const.i32 a.b.c _x r12"),
            vec![
                name("i64.add"),
                name("std.println"),
                name("msg.len"),
                name("i64.trunc_f64_s"),
                name("const.i32"),
                name("a.b.c"),
                name("_x"),
                name("r12"),
                Newline,
                Eof
            ]
        );
        // a trailing dot or a dot before a digit is not part of the name
        assert_eq!(errs("a.").len(), 1);
        assert_eq!(errs("r0.1").len(), 1);
    }

    #[test]
    fn integers() {
        assert_eq!(
            kinds("0 7 123 1_000 0x10 0xFF_ff 0b1010 0b1_0 0o17 -5 -0x10 -0b11 -0o7 007"),
            vec![
                Int(0),
                Int(7),
                Int(123),
                Int(1000),
                Int(16),
                Int(255 * 256 + 255),
                Int(10),
                Int(2),
                Int(15),
                Int(-5),
                Int(-16),
                Int(-3),
                Int(-7),
                Int(7),
                Newline,
                Eof
            ]
        );
        assert_eq!(
            kinds("18446744073709551615 -9223372036854775808 0xFFFFFFFFFFFFFFFF"),
            vec![
                Int(u64::MAX as i128),
                Int(i64::MIN as i128),
                Int(u64::MAX as i128),
                Newline,
                Eof
            ]
        );
        assert_eq!(
            kinds("170141183460469231731687303715884105727 -170141183460469231731687303715884105728"),
            vec![Int(i128::MAX), Int(i128::MIN), Newline, Eof]
        );
    }

    #[test]
    fn integer_errors() {
        assert_eq!(
            errs("170141183460469231731687303715884105728"),
            vec![(1, 1, 39, "integer literal is too large".to_string())]
        );
        assert_eq!(
            errs("  0x"),
            vec![(1, 3, 2, "expected digits after `0x`".to_string())]
        );
        assert_eq!(
            errs("0b102"),
            vec![(1, 1, 5, "invalid digit `2` in `0b` literal".to_string())]
        );
        assert_eq!(errs("0o9"), vec![(1, 1, 3, "invalid digit `9` in `0o` literal".to_string())]);
        assert_eq!(errs("0xZZ"), vec![(1, 1, 4, "invalid digit `Z` in `0x` literal".to_string())]);
        assert_eq!(errs("12abc"), vec![(1, 1, 5, "invalid number literal".to_string())]);
        assert_eq!(errs("-12abc"), vec![(1, 1, 6, "invalid number literal".to_string())]);
        // recovery: the next token is still lexed
        let (t, d) = lex("12abc 7", "t");
        assert_eq!(d.len(), 1);
        assert_eq!(t.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(), vec![Int(7), Newline, Eof]);
    }

    #[test]
    fn floats() {
        assert_eq!(
            kinds("1.5 -0.25 1e9 1E9 1.5e-3 2e+4 1_0.2_5 inf -inf nan -nan 0.0"),
            vec![
                Float("1.5".into()),
                Float("-0.25".into()),
                Float("1e9".into()),
                Float("1e9".into()),
                Float("1.5e-3".into()),
                Float("2e+4".into()),
                Float("10.25".into()),
                Float("inf".into()),
                Float("-inf".into()),
                Float("nan".into()),
                Float("-nan".into()),
                Float("0.0".into()),
                Newline,
                Eof
            ]
        );
        assert_eq!(
            kinds("nan:0x7fc00001 nan:0xFFF8000000000000 nan:0x1"),
            vec![
                Float("nan:0x7fc00001".into()),
                Float("nan:0xFFF8000000000000".into()),
                Float("nan:0x1".into()),
                Newline,
                Eof
            ]
        );
        // `1.` is the integer 1 followed by a stray dot; `1e` is an invalid number
        assert_eq!(errs("1.").len(), 1);
        assert_eq!(errs("1e").len(), 1);
        // `inf`/`nan` are literals only when they are the whole name
        assert_eq!(kinds("info nano inf.x"), vec![name("info"), name("nano"), name("inf.x"), Newline, Eof]);
    }

    #[test]
    fn nan_payload_errors() {
        assert_eq!(errs("nan:0x"), vec![(1, 1, 6, "`nan:0x` needs 1 to 16 hex digits".to_string())]);
        assert_eq!(
            errs("nan:0x12345678901234567"),
            vec![(1, 1, 23, "`nan:0x` needs 1 to 16 hex digits".to_string())]
        );
        assert_eq!(errs("-nan:0x1").len(), 1);
    }

    #[test]
    fn minus_handling() {
        // `i64.sub n, n, 1`: no minus at all; the dash only exists inside literals and `->`.
        assert_eq!(
            kinds("i64.sub n, n, 1"),
            vec![name("i64.sub"), name("n"), Comma, name("n"), Comma, Int(1), Newline, Eof]
        );
        assert_eq!(
            kinds("i64.add n, n, -1"),
            vec![name("i64.add"), name("n"), Comma, name("n"), Comma, Int(-1), Newline, Eof]
        );
        assert_eq!(kinds("-> -1"), vec![Arrow, Int(-1), Newline, Eof]);
        assert_eq!(kinds("->-1"), vec![Arrow, Int(-1), Newline, Eof]);
        // a detached minus, or one before a name, is the operator of a constant expression
        assert_eq!(kinds("- 1"), vec![Minus, Int(1), Newline, Eof]);
        assert_eq!(kinds("-x"), vec![Minus, name("x"), Newline, Eof]);
        assert_eq!(kinds("-info"), vec![Minus, name("info"), Newline, Eof]);
        assert_eq!(kinds("-inf nan"), vec![Float("-inf".into()), Float("nan".into()), Newline, Eof]);
        assert_eq!(kinds("a -x b"), vec![name("a"), Minus, name("x"), name("b"), Newline, Eof]);
    }

    #[test]
    fn expression_operators() {
        assert_eq!(
            kinds("+ * / % | ^ ~ & << >> ( )"),
            vec![Plus, Star, Slash, Percent, Pipe, Caret, Tilde, Amp, Shl, Shr, LParen, RParen, Newline, Eof]
        );
        // a single `<` or `>` is still an error
        assert_eq!(errs("<").len(), 1);
        assert_eq!(errs(">").len(), 1);
        // `4-1` lexes as `4` and the literal `-1`; the parser splits it
        assert_eq!(kinds("4-1"), vec![Int(4), Int(-1), Newline, Eof]);
    }

    #[test]
    fn chars() {
        assert_eq!(
            kinds(r#"'a' '\n' '\t' '\r' '\0' '\\' '\'' '\x41' ' ' '\"' 'é'"#),
            vec![
                Char('a'),
                Char('\n'),
                Char('\t'),
                Char('\r'),
                Char('\0'),
                Char('\\'),
                Char('\''),
                Char('A'),
                Char(' '),
                Char('"'),
                Char('é'),
                Newline,
                Eof
            ]
        );
    }

    #[test]
    fn char_errors() {
        assert_eq!(errs("''"), vec![(1, 1, 2, "empty character literal".to_string())]);
        assert_eq!(errs("'ab'"), vec![(1, 1, 4, "a character literal holds exactly one character".to_string())]);
        assert_eq!(errs("'a"), vec![(1, 1, 2, "unterminated character literal".to_string())]);
        assert_eq!(errs("'"), vec![(1, 1, 1, "unterminated character literal".to_string())]);
        assert_eq!(errs(r"'\q'"), vec![(1, 2, 2, "unknown escape `\\q`".to_string())]);
        assert_eq!(errs(r"'\xZ'"), vec![(1, 2, 2, "`\\x` needs two hex digits".to_string())]);
    }

    #[test]
    fn strings() {
        let (t, d) = lex(r#""hello" "" "a\nb\t\r\0\\\"" "\x00\xff\x7F" "é""#, "t");
        assert!(d.is_empty());
        let k: Vec<TokenKind> = t.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            k,
            vec![
                Str(b"hello".to_vec()),
                Str(vec![]),
                Str(b"a\nb\t\r\0\\\"".to_vec()),
                Str(vec![0, 255, 0x7f]),
                Str("é".as_bytes().to_vec()),
                Newline,
                Eof
            ]
        );
        // `;` inside a string is not a comment
        assert_eq!(kinds(r#""a;b""#), vec![Str(b"a;b".to_vec()), Newline, Eof]);
    }

    #[test]
    fn string_errors() {
        assert_eq!(errs("  \"abc"), vec![(1, 3, 4, "unterminated string literal".to_string())]);
        assert_eq!(errs("\"abc\nnext"), vec![(1, 1, 4, "unterminated string literal".to_string())]);
        assert_eq!(errs(r#""a\qb""#), vec![(1, 3, 2, "unknown escape `\\q`".to_string())]);
        assert_eq!(errs(r#""a\x4""#), vec![(1, 3, 3, "`\\x` needs two hex digits".to_string())]);
        // `\'` is only valid in characters
        assert_eq!(errs(r#""\'""#).len(), 1);
        // the bad string is dropped, the rest is lexed
        let (t, d) = lex("a \"x\\q\" b", "t");
        assert_eq!(d.len(), 1);
        assert_eq!(
            t.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(),
            vec![name("a"), name("b"), Newline, Eof]
        );
    }

    #[test]
    fn unexpected_characters_with_positions() {
        let (t, d) = lex("ab @ cd\n  é $ x\n", "f.efa");
        assert_eq!(d.len(), 3);
        assert_eq!((d[0].line, d[0].column, d[0].len), (1, 4, 1));
        assert_eq!(d[0].message, "unexpected character `@`");
        assert_eq!((d[1].line, d[1].column), (2, 3));
        assert_eq!((d[2].line, d[2].column), (2, 5));
        assert_eq!(d[0].source_line, "ab @ cd");
        assert_eq!(d[0].file, "f.efa");
        let k: Vec<TokenKind> = t.into_iter().map(|t| t.kind).collect();
        assert_eq!(k, vec![name("ab"), name("cd"), Newline, name("x"), Newline, Eof]);
    }

    #[test]
    fn spans() {
        let (t, _) = lex("  i64.add acc, -12\nlabel:\n", "t");
        let s: Vec<(u32, u32, u32)> = t.iter().map(|t| (t.span.line, t.span.column, t.span.len)).collect();
        assert_eq!(s[0], (1, 3, 7));
        assert_eq!(s[1], (1, 11, 3));
        assert_eq!(s[2], (1, 14, 1));
        assert_eq!(s[3], (1, 16, 3));
        assert_eq!(t[4].kind, Newline);
        assert_eq!(s[5], (2, 1, 5));
        assert_eq!(s[6], (2, 6, 1));
    }

    #[test]
    fn tab_is_one_column() {
        let (t, _) = lex("\tret", "t");
        assert_eq!((t[0].span.line, t[0].span.column), (1, 2));
    }

    #[test]
    fn a_whole_function() {
        let src = "export fn sum(n: i64) -> i64 {\n    reg acc\nloop:\n    br_if t, done ; c\n}\n";
        let k = kinds(src);
        assert_eq!(k.first(), Some(&name("export")));
        assert!(k.contains(&Arrow));
        assert!(k.contains(&Colon));
        assert_eq!(k.last(), Some(&Eof));
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn never_panics_on_odd_and_mutated_inputs() {
        let seeds = [
            "export fn main() {\n    call std.println(msg, msg.len)\n    ret\n}\n",
            "msg db \"hi\\n\", 0xff, 'a', '\\n'\nconst X = -1.5e3\nglobal mut g: f64 = nan:0x7ff8000000000001\n",
            "table [f, null]\nmemory 1 2\nbr_table a, [x, y], z\ncall_indirect (i64) -> i64, i(a) -> x\n",
            "0x 0b 0o -0x - -> ' '' \" \\ \\x 1e 1e+ 1.e5 .5 nan:0x inf -inf é 日本 \u{0} \r\n",
        ];
        let alphabet: Vec<char> =
            "abz_09 .,:=-+(){}[]\"'\\;\n\t\rxXeE\u{e9}\u{65e5}\u{0}>".chars().collect();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut count = 0;
        for seed in seeds {
            let base: Vec<char> = seed.chars().collect();
            for _ in 0..120 {
                let mut s = base.clone();
                let edits = 1 + (rng.next() % 6) as usize;
                for _ in 0..edits {
                    let r = rng.next();
                    let at = if s.is_empty() { 0 } else { (r >> 8) as usize % s.len() };
                    let c = alphabet[(r >> 20) as usize % alphabet.len()];
                    match r % 4 {
                        0 if !s.is_empty() => {
                            s.remove(at);
                        }
                        1 => s.insert(at.min(s.len()), c),
                        2 if !s.is_empty() => s[at] = c,
                        _ => s.truncate(at),
                    }
                }
                let text: String = s.into_iter().collect();
                let (toks, _) = lex(&text, "fuzz.efa");
                assert!(matches!(toks.last().map(|t| &t.kind), Some(TokenKind::Eof)));
                count += 1;
            }
        }
        // purely random strings
        for _ in 0..120 {
            let len = (rng.next() % 40) as usize;
            let text: String =
                (0..len).map(|_| alphabet[(rng.next() >> 11) as usize % alphabet.len()]).collect();
            let (toks, _) = lex(&text, "fuzz.efa");
            assert!(matches!(toks.last().map(|t| &t.kind), Some(TokenKind::Eof)));
            count += 1;
        }
        assert!(count >= 400);
    }
}
