use crate::error::Span;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    /// logical `!`
    Not,
    /// bitwise `~`
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    /// Left binding power (higher binds tighter); all operators are left associative.
    pub fn precedence(self) -> u8 {
        use BinOp::*;
        match self {
            Or => 1,
            And => 3,
            Eq | Ne | Lt | Le | Gt | Ge => 5,
            BitOr => 7,
            BitXor => 9,
            BitAnd => 11,
            Shl | Shr => 13,
            Add | Sub => 15,
            Mul | Div | Rem => 17,
        }
    }
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Shl => "<<",
            Shr => ">>",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }
}

impl UnOp {
    pub fn symbol(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
        }
    }
}

/// Binding power of prefix operators (above every binary operator).
pub(crate) const UNARY_PREC: u8 = 19;
const POSTFIX_PREC: u8 = 21;

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// Equality is structural: spans are ignored.
impl PartialEq for Expr {
    fn eq(&self, other: &Expr) -> bool {
        self.kind == other.kind
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(i128),
    Float(f64),
    /// `true` / `false`
    Bool(bool),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    Call(String, Vec<Expr>),
    Paren(Box<Expr>),
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span }
    }

    /// For `a.b.c` (a `Var` followed by fields) returns the dotted path `"a.b.c"`.
    pub fn dotted_path(&self) -> Option<String> {
        match &self.kind {
            ExprKind::Var(n) => Some(n.clone()),
            ExprKind::Field(b, f) => Some(format!("{}.{}", b.dotted_path()?, f)),
            _ => None,
        }
    }

    fn prec(&self) -> u8 {
        match &self.kind {
            ExprKind::Binary(op, ..) => op.precedence(),
            ExprKind::Unary(..) => UNARY_PREC,
            ExprKind::Int(n) if *n < 0 => UNARY_PREC,
            ExprKind::Float(x) if x.is_sign_negative() => UNARY_PREC,
            _ => 100,
        }
    }

    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, min: u8) -> fmt::Result {
        let paren = self.prec() < min;
        if paren {
            f.write_str("(")?;
        }
        match &self.kind {
            ExprKind::Int(n) => write!(f, "{n}")?,
            ExprKind::Float(x) => write!(f, "{x:?}")?,
            ExprKind::Bool(b) => write!(f, "{b}")?,
            ExprKind::Var(n) => f.write_str(n)?,
            ExprKind::Unary(op, e) => {
                f.write_str(op.symbol())?;
                e.fmt_prec(f, UNARY_PREC)?;
            }
            ExprKind::Binary(op, l, r) => {
                let p = op.precedence();
                l.fmt_prec(f, p)?;
                write!(f, " {} ", op.symbol())?;
                r.fmt_prec(f, p + 1)?;
            }
            ExprKind::Field(b, name) => {
                b.fmt_prec(f, POSTFIX_PREC)?;
                write!(f, ".{name}")?;
            }
            ExprKind::Call(name, args) => {
                write!(f, "{name}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    a.fmt_prec(f, 0)?;
                }
                f.write_str(")")?;
            }
            ExprKind::Paren(e) => {
                f.write_str("(")?;
                e.fmt_prec(f, 0)?;
                f.write_str(")")?;
            }
        }
        if paren {
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// Infix printer. `parse(&e.to_string())` equals `e` structurally.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_prec(f, 0)
    }
}
