use crate::ast::{BinOp, Expr, ExprKind, UnOp};
use crate::error::{CalcError, CalcErrorKind, Span};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Int(i128),
    Float(f64),
    Bool(bool),
}

impl From<i128> for Value {
    fn from(v: i128) -> Value {
        Value::Int(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Value {
        Value::Int(v as i128)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Value {
        Value::Int(v as i128)
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Value {
        Value::Int(v as i128)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Value {
        Value::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x:?}"),
            Value::Bool(b) => write!(f, "{b}"),
        }
    }
}

/// Fixed-width integer types of the target VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntWidth {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntWidth {
    pub fn bits(self) -> u32 {
        use IntWidth::*;
        match self {
            I8 | U8 => 8,
            I16 | U16 => 16,
            I32 | U32 => 32,
            I64 | U64 => 64,
        }
    }
    pub fn signed(self) -> bool {
        matches!(self, IntWidth::I8 | IntWidth::I16 | IntWidth::I32 | IntWidth::I64)
    }
    /// Inclusive `(min, max)`.
    pub fn range(self) -> (i128, i128) {
        let b = self.bits();
        if self.signed() {
            (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1)
        } else {
            (0, (1i128 << b) - 1)
        }
    }
    /// `"i8"`, `"u32"`, ...
    pub fn from_name(s: &str) -> Option<IntWidth> {
        use IntWidth::*;
        Some(match s {
            "i8" => I8,
            "i16" => I16,
            "i32" => I32,
            "i64" => I64,
            "u8" => U8,
            "u16" => U16,
            "u32" => U32,
            "u64" => U64,
            _ => return None,
        })
    }
}

impl Value {
    /// Range-checks an integer value for `width` (no implicit float truncation or bool conversion).
    /// Errors carry `Span::default()`; use [`CalcError::with_span`] to attach a location.
    pub fn to_int_of(self, width: IntWidth) -> Result<i128, CalcError> {
        match self {
            Value::Int(n) => {
                let (lo, hi) = width.range();
                if n < lo || n > hi {
                    Err(CalcError::new(CalcErrorKind::Overflow, Span::default()))
                } else {
                    Ok(n)
                }
            }
            other => Err(CalcError::mismatch(
                format!("expected integer, found {}", other.type_name()),
                Span::default(),
            )),
        }
    }

    /// Int or Float as `f32`; Overflow if a finite value does not fit.
    pub fn to_f32(self) -> Result<f32, CalcError> {
        let x = self.to_f64()?;
        let y = x as f32;
        if x.is_finite() && !y.is_finite() {
            return Err(CalcError::new(CalcErrorKind::Overflow, Span::default()));
        }
        Ok(y)
    }

    /// Int (rounded to nearest) or Float as `f64`.
    pub fn to_f64(self) -> Result<f64, CalcError> {
        match self {
            Value::Int(n) => Ok(n as f64),
            Value::Float(x) => Ok(x),
            Value::Bool(_) => {
                Err(CalcError::mismatch("expected number, found bool", Span::default()))
            }
        }
    }

    pub fn type_name(self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
        }
    }

    /// Literal expression for this value.
    pub fn to_expr(self, span: Span) -> Expr {
        Expr::new(
            match self {
                Value::Int(n) => ExprKind::Int(n),
                Value::Float(x) => ExprKind::Float(x),
                Value::Bool(b) => ExprKind::Bool(b),
            },
            span,
        )
    }
}

/// Name resolution for [`eval`] / [`simplify`]. `a.b.c` is looked up as the single name `"a.b.c"`.
/// Functions are assumed pure (`simplify` calls them when all arguments are constant).
pub trait Env {
    fn var(&self, name: &str) -> Option<Value>;
    fn call(&self, name: &str, args: &[Value]) -> Option<Result<Value, CalcError>>;
}

/// Environment with nothing defined.
pub struct NoEnv;

impl Env for NoEnv {
    fn var(&self, _: &str) -> Option<Value> {
        None
    }
    fn call(&self, _: &str, _: &[Value]) -> Option<Result<Value, CalcError>> {
        None
    }
}

/// HashMap-backed variable environment (no functions).
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub map: HashMap<String, Value>,
}

impl Vars {
    pub fn new() -> Vars {
        Vars::default()
    }
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        self.map.insert(name.into(), value.into())
    }
    pub fn unset(&mut self, name: &str) -> Option<Value> {
        self.map.remove(name)
    }
}

impl Env for Vars {
    fn var(&self, name: &str) -> Option<Value> {
        self.map.get(name).copied()
    }
    fn call(&self, _: &str, _: &[Value]) -> Option<Result<Value, CalcError>> {
        None
    }
}

fn overflow(span: Span) -> CalcError {
    CalcError::new(CalcErrorKind::Overflow, span)
}

pub(crate) fn apply_unary(op: UnOp, v: Value, span: Span) -> Result<Value, CalcError> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => n.checked_neg().map(Value::Int).ok_or_else(|| overflow(span)),
        (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (UnOp::BitNot, Value::Int(n)) => Ok(Value::Int(!n)),
        (op, v) => Err(CalcError::mismatch(
            format!("`{}` cannot be applied to {}", op.symbol(), v.type_name()),
            span,
        )),
    }
}

pub(crate) fn apply_binary(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, CalcError> {
    use BinOp::*;
    let mismatch = || {
        CalcError::mismatch(
            format!("`{}` cannot be applied to {} and {}", op.symbol(), l.type_name(), r.type_name()),
            span,
        )
    };
    match op {
        And | Or => match (l, r) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(if op == And { a && b } else { a || b })),
            _ => Err(mismatch()),
        },
        Add | Sub | Mul | Div => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                let v = match op {
                    Add => a.checked_add(b),
                    Sub => a.checked_sub(b),
                    Mul => a.checked_mul(b),
                    _ => {
                        if b == 0 {
                            return Err(CalcError::new(CalcErrorKind::DivisionByZero, span));
                        }
                        a.checked_div(b)
                    }
                };
                v.map(Value::Int).ok_or_else(|| overflow(span))
            }
            (Value::Bool(_), _) | (_, Value::Bool(_)) => Err(mismatch()),
            _ => {
                let (a, b) = (l.to_f64().unwrap(), r.to_f64().unwrap());
                Ok(Value::Float(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    _ => {
                        if b == 0.0 {
                            return Err(CalcError::new(CalcErrorKind::DivisionByZero, span));
                        }
                        a / b
                    }
                }))
            }
        },
        Rem | Shl | Shr | BitAnd | BitOr | BitXor => match (l, r) {
            (Value::Int(a), Value::Int(b)) => match op {
                Rem => {
                    if b == 0 {
                        Err(CalcError::new(CalcErrorKind::DivisionByZero, span))
                    } else {
                        a.checked_rem(b).map(Value::Int).ok_or_else(|| overflow(span))
                    }
                }
                Shl | Shr => {
                    let amt = u32::try_from(b).ok().filter(|&s| s < 128).ok_or_else(|| overflow(span))?;
                    Ok(Value::Int(if op == Shl { a << amt } else { a >> amt }))
                }
                BitAnd => Ok(Value::Int(a & b)),
                BitOr => Ok(Value::Int(a | b)),
                _ => Ok(Value::Int(a ^ b)),
            },
            _ => Err(mismatch()),
        },
        Eq | Ne | Lt | Le | Gt | Ge => {
            use std::cmp::Ordering;
            let ord: Option<Ordering> = match (l, r) {
                (Value::Int(a), Value::Int(b)) => Some(a.cmp(&b)),
                (Value::Bool(a), Value::Bool(b)) if matches!(op, Eq | Ne) => Some(a.cmp(&b)),
                (Value::Bool(_), _) | (_, Value::Bool(_)) => return Err(mismatch()),
                _ => l.to_f64().unwrap().partial_cmp(&r.to_f64().unwrap()),
            };
            Ok(Value::Bool(match (op, ord) {
                (Eq, o) => o == Some(Ordering::Equal),
                (Ne, o) => o != Some(Ordering::Equal),
                (_, None) => false,
                (Lt, Some(o)) => o == Ordering::Less,
                (Le, Some(o)) => o != Ordering::Greater,
                (Gt, Some(o)) => o == Ordering::Greater,
                (_, Some(o)) => o != Ordering::Less,
            }))
        }
    }
}

/// Evaluates `expr`; errors carry the span of the offending node.
pub fn eval(expr: &Expr, env: &dyn Env) -> Result<Value, CalcError> {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Int(n) => Ok(Value::Int(*n)),
        ExprKind::Float(x) => Ok(Value::Float(*x)),
        ExprKind::Bool(b) => Ok(Value::Bool(*b)),
        ExprKind::Var(n) => env
            .var(n)
            .ok_or_else(|| CalcError::new(CalcErrorKind::UnknownVariable(n.clone()), span)),
        ExprKind::Paren(e) => eval(e, env),
        ExprKind::Field(base, _) => match expr.dotted_path() {
            Some(path) => env
                .var(&path)
                .ok_or_else(|| CalcError::new(CalcErrorKind::UnknownVariable(path), span)),
            None => Err(CalcError::mismatch("field access on a non-name expression", base.span)),
        },
        ExprKind::Unary(op, e) => apply_unary(*op, eval(e, env)?, span),
        ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
            let lv = eval(l, env)?;
            match (op, lv) {
                (BinOp::And, Value::Bool(false)) => Ok(Value::Bool(false)),
                (BinOp::Or, Value::Bool(true)) => Ok(Value::Bool(true)),
                _ => apply_binary(*op, lv, eval(r, env)?, span),
            }
        }
        ExprKind::Binary(op, l, r) => apply_binary(*op, eval(l, env)?, eval(r, env)?, span),
        ExprKind::Call(name, args) => {
            let vals = args.iter().map(|a| eval(a, env)).collect::<Result<Vec<_>, _>>()?;
            match env.call(name, &vals) {
                Some(r) => r.map_err(|e| e.with_span(span)),
                None => Err(CalcError::new(CalcErrorKind::UnknownFunction(name.clone()), span)),
            }
        }
    }
}

fn literal(e: &Expr) -> Option<Value> {
    match e.kind {
        ExprKind::Int(n) => Some(Value::Int(n)),
        ExprKind::Float(x) => Some(Value::Float(x)),
        ExprKind::Bool(b) => Some(Value::Bool(b)),
        _ => None,
    }
}

/// Folds constant subtrees (using `env` for known variables/functions) and keeps the rest.
/// Applies `x+0`, `0+x`, `x-0`, `x*1`, `1*x`, `x/1`, `x*0`, `0*x` (integer literals only),
/// and `false && _`, `true || _`. Errors (overflow, `/0`, type errors in constant parts)
/// propagate with the span of the offending node.
pub fn simplify(expr: &Expr, env: &dyn Env) -> Result<Expr, CalcError> {
    let span = expr.span;
    let node = |k: ExprKind| Expr::new(k, span);
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) => Ok(expr.clone()),
        ExprKind::Var(n) => Ok(match env.var(n) {
            Some(v) => v.to_expr(span),
            None => expr.clone(),
        }),
        ExprKind::Field(base, name) => match expr.dotted_path() {
            Some(path) => Ok(match env.var(&path) {
                Some(v) => v.to_expr(span),
                None => expr.clone(),
            }),
            None => Ok(node(ExprKind::Field(Box::new(simplify(base, env)?), name.clone()))),
        },
        ExprKind::Paren(e) => {
            let s = simplify(e, env)?;
            Ok(if literal(&s).is_some() { Expr { span, ..s } } else { node(ExprKind::Paren(Box::new(s))) })
        }
        ExprKind::Unary(op, e) => {
            let s = simplify(e, env)?;
            match literal(&s) {
                Some(v) => Ok(apply_unary(*op, v, span)?.to_expr(span)),
                None => Ok(node(ExprKind::Unary(*op, Box::new(s)))),
            }
        }
        ExprKind::Binary(op, l, r) => {
            let ls = simplify(l, env)?;
            if let Some(Value::Bool(b)) = literal(&ls) {
                match (op, b) {
                    (BinOp::And, false) | (BinOp::Or, true) => return Ok(Value::Bool(b).to_expr(span)),
                    _ => {}
                }
            }
            let rs = simplify(r, env)?;
            if let (Some(a), Some(b)) = (literal(&ls), literal(&rs)) {
                return Ok(apply_binary(*op, a, b, span)?.to_expr(span));
            }
            let int = |e: &Expr, n: i128| matches!(e.kind, ExprKind::Int(m) if m == n);
            let keep_l = (matches!(op, BinOp::Add | BinOp::Sub) && int(&rs, 0))
                || (matches!(op, BinOp::Mul | BinOp::Div) && int(&rs, 1));
            let keep_r = (*op == BinOp::Add && int(&ls, 0)) || (*op == BinOp::Mul && int(&ls, 1));
            if keep_l {
                return Ok(ls);
            }
            if keep_r {
                return Ok(rs);
            }
            if *op == BinOp::Mul && (int(&ls, 0) || int(&rs, 0)) {
                return Ok(Value::Int(0).to_expr(span));
            }
            Ok(node(ExprKind::Binary(*op, Box::new(ls), Box::new(rs))))
        }
        ExprKind::Call(name, args) => {
            let sargs = args.iter().map(|a| simplify(a, env)).collect::<Result<Vec<_>, _>>()?;
            let vals: Option<Vec<Value>> = sargs.iter().map(literal).collect();
            if let Some(vals) = vals
                && let Some(r) = env.call(name, &vals)
            {
                return Ok(r.map_err(|e| e.with_span(span))?.to_expr(span));
            }
            Ok(node(ExprKind::Call(name.clone(), sargs)))
        }
    }
}
