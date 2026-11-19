use std::fmt;

/// Byte range `start..end` into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }
    /// Smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalcErrorKind {
    /// A token (or character) that is not valid here; carries its text.
    UnexpectedToken(String),
    UnexpectedEnd,
    /// An opening bracket that is never closed (span = the opening bracket).
    UnclosedBracket,
    InvalidNumber(String),
    UnknownVariable(String),
    UnknownFunction(String),
    DivisionByZero,
    Overflow,
    TypeMismatch(String),
    /// Expression nested deeper than the parser limit.
    TooDeep,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalcError {
    pub kind: CalcErrorKind,
    pub span: Span,
}

impl CalcError {
    pub fn new(kind: CalcErrorKind, span: Span) -> CalcError {
        CalcError { kind, span }
    }
    pub fn with_span(mut self, span: Span) -> CalcError {
        self.span = span;
        self
    }
    pub(crate) fn mismatch(msg: impl Into<String>, span: Span) -> CalcError {
        CalcError::new(CalcErrorKind::TypeMismatch(msg.into()), span)
    }

    /// Multi-line message with the source line and a caret underline.
    pub fn render(&self, src: &str) -> String {
        let start = self.span.start.min(src.len());
        let end = self.span.end.clamp(start, src.len());
        let (start, end) = (floor_boundary(src, start), floor_boundary(src, end));
        let col = src[..start].chars().count();
        let width = src[start..end].chars().count().max(1);
        format!("error: {}\n  {}\n  {}{}", self.kind, src, " ".repeat(col), "^".repeat(width))
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl fmt::Display for CalcErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalcErrorKind::UnexpectedToken(t) => write!(f, "unexpected token `{t}`"),
            CalcErrorKind::UnexpectedEnd => write!(f, "unexpected end of input"),
            CalcErrorKind::UnclosedBracket => write!(f, "unclosed bracket"),
            CalcErrorKind::InvalidNumber(t) => write!(f, "invalid number `{t}`"),
            CalcErrorKind::UnknownVariable(n) => write!(f, "unknown variable `{n}`"),
            CalcErrorKind::UnknownFunction(n) => write!(f, "unknown function `{n}`"),
            CalcErrorKind::DivisionByZero => write!(f, "division by zero"),
            CalcErrorKind::Overflow => write!(f, "overflow"),
            CalcErrorKind::TypeMismatch(m) => write!(f, "type mismatch: {m}"),
            CalcErrorKind::TooDeep => write!(f, "expression nested too deeply"),
        }
    }
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}..{}", self.kind, self.span.start, self.span.end)
    }
}

impl std::error::Error for CalcError {}
