


impl std::fmt::Display for LineColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Position{
    pub start:LineColumn,
    pub end:LineColumn
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct LineColumn{
    pub line: u64,
    pub column:u64
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
    /// sign :
    Colon,
    /// string literal, already unescaped (raw bytes)
    Str(Vec<u8>),
    /// malformed string literal (message)
    StrErr(String),

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
    /// short human readable description used in error messages
    pub fn describe(&self) -> String {
        match self {
            LexerTokenType::Ident(s) => format!("`{s}`"),
            LexerTokenType::Number(n) => format!("number {n}"),
            LexerTokenType::Str(_) => "a string literal".to_string(),
            LexerTokenType::StrErr(_) => "a malformed string literal".to_string(),
            LexerTokenType::EOL => "end of line".to_string(),
            LexerTokenType::EOF => "end of file".to_string(),
            LexerTokenType::Space | LexerTokenType::TabSpace => "whitespace".to_string(),
            LexerTokenType::Plus => "`+`".to_string(),
            LexerTokenType::Minus => "`-`".to_string(),
            LexerTokenType::Punc => "`,`".to_string(),
            LexerTokenType::Dot => "`.`".to_string(),
            LexerTokenType::LSPar => "`[`".to_string(),
            LexerTokenType::RSPar => "`]`".to_string(),
            LexerTokenType::Colon => "`:`".to_string(),
            LexerTokenType::SemiColon => "`;`".to_string(),
            other => format!("{other:?}"),
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

fn sym_token_type(c: char) -> Option<LexerTokenType> {
    use LexerTokenType as T;
    Some(match c {
        '+' => T::Plus,
        '-' => T::Minus,
        '*' => T::Star,
        '^' => T::Exponent,
        '%' => T::Percent,
        '&' => T::And,
        '|' => T::Or,
        '!' => T::Exclamation,
        '(' => T::LPar,
        ')' => T::RPar,
        '{' => T::LCPar,
        '}' => T::RCPar,
        '[' => T::LSPar,
        ']' => T::RSPar,
        '.' => T::Dot,
        ',' => T::Punc,
        '=' => T::Equal,
        '~' => T::Tilde,
        ';' => T::SemiColon,
        ':' => T::Colon,
        '\n' => T::EOL,
        '\r' => T::Space,
        '\t' => T::TabSpace,
        '/' => T::Slash,
        '\'' => T::Quote,
        ' ' => T::Space,
        '`' => T::TildeQuote,
        _ => return None,
    })
}

impl LexerTokens {

    pub fn len(&self) -> u64{
        self.pos.end.column.saturating_sub(self.pos.start.column)
    }

    pub fn str_to_token<S:Into<String>>(str:S) -> Vec<Self>{
        let str:String = str.into();
        Self::get_tokens_merged(&str)
    }
    pub fn string_to_token(str:&String) -> Vec<Self>{
        Self::get_tokens_merged(str)
    }

    // does the space merging
    fn get_tokens_merged(str:&String) -> Vec<Self>{
        let tokens = Self::to_token_inner(str);
        Self::merge_spaces(tokens)
    }

    fn new(start:LineColumn,end:LineColumn,tok_type:LexerTokenType) -> Self{
        Self {
            token_type: tok_type,
            pos: Position { start, end }
        }
    }

    /// Tokenizes the text. The last token is always `EOF`.
    ///
    /// * `;` produces a `SemiColon` token and the rest of the line is dropped (comment).
    /// * `"..."` produces one `Str` token (escapes `\n \t \r \0 \\ \" \xNN`); a malformed
    ///   literal produces a `StrErr` token.
    /// * a run of other characters is a `Number` (decimal) or an `Ident`.
    fn to_token_inner(str:&String) -> Vec<LexerTokens>{
        let chars: Vec<char> = str.chars().collect();
        let mut i = 0usize;
        let (mut line, mut column) = (1u64, 1u64);
        let mut tokens = Vec::<LexerTokens>::new();

        while i < chars.len() {
            let c = chars[i];
            let start = LineColumn { line, column };

            if c == '"' {
                i += 1;
                column += 1;
                let mut bytes: Vec<u8> = Vec::new();
                let mut err: Option<String> = None;
                let mut closed = false;
                while i < chars.len() {
                    let ch = chars[i];
                    if ch == '\n' {
                        break;
                    }
                    i += 1;
                    column += 1;
                    if ch == '"' {
                        closed = true;
                        break;
                    }
                    if ch == '\\' {
                        let esc = chars.get(i).copied();
                        match esc {
                            Some('n') => { bytes.push(b'\n'); i += 1; column += 1; }
                            Some('t') => { bytes.push(b'\t'); i += 1; column += 1; }
                            Some('r') => { bytes.push(b'\r'); i += 1; column += 1; }
                            Some('0') => { bytes.push(0); i += 1; column += 1; }
                            Some('\\') => { bytes.push(b'\\'); i += 1; column += 1; }
                            Some('"') => { bytes.push(b'"'); i += 1; column += 1; }
                            Some('x') => {
                                let h1 = chars.get(i + 1).and_then(|c| c.to_digit(16));
                                let h2 = chars.get(i + 2).and_then(|c| c.to_digit(16));
                                if let (Some(a), Some(b)) = (h1, h2) {
                                    bytes.push((a * 16 + b) as u8);
                                    i += 3;
                                    column += 3;
                                } else {
                                    if err.is_none() {
                                        err = Some("`\\x` must be followed by two hex digits".to_string());
                                    }
                                    i += 1;
                                    column += 1;
                                }
                            }
                            Some(o) if o != '\n' => {
                                if err.is_none() {
                                    err = Some(format!("unknown escape `\\{o}`"));
                                }
                                i += 1;
                                column += 1;
                            }
                            _ => {
                                if err.is_none() {
                                    err = Some("unfinished escape at end of line".to_string());
                                }
                            }
                        }
                    } else {
                        let mut buf = [0u8; 4];
                        bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                }
                if !closed && err.is_none() {
                    err = Some("unterminated string literal".to_string());
                }
                let end = LineColumn { line, column };
                let tt = match err {
                    Some(e) => LexerTokenType::StrErr(e),
                    None => LexerTokenType::Str(bytes),
                };
                tokens.push(LexerTokens::new(start, end, tt));
                continue;
            }

            if c == ';' {
                tokens.push(LexerTokens::new(
                    start,
                    LineColumn { line, column: column + 1 },
                    LexerTokenType::SemiColon,
                ));
                // comment: drop everything up to (not including) the newline
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                    column += 1;
                }
                continue;
            }

            if let Some(tt) = sym_token_type(c) {
                let is_eol = tt == LexerTokenType::EOL;
                tokens.push(LexerTokens::new(
                    start,
                    LineColumn { line, column: column + 1 },
                    tt,
                ));
                i += 1;
                if is_eol {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
                continue;
            }

            // identifier / number run
            let mut s = String::new();
            while i < chars.len() {
                let ch = chars[i];
                if ch == '"' || sym_token_type(ch).is_some() {
                    break;
                }
                s.push(ch);
                i += 1;
                column += 1;
            }
            let end = LineColumn { line, column };
            let tt = match i64::from_str_radix(&s, 10) {
                Ok(n) if !s.starts_with('+') && !s.starts_with('-') => LexerTokenType::Number(n),
                _ => LexerTokenType::Ident(s),
            };
            tokens.push(LexerTokens::new(start, end, tt));
        }

        let eof = LineColumn { line, column };
        tokens.push(LexerTokens::new(eof.clone(), eof, LexerTokenType::EOF));
        tokens
    }

    /// merges runs of the same kind of whitespace into one token
    fn merge_spaces(v:Vec<Self>) -> Vec<Self>{
        let mut t: Vec<Self> = Vec::new();
        for x in v {
            let is_ws = x.token_type == LexerTokenType::Space || x.token_type == LexerTokenType::TabSpace;
            if is_ws {
                if let Some(last) = t.last_mut() {
                    if last.token_type == x.token_type && last.pos.end == x.pos.start {
                        last.pos.end = x.pos.end;
                        continue;
                    }
                }
            }
            t.push(x);
        }
        t
    }

    pub fn trim_spaces(v:Vec<Self>) -> Vec<Self>{
        v.into_iter()
            .filter(|x| !(x.token_type == LexerTokenType::Space || x.token_type == LexerTokenType::TabSpace) )
            .collect()
    }

}


#[cfg(test)]
mod test{
    use crate::compiler::ast::lexer::{LexerTokenType as T, LexerTokens};

    fn kinds(s: &str) -> Vec<T> {
        LexerTokens::trim_spaces(LexerTokens::str_to_token(s))
            .into_iter()
            .map(|t| t.token_type)
            .collect()
    }

    #[test]
    fn test_tokenize(){
        let _ = LexerTokens::str_to_token("");
        let _ = LexerTokens::str_to_token("\n        void main(){int a = 10;}  3818  .");
        let _ = LexerTokens::str_to_token("\n        ..,()^+\n        .....");
    }

    #[test]
    fn empty_is_only_eof() {
        assert_eq!(kinds(""), vec![T::EOF]);
    }

    #[test]
    fn sizeofchar(){
        assert_eq!(size_of_val(&'a'), 4);
    }

    #[test]
    fn unicode_ident(){
        let 𐰦 = 21;
        assert_eq!(𐰦, 21);
    }

    #[test]
    fn numbers_and_idents() {
        assert_eq!(
            kinds("mov ra, 10 0x1f"),
            vec![
                T::Ident("mov".into()),
                T::Ident("ra".into()),
                T::Punc,
                T::Number(10),
                T::Ident("0x1f".into()),
                T::EOF
            ]
        );
    }

    #[test]
    fn colon_token() {
        assert_eq!(kinds("a:"), vec![T::Ident("a".into()), T::Colon, T::EOF]);
    }

    #[test]
    fn string_literal_escapes() {
        assert_eq!(
            kinds(r#""a\n\t\r\0\\\"\x41é""#),
            vec![T::Str(vec![b'a', b'\n', b'\t', b'\r', 0, b'\\', b'"', 0x41, 0xc3, 0xa9]), T::EOF]
        );
    }

    #[test]
    fn string_keeps_spaces_and_comment_chars() {
        assert_eq!(kinds(r#""a ; b""#), vec![T::Str(b"a ; b".to_vec()), T::EOF]);
    }

    #[test]
    fn string_errors() {
        assert!(matches!(kinds("\"abc")[0], T::StrErr(_)));
        assert!(matches!(kinds("\"a\\qb\"")[0], T::StrErr(_)));
        assert!(matches!(kinds("\"a\\x4\"")[0], T::StrErr(_)));
        assert!(matches!(kinds("\"a\\")[0], T::StrErr(_)));
    }

    #[test]
    fn comment_dropped_after_semicolon() {
        assert_eq!(
            kinds("nop ; \"unterminated\nret"),
            vec![T::Ident("nop".into()), T::SemiColon, T::EOL, T::Ident("ret".into()), T::EOF]
        );
    }

    #[test]
    fn positions() {
        let t = LexerTokens::trim_spaces(LexerTokens::str_to_token("ab\n  cd"));
        assert_eq!((t[2].pos.start.line, t[2].pos.start.column), (2, 3));
    }
}
