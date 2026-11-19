//! Constant expressions (efa-syntax 18) and the lazy, dependency-ordered evaluation of the
//! declarations that others read while the file is declared: consts, struct layouts and data
//! sizes.
//!
//! An expression is evaluated by `calc` on `i128` / `f64` values; the names in it are looked
//! up first (in the value name space), then the tree is folded. The result is an untyped
//! [`Val`]; the use site narrows it with `type_val` as it does for a literal.
//!
//! [`Fold`] evaluates a declaration when something needs it (so a const may use a const, a
//! struct or a data size declared further down) and reports a cycle at the use that closes it.
//! A name whose value only the layout knows (a data address, an event id, or a const built
//! from one) is "late": it works where the value is only emitted (instruction immediates,
//! global initial values, `db`/`dw`/`dd`/`dq` values, another late const) but not where
//! the layout itself needs a number (`memory`, `scratch`, `resb`, array lengths, a `pub const`).

use std::collections::HashMap;

use calc::{CalcErrorKind, Value as CV};
use efevm::api::{FieldDesc, StructDesc};

use super::value::{describe, float_text, parse_f64, Val};
use super::{prim_of, Env, Res, VKind};
use crate::asm::ast::{ConstDecl, ConstExpr, DataDecl, DataKind, Ident, Operand, Span, StructDecl};
use crate::asm::diag::Diagnostic;
use crate::link;

/// Why a value is not available.
#[derive(Clone, Debug)]
pub(super) enum Fail {
    /// Known only after the layout; the diagnostic is made by the position that needs the value
    /// now (`Late::message`).
    Late(Late),
    /// A diagnostic was already recorded.
    Reported,
}

#[derive(Clone, Debug)]
pub(super) struct Late {
    /// The data symbol or event id at the bottom of the dependency.
    pub root: String,
    pub what: &'static str,
    /// The const through which the root was reached, if any.
    pub via: Option<String>,
    /// Where the late name was used.
    pub at: Span,
}

impl Late {
    pub(super) fn message(&self) -> String {
        match &self.via {
            None => format!(
                "`{}` is {}, which is only known when the program is linked; it cannot be used here",
                self.root, self.what
            ),
            Some(k) => format!(
                "`{k}` depends on `{}` ({}), which is only known when the program is linked; it cannot be used here",
                self.root, self.what
            ),
        }
    }
}

type FRes<T> = Result<T, Fail>;

enum St<T> {
    Todo,
    Busy,
    Done(T),
}

fn calc_message(k: &CalcErrorKind) -> String {
    match k {
        CalcErrorKind::Overflow => "overflow in a constant expression (integers are computed in 128 bits)".into(),
        CalcErrorKind::DivisionByZero => "division by zero in a constant expression".into(),
        CalcErrorKind::TypeMismatch(m) => format!("{m} (constant expression)"),
        CalcErrorKind::UnknownFunction(n) => {
            format!("unknown function `{n}`: constant expressions have no functions")
        }
        other => format!("invalid constant expression: {other}"),
    }
}

/// The names an expression reads, with their `calc` spans.
fn vars_of(e: &calc::Expr, out: &mut Vec<(String, calc::Span)>) {
    use calc::ExprKind as K;
    match &e.kind {
        K::Var(n) => out.push((n.clone(), e.span)),
        K::Unary(_, a) | K::Paren(a) | K::Field(a, _) => vars_of(a, out),
        K::Binary(_, a, b) => {
            vars_of(a, out);
            vars_of(b, out);
        }
        K::Call(_, args) => args.iter().for_each(|a| vars_of(a, out)),
        K::Int(_) | K::Float(_) | K::Bool(_) => {}
    }
}

impl Env<'_> {
    pub(super) fn to_calc(&self, v: &Val, span: Span) -> Res<CV> {
        match v {
            Val::Int(n) => Ok(CV::Int(*n)),
            Val::Float(t) => parse_f64(t).map(CV::Float).map_err(|m| self.err(span, m)),
        }
    }

    /// Folds the expression with the values of `vars`.
    pub(super) fn fold_expr(&self, ce: &ConstExpr, vars: &calc::Vars) -> Res<Val> {
        match calc::eval(&ce.expr, vars) {
            Ok(CV::Int(n)) => Ok(Val::Int(n)),
            Ok(CV::Float(x)) => Ok(Val::Float(float_text(x))),
            Ok(CV::Bool(_)) => {
                Err(self.err(ce.span, "a constant expression must be a number, found a truth value"))
            }
            Err(e) => Err(self.err(ce.span_of(e.span), calc_message(&e.kind))),
        }
    }

    /// Evaluates an expression once every value is known (after the layout).
    pub(super) fn eval_known(&self, ce: &ConstExpr) -> Res<Val> {
        let mut names = Vec::new();
        vars_of(&ce.expr, &mut names);
        let mut vars = calc::Vars::new();
        for (name, sp) in names {
            let span = ce.span_of(sp);
            match self.values.get(&name) {
                Some(v) => {
                    let c = self.to_calc(&v.val, span)?;
                    vars.set(name, c);
                }
                None => return Err(self.unknown_value(&Ident { name, span })),
            }
        }
        self.fold_expr(ce, &vars)
    }

    /// An integer-valued operand once every value is known.
    pub(super) fn int_known(&self, op: &Operand) -> Res<i128> {
        match self.value_of(op)? {
            Val::Int(v) => Ok(v),
            Val::Float(_) => Err(self.err(op.span(), "expected an integer, found a float")),
        }
    }
}

/// The lazily evaluated declarations of one file.
pub(super) struct Fold<'a> {
    consts: Vec<&'a ConstDecl>,
    cstate: Vec<St<FRes<Val>>>,
    const_by_name: HashMap<&'a str, usize>,
    structs: Vec<&'a StructDecl>,
    sstate: Vec<St<(Option<StructDesc>, bool)>>,
    struct_by_name: HashMap<&'a str, usize>,
    datas: Vec<&'a DataDecl>,
    dstate: Vec<St<u64>>,
    data_by_name: HashMap<&'a str, usize>,
    /// Names being evaluated, for the cycle message.
    stack: Vec<String>,
    /// After the layout: data addresses and event ids are known (`env.values` has them).
    layout_known: bool,
}

impl<'a> Fold<'a> {
    /// For the declare phase: every const, struct and data item of the file.
    pub(super) fn new(
        consts: &[&'a ConstDecl],
        structs: &[&'a StructDecl],
        datas: &[&'a DataDecl],
    ) -> Fold<'a> {
        let mut f = Fold::with_consts(consts, false);
        f.structs = structs.to_vec();
        f.datas = datas.to_vec();
        f.sstate = structs.iter().map(|_| St::Todo).collect();
        f.dstate = datas.iter().map(|_| St::Todo).collect();
        for (i, s) in structs.iter().enumerate() {
            f.struct_by_name.entry(s.name.name.as_str()).or_insert(i);
        }
        for (i, d) in datas.iter().enumerate() {
            f.data_by_name.entry(d.name.name.as_str()).or_insert(i);
        }
        f
    }

    /// For the resolve phase: only the consts that were late.
    pub(super) fn after_layout(late: &[&'a ConstDecl]) -> Fold<'a> {
        Fold::with_consts(late, true)
    }

    fn with_consts(consts: &[&'a ConstDecl], layout_known: bool) -> Fold<'a> {
        let mut by = HashMap::new();
        for (i, c) in consts.iter().enumerate() {
            by.entry(c.name.name.as_str()).or_insert(i);
        }
        Fold {
            consts: consts.to_vec(),
            cstate: consts.iter().map(|_| St::Todo).collect(),
            const_by_name: by,
            structs: Vec::new(),
            sstate: Vec::new(),
            struct_by_name: HashMap::new(),
            datas: Vec::new(),
            dstate: Vec::new(),
            data_by_name: HashMap::new(),
            stack: Vec::new(),
            layout_known,
        }
    }

    // ---- values ---------------------------------------------------------------------------

    /// The value of the `i`-th const (evaluated once).
    pub(super) fn const_value(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, i: usize) -> FRes<Val> {
        match self.cstate.get(i) {
            Some(St::Done(r)) => return r.clone(),
            Some(St::Busy) | None => return Err(Fail::Reported),
            Some(St::Todo) => {}
        }
        let c = self.consts[i];
        self.cstate[i] = St::Busy;
        self.stack.push(c.name.name.clone());
        let r = self.operand_value(env, diags, &c.value);
        self.stack.pop();
        self.cstate[i] = St::Done(r.clone());
        r
    }

    /// The value of an operand in a constant position: a literal, a name or an expression.
    pub(super) fn operand_value(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, op: &Operand) -> FRes<Val> {
        match op {
            Operand::Int(v, _) => Ok(Val::Int(*v)),
            Operand::Char(c, _) => Ok(Val::Int(*c as i128)),
            Operand::Float(t, _) => Ok(Val::Float(t.clone())),
            Operand::Name(id) => self.lookup(env, diags, &id.name, id.span),
            Operand::Expr(ce) => self.expression(env, diags, ce),
            other => {
                diags.push(env.err(
                    other.span(),
                    format!("expected an integer, a float or a constant, found {}", describe(other)),
                ));
                Err(Fail::Reported)
            }
        }
    }

    /// An integer that the layout needs now (`memory`, `scratch`, `resb`, array lengths).
    pub(super) fn int_now(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, op: &Operand) -> Option<i128> {
        match self.operand_value(env, diags, op) {
            Ok(Val::Int(n)) => Some(n),
            Ok(Val::Float(_)) => {
                diags.push(env.err(op.span(), "expected an integer, found a float"));
                None
            }
            Err(Fail::Late(l)) => {
                diags.push(env.err(l.at, l.message()));
                None
            }
            Err(Fail::Reported) => None,
        }
    }

    fn expression(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, ce: &ConstExpr) -> FRes<Val> {
        let mut names = Vec::new();
        vars_of(&ce.expr, &mut names);
        let mut vars = calc::Vars::new();
        let mut late: Option<Late> = None;
        for (name, sp) in names {
            if vars.map.contains_key(&name) {
                continue;
            }
            let span = ce.span_of(sp);
            match self.lookup(env, diags, &name, span) {
                Ok(v) => match env.to_calc(&v, span) {
                    Ok(c) => {
                        vars.set(name, c);
                    }
                    Err(d) => {
                        diags.push(d);
                        return Err(Fail::Reported);
                    }
                },
                Err(Fail::Late(l)) => {
                    late.get_or_insert(l);
                }
                Err(Fail::Reported) => return Err(Fail::Reported),
            }
        }
        if let Some(l) = late {
            return Err(Fail::Late(l));
        }
        match env.fold_expr(ce, &vars) {
            Ok(v) => Ok(v),
            Err(d) => {
                diags.push(d);
                Err(Fail::Reported)
            }
        }
    }

    fn cycle(&self, env: &Env, diags: &mut Vec<Diagnostic>, name: &str, span: Span) -> Fail {
        let from = self.stack.iter().position(|n| n == name).unwrap_or(0);
        let mut path: Vec<&str> = self.stack[from..].iter().map(String::as_str).collect();
        path.push(name);
        diags.push(
            env.err(span, format!("`{name}` is defined in terms of itself"))
                .with_note(format!("cycle: {}", path.join(" -> "))),
        );
        Fail::Reported
    }

    /// The value of the name `name` used at `span`.
    fn lookup(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, name: &str, span: Span) -> FRes<Val> {
        if let Some(&i) = self.const_by_name.get(name) {
            if matches!(self.cstate[i], St::Busy) {
                return Err(self.cycle(env, diags, name, span));
            }
            return match self.const_value(env, diags, i) {
                Err(Fail::Late(mut l)) => {
                    l.via = Some(name.to_string());
                    l.at = span;
                    Err(Fail::Late(l))
                }
                r => r,
            };
        }
        if !self.layout_known {
            if self.data_by_name.contains_key(name) {
                return Err(Fail::Late(Late { root: name.into(), what: "a data address", via: None, at: span }));
            }
            if let Some(&i) = name.strip_suffix(".len").and_then(|b| self.data_by_name.get(b)) {
                if matches!(self.dstate[i], St::Busy) {
                    return Err(self.cycle(env, diags, name, span));
                }
                return Ok(Val::Int(i128::from(self.data_len(env, diags, i))));
            }
            if let Some(&i) = name.split_once('.').and_then(|(s, _)| self.struct_by_name.get(s)) {
                if matches!(self.sstate[i], St::Busy) {
                    return Err(self.cycle(env, diags, name, span));
                }
                let (desc, _) = self.struct_layout(env, diags, i);
                match desc {
                    Some(d) => {
                        if let Some((_, v)) = link::struct_consts(&d).into_iter().find(|(n, _)| n == name) {
                            return Ok(Val::Int(v));
                        }
                    }
                    None => return Err(Fail::Reported),
                }
            }
        }
        match env.values.get(name) {
            Some(v) => match v.kind {
                VKind::Const | VKind::DataLen | VKind::LateConst => Ok(v.val.clone()),
                VKind::Data if self.layout_known => Ok(v.val.clone()),
                VKind::Event if self.layout_known => Ok(v.val.clone()),
                VKind::Data => {
                    Err(Fail::Late(Late { root: name.into(), what: "a data address", via: None, at: span }))
                }
                VKind::Event => Err(Fail::Late(Late { root: name.into(), what: "an event id", via: None, at: span })),
            },
            None => {
                diags.push(env.unknown_value(&Ident { name: name.to_string(), span }));
                Err(Fail::Reported)
            }
        }
    }

    // ---- data sizes -----------------------------------------------------------------------

    /// The size in bytes the `i`-th data item occupies (depends only on the count and kind of
    /// its values; `resb` evaluates its size).
    pub(super) fn data_len(&mut self, env: &Env, diags: &mut Vec<Diagnostic>, i: usize) -> u64 {
        match self.dstate.get(i) {
            Some(St::Done(n)) => return *n,
            Some(St::Busy) | None => return 0,
            Some(St::Todo) => {}
        }
        let d = self.datas[i];
        self.dstate[i] = St::Busy;
        self.stack.push(d.name.name.clone());
        let n = d.values.len() as u64;
        let len = match d.kind {
            DataKind::Resb => match d.values.as_slice() {
                [one] => match self.int_now(env, diags, one) {
                    Some(v) => match u64::try_from(v).ok().filter(|c| *c <= u64::from(u32::MAX)) {
                        Some(c) => c,
                        None => {
                            diags.push(env.err(one.span(), format!("`resb` size {v} is out of range")));
                            0
                        }
                    },
                    None => 0,
                },
                _ => {
                    diags.push(env.err(
                        d.kind_span,
                        format!("`resb` takes exactly one size, found {}", d.values.len()),
                    ));
                    0
                }
            },
            _ => {
                if d.values.is_empty() {
                    diags.push(env.err(d.kind_span, format!("`{}` needs at least one value", d.kind.name())));
                }
                match d.kind {
                    DataKind::Db => d
                        .values
                        .iter()
                        .map(|v| match v {
                            Operand::Str(b, _) => b.len() as u64,
                            _ => 1,
                        })
                        .sum(),
                    DataKind::Dw => 2 * n,
                    DataKind::Dd => 4 * n,
                    _ => 8 * n,
                }
            }
        };
        self.stack.pop();
        self.dstate[i] = St::Done(len);
        len
    }

    // ---- struct layouts -------------------------------------------------------------------

    /// The C layout of the `i`-th struct (computed once): the descriptor (`None` after an
    /// error) and whether the name counts as declared (`false` only for a struct that is too
    /// large). The caller has already checked the struct's name.
    pub(super) fn struct_layout(
        &mut self,
        env: &Env,
        diags: &mut Vec<Diagnostic>,
        i: usize,
    ) -> (Option<StructDesc>, bool) {
        match self.sstate.get(i) {
            Some(St::Done(r)) => return r.clone(),
            Some(St::Busy) | None => return (None, true),
            Some(St::Todo) => {}
        }
        let s = self.structs[i];
        self.sstate[i] = St::Busy;
        self.stack.push(s.name.name.clone());
        let r = self.layout_fields(env, diags, s);
        self.stack.pop();
        self.sstate[i] = St::Done(r.clone());
        r
    }

    fn layout_fields(
        &mut self,
        env: &Env,
        diags: &mut Vec<Diagnostic>,
        s: &StructDecl,
    ) -> (Option<StructDesc>, bool) {
        let name = &s.name.name;
        let mut fields: Vec<FieldDesc> = Vec::new();
        let mut field_spans: HashMap<&str, Span> = HashMap::new();
        let mut cursor: u64 = 0;
        let mut align: u32 = 1;
        let mut ok = true;
        for f in &s.fields {
            if !env.check_name(&f.name, "field", diags) {
                ok = false;
                continue;
            }
            if let Some(prev) = field_spans.get(f.name.name.as_str()) {
                let d = env.err(f.name.span, format!("duplicate field `{}` in struct `{name}`", f.name.name));
                diags.push(env.first_defined(d, *prev));
                ok = false;
                continue;
            }
            field_spans.insert(&f.name.name, f.name.span);
            let Some(prim) = prim_of(&f.prim.name) else {
                diags.push(
                    env.err(f.prim.span, format!("unknown field type `{}`", f.prim.name))
                        .with_note("use u8 i8 u16 i16 u32 i32 u64 i64 f32 f64 bool or entity"),
                );
                ok = false;
                continue;
            };
            let count = match &f.count {
                None => 1u32,
                Some(op) => match self.int_now(env, diags, op) {
                    Some(n) if (1..=i128::from(u32::MAX)).contains(&n) => n as u32,
                    Some(n) => {
                        diags.push(env.err(op.span(), format!("array length {n} must be at least 1")));
                        ok = false;
                        continue;
                    }
                    None => {
                        ok = false;
                        continue;
                    }
                },
            };
            let size = u64::from(prim.size());
            let offset = link::layout::align_up(cursor, size);
            cursor = offset.saturating_add(size.saturating_mul(u64::from(count)));
            align = align.max(prim.size());
            let mut fd = FieldDesc::new(f.name.name.clone(), u32::try_from(offset).unwrap_or(u32::MAX), prim, count);
            if let Some(doc) = &f.doc {
                fd = fd.with_doc(String::from_utf8_lossy(doc));
            }
            fields.push(fd);
        }
        if s.fields.is_empty() {
            diags.push(env.err(s.name.span, format!("struct `{name}` needs at least one field")));
            ok = false;
        }
        let size = link::layout::align_up(cursor, u64::from(align));
        let Ok(size) = u32::try_from(size) else {
            diags.push(env.err(s.name.span, format!("struct `{name}` is too large ({size} bytes)")));
            return (None, false);
        };
        if !ok {
            return (None, true);
        }
        let mut desc = StructDesc::new(name.clone(), size, align);
        desc.fields = fields;
        if let Some(doc) = &s.doc {
            desc.doc = String::from_utf8_lossy(doc).into_owned();
        }
        (Some(desc), true)
    }
}
