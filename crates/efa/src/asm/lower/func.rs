//! Function bodies: registers, labels, operand forms, hidden temporaries, call and ret windows,
//! nregs (efa-syntax.md sections 4, 5, 6 and 7).

use std::collections::HashMap;

use efevm::{FunctionBuilder, Label, ModuleBuilder, Op, Shape};

use super::value::{describe, raw_reg, type_val, Konst, Val};
use super::{func_type_of, Env, Res};
use crate::asm::ast::{FuncDecl, Ident, Inst, Operand, Span, StmtKind, Ty};
use crate::asm::diag::{did_you_mean, Diagnostic};

/// What the module level needs back from one lowered function.
pub(super) struct FuncOut {
    pub body: FunctionBuilder,
    /// The statement span of every emitted instruction (for verification errors).
    pub spans: Vec<Span>,
    /// Named registers in index order.
    pub reg_names: Vec<(u16, String)>,
}

struct Fl<'a, 'm> {
    env: &'a Env<'a>,
    mb: &'m mut ModuleBuilder,
    fb: FunctionBuilder,
    file_id: u32,
    regs: HashMap<String, u16>,
    reg_list: Vec<String>,
    labels: HashMap<String, (Label, Span)>,
    results: Vec<Ty>,
    /// `B`: the first hidden temporary / window register.
    base: usize,
    /// Registers required so far.
    need: usize,
    /// Hidden temporaries used by the current statement.
    temp: usize,
    stmt: Span,
    pending: Vec<(Label, String, Span)>,
    spans: Vec<Span>,
    last_op: Option<Op>,
    errs: Vec<Diagnostic>,
}

/// The slot type of the source operands of an instruction, from its mnemonic: the type named
/// in a conversion suffix (`wrap_i64`, `trunc_f64_s`, `convert_i32_u`, `promote_f32`...), else
/// the type prefix; `memory.*` (no type prefix) is `i64`.
fn slot_ty(op: Op) -> Ty {
    let name = op.name();
    let Some((prefix, rest)) = name.split_once('.') else {
        return Ty::I64;
    };
    for tok in rest.split('_') {
        if let Some(t) = Ty::from_name(tok) {
            return t;
        }
    }
    Ty::from_name(prefix).unwrap_or(Ty::I64)
}

fn scan_raw(op: &Operand, hi: &mut Option<usize>) {
    match op {
        Operand::Name(id) => {
            if let Some(n) = raw_reg(&id.name) {
                if n <= usize::from(u16::MAX) {
                    *hi = Some(hi.map_or(n, |h| h.max(n)));
                }
            }
        }
        Operand::Call(cs) => {
            scan_raw(&cs.callee, hi);
            for a in cs.args.iter().chain(cs.results.iter()) {
                scan_raw(a, hi);
            }
        }
        Operand::Tuple(items, _) => {
            for a in items {
                scan_raw(a, hi);
            }
        }
        _ => {}
    }
}

pub(super) fn lower_func(
    env: &Env,
    mb: &mut ModuleBuilder,
    file_id: u32,
    decl: &FuncDecl,
    diags: &mut Vec<Diagnostic>,
) -> FuncOut {
    let mut errs: Vec<Diagnostic> = Vec::new();
    let mut regs: HashMap<String, u16> = HashMap::new();
    let mut reg_spans: HashMap<String, Span> = HashMap::new();
    let mut reg_list: Vec<String> = Vec::new();
    let mut reg_names: Vec<(u16, String)> = Vec::new();
    let nparams = decl.params.len();
    let mut next = nparams;

    let mut too_many = false;
    let mut declare = |env: &Env, errs: &mut Vec<Diagnostic>, id: &Ident, index: usize| -> bool {
        let Ok(r) = u16::try_from(index) else {
            if !too_many {
                too_many = true;
                errs.push(env.err(id.span, "too many registers"));
            }
            return false;
        };
        if let Some(prev) = reg_spans.get(&id.name) {
            let d = env.err(id.span, format!("duplicate register `{}`", id.name));
            errs.push(env.first_defined(d, *prev));
            return false;
        }
        reg_spans.insert(id.name.clone(), id.span);
        regs.insert(id.name.clone(), r);
        reg_list.push(id.name.clone());
        reg_names.push((r, id.name.clone()));
        true
    };

    for (pos, p) in decl.params.iter().enumerate() {
        let id = &p.name;
        match raw_reg(&id.name) {
            Some(n) if n == pos => {} // an unnamed parameter
            Some(_) => errs.push(env.err(
                id.span,
                format!("parameter `{}` at position {pos} must be named `r{pos}` or a plain name", id.name),
            )),
            None => {
                if env.check_name(id, "parameter", &mut errs) {
                    declare(env, &mut errs, id, pos);
                }
            }
        }
    }
    let mut nregs_decl: Option<(Operand, Span)> = None;
    for s in &decl.body {
        match &s.kind {
            StmtKind::Reg(ids) => {
                for id in ids {
                    if env.check_name(id, "register", &mut errs) {
                        if declare(env, &mut errs, id, next) {
                            next += 1;
                        }
                    }
                }
            }
            StmtKind::Nregs(op) => match &nregs_decl {
                None => nregs_decl = Some((op.clone(), s.span)),
                Some((_, first)) => {
                    let d = env.err(s.span, "duplicate `nregs` statement");
                    errs.push(env.first_defined(d, *first));
                }
            },
            _ => {}
        }
    }
    let declared = next;

    let mut fb = FunctionBuilder::new(0);
    let mut labels: HashMap<String, (Label, Span)> = HashMap::new();
    for s in &decl.body {
        if let StmtKind::Label(id) = &s.kind {
            if !env.check_name(id, "label", &mut errs) {
                continue;
            }
            if let Some((_, prev)) = labels.get(&id.name) {
                let d = env.err(id.span, format!("duplicate label `{}`", id.name));
                errs.push(env.first_defined(d, *prev));
            } else {
                labels.insert(id.name.clone(), (fb.new_label(), id.span));
            }
        }
    }

    let mut hi: Option<usize> = None;
    for s in &decl.body {
        if let StmtKind::Inst(i) = &s.kind {
            for op in &i.operands {
                scan_raw(op, &mut hi);
            }
        }
    }
    let base = hi.map_or(declared, |h| declared.max(h + 1));

    let mut fl = Fl {
        env,
        mb,
        fb,
        file_id,
        regs,
        reg_list,
        labels,
        results: decl.results.iter().map(|t| t.ty).collect(),
        base,
        need: declared,
        temp: 0,
        stmt: decl.name.span,
        pending: Vec::new(),
        spans: Vec::new(),
        last_op: None,
        errs,
    };
    fl.run(decl, nregs_decl);
    diags.append(&mut fl.errs);
    FuncOut { body: fl.fb, spans: fl.spans, reg_names }
}

impl<'a, 'm> Fl<'a, 'm> {
    fn run(&mut self, decl: &FuncDecl, nregs_decl: Option<(Operand, Span)>) {
        let mut last_stmt = decl.name.span;
        for s in &decl.body {
            last_stmt = s.span;
            match &s.kind {
                StmtKind::Label(id) => {
                    if let Some((l, span)) = self.labels.get(&id.name) {
                        if *span == id.span {
                            self.pending.push((*l, id.name.clone(), id.span));
                        }
                    }
                }
                StmtKind::Reg(_) | StmtKind::Nregs(_) => {}
                StmtKind::Inst(i) => {
                    self.stmt = i.mnemonic_span;
                    self.temp = 0;
                    if let Err(e) = self.inst(i) {
                        self.errs.push(e);
                    }
                }
            }
        }

        if self.errs.is_empty() {
            if self.fb.is_empty() {
                let d = self.d(decl.name.span, format!("function `{}` has no instructions", decl.name.name));
                self.errs.push(d);
            } else if let Some((_, name, span)) = self.pending.first().cloned() {
                let d = self.d(span, format!("label `{name}` has no instruction"));
                self.errs.push(d);
            } else if !self.last_op.is_some_and(Op::is_terminator) {
                let at = if decl.end.line != 0 { decl.end } else { last_stmt };
                let d = self
                    .d(
                        at,
                        format!(
                            "function `{}` does not end in a terminator (`ret`, `jmp`, `br_table` or `unreachable`)",
                            decl.name.name
                        ),
                    )
                    .with_note("the function can fall off its end");
                self.errs.push(d);
            }
        }

        let required = self.need;
        let mut count = required;
        if required > usize::from(u16::MAX) {
            let d = self.d(
                decl.name.span,
                format!("function `{}` needs {required} registers, the limit is 65535", decl.name.name),
            );
            self.errs.push(d);
        }
        if let Some((op, _)) = nregs_decl {
            match self.env.int_known(&op) {
                Ok(v) => match usize::try_from(v) {
                    Ok(n) if n <= usize::from(u16::MAX) => {
                        if n < required && self.errs.is_empty() {
                            let d = self.d(
                                op.span(),
                                format!("`nregs {n}` is too small, this function needs {required}"),
                            );
                            self.errs.push(d);
                        }
                        count = n;
                    }
                    _ => {
                        let d = self.d(
                            op.span(),
                            format!("`nregs {v}` is out of range (the limit is 65535)"),
                        );
                        self.errs.push(d);
                    }
                },
                Err(e) => self.errs.push(e),
            }
        }
        self.fb.set_nregs(u16::try_from(count).unwrap_or(u16::MAX));
        // "too many registers" is reported at most once per function.
        let mut seen = false;
        self.errs.retain(|e| {
            let dup = e.message.starts_with("too many registers");
            let keep = !(dup && seen);
            seen |= dup;
            keep
        });
    }

    // ---- small helpers ----

    fn d(&self, span: Span, msg: impl Into<String>) -> Diagnostic {
        self.env.err(span, msg)
    }

    fn touch_to(&mut self, n: usize) {
        self.need = self.need.max(n);
    }

    fn reg_u16(&self, n: usize, span: Span) -> Res<u16> {
        match u16::try_from(n) {
            Ok(r) if r != u16::MAX => Ok(r),
            _ => Err(self.d(span, format!("too many registers: register {n} does not fit (limit is 65535 registers)"))),
        }
    }

    /// A register index that counts as a used register operand (`nregs` is at least `n + 1`).
    fn reg_of(&mut self, n: usize, span: Span) -> Res<u16> {
        let r = self.reg_u16(n, span)?;
        self.touch_to(n + 1);
        Ok(r)
    }

    /// `None` when `id` is not a register name. `touch` is false for call and ret window bases.
    fn lookup_reg(&mut self, id: &Ident, touch: bool) -> Option<Res<u16>> {
        if id.name.contains('.') {
            return None;
        }
        if let Some(n) = raw_reg(&id.name) {
            return Some(if touch { self.reg_of(n, id.span) } else { self.reg_u16(n, id.span) });
        }
        let r = *self.regs.get(&id.name)?;
        if touch {
            self.touch_to(usize::from(r) + 1);
        }
        Some(Ok(r))
    }

    fn unknown_reg(&self, id: &Ident) -> Diagnostic {
        if let Some(d) = self.env.private_err(crate::link::Space::Value, id) {
            return d;
        }
        let what = if id.name.contains('.') { "name" } else { "register" };
        let d = self.d(id.span, format!("unknown {what} `{}`", id.name));
        let cands = self.reg_list.iter().chain(self.env.value_names.iter());
        match did_you_mean(&id.name, cands) {
            Some(s) => d.with_note(format!("did you mean `{s}`?")),
            None => d,
        }
    }

    /// A destination (or any plain register) operand.
    fn dst(&mut self, op: &Operand) -> Res<u16> {
        match op {
            Operand::Name(id) => match self.lookup_reg(id, true) {
                Some(r) => r,
                None if self.env.values.contains_key(&id.name) => Err(self.d(
                    id.span,
                    format!("expected a register, found constant `{}`", id.name),
                )),
                None => Err(self.unknown_reg(id)),
            },
            other => Err(self.d(
                other.span(),
                format!("expected a register, found {}", describe(other)),
            )),
        }
    }

    /// The base register of a call or ret window: not counted as a register operand.
    fn window_base(&mut self, op: &Operand) -> Res<u16> {
        match op {
            Operand::Name(id) => match self.lookup_reg(id, false) {
                Some(r) => r,
                None => Err(self.unknown_reg(id)),
            },
            other => Err(self.d(
                other.span(),
                format!("expected a register, found {}", describe(other)),
            )),
        }
    }

    /// The value of an immediate in a source position.
    fn imm(&self, op: &Operand) -> Res<Val> {
        match op {
            Operand::Int(v, _) => Ok(Val::Int(*v)),
            Operand::Char(c, _) => Ok(Val::Int(*c as i128)),
            Operand::Float(t, _) => Ok(Val::Float(t.clone())),
            Operand::Name(id) => match self.env.values.get(&id.name) {
                Some(v) => Ok(v.val.clone()),
                None => Err(self.unknown_reg(id)),
            },
            Operand::Expr(ce) => self.env.eval_known(ce),
            other => Err(self.d(
                other.span(),
                format!("expected a register or an immediate, found {}", describe(other)),
            )),
        }
    }

    /// An immediate that is never a register (`const.*` values, load/store offsets).
    fn imm_only(&self, op: &Operand) -> Res<Val> {
        match op {
            Operand::Name(id) => match self.env.values.get(&id.name) {
                Some(v) => Ok(v.val.clone()),
                None if self.regs.contains_key(&id.name) || raw_reg(&id.name).is_some() => Err(self
                    .d(id.span, format!("expected an immediate, found register `{}`", id.name))),
                None => Err(self.env.unknown_value(id)),
            },
            other => self.imm(other),
        }
    }

    fn konst(&self, ty: Ty, val: &Val, span: Span) -> Res<Konst> {
        type_val(ty, val).map_err(|m| self.d(span, m))
    }

    fn is_float(&self, op: &Operand) -> bool {
        match op {
            Operand::Float(..) => true,
            Operand::Name(id) => {
                !self.regs.contains_key(&id.name)
                    && matches!(self.env.values.get(&id.name), Some(v) if matches!(v.val, Val::Float(_)))
            }
            _ => false,
        }
    }

    fn temp(&mut self, span: Span) -> Res<u16> {
        let n = self.base + self.temp;
        self.temp += 1;
        self.reg_of(n, span)
    }

    /// A source operand: a register, or an immediate loaded into a hidden temporary as `ty`.
    fn src(&mut self, op: &Operand, ty: Ty) -> Res<u16> {
        if let Operand::Name(id) = op {
            if let Some(r) = self.lookup_reg(id, true) {
                return r;
            }
        }
        let val = self.imm(op)?;
        let k = self.konst(ty, &val, op.span())?;
        let t = self.temp(op.span())?;
        self.e_const(t, k);
        Ok(t)
    }

    /// Like `src` for `mov` and `select`: integer-like immediates are `i64`, floats `f64`.
    fn src_auto(&mut self, op: &Operand) -> Res<u16> {
        let ty = if self.is_float(op) { Ty::F64 } else { Ty::I64 };
        self.src(op, ty)
    }

    fn label_id(&self, id: &Ident) -> Res<Label> {
        match self.labels.get(&id.name) {
            Some((l, _)) => Ok(*l),
            None => {
                let d = self.d(id.span, format!("unknown label `{}`", id.name));
                let names: Vec<&String> = self.labels.keys().collect();
                let mut names = names;
                names.sort();
                Err(match did_you_mean(&id.name, names) {
                    Some(s) => d.with_note(format!("did you mean `{s}`?")),
                    None => d,
                })
            }
        }
    }

    fn label(&self, op: &Operand) -> Res<Label> {
        match op {
            Operand::Name(id) => self.label_id(id),
            other => Err(self.d(
                other.span(),
                format!("expected a label, found {}", describe(other)),
            )),
        }
    }

    fn global(&self, op: &Operand) -> Res<(u32, Ty)> {
        match op {
            Operand::Name(id) => match self.env.globals.get(&id.name) {
                Some(g) => Ok((g.index, g.ty)),
                None => Err(self.env.unknown_global(id)),
            },
            other => Err(self.d(
                other.span(),
                format!("expected a global name, found {}", describe(other)),
            )),
        }
    }

    fn offset(&self, op: Option<&Operand>) -> Res<u32> {
        let Some(op) = op else { return Ok(0) };
        match self.imm_only(op)? {
            Val::Int(v) => u32::try_from(v)
                .map_err(|_| self.d(op.span(), format!("offset {v} does not fit in u32"))),
            Val::Float(_) => Err(self.d(op.span(), "float literal in an integer position")),
        }
    }

    fn ops<'i, const N: usize>(&self, i: &'i Inst) -> Res<&'i [Operand; N]> {
        <&[Operand; N]>::try_from(i.operands.as_slice()).map_err(|_| {
            self.d(
                i.mnemonic_span,
                format!(
                    "wrong number of operands for `{}`: expected {N}, found {}",
                    i.mnemonic,
                    i.operands.len()
                ),
            )
        })
    }

    fn arity_range(&self, i: &Inst, lo: usize, hi: usize) -> Diagnostic {
        self.d(
            i.mnemonic_span,
            format!(
                "wrong number of operands for `{}`: expected {lo} or {hi}, found {}",
                i.mnemonic,
                i.operands.len()
            ),
        )
    }

    // ---- emission: every instruction gets `loc` and its statement span ----

    fn mark(&mut self, op: Op) {
        self.fb.loc(self.file_id, self.stmt.line, self.stmt.column);
        for (l, _, _) in self.pending.drain(..) {
            self.fb.bind(l);
        }
        self.spans.push(self.stmt);
        self.last_op = Some(op);
    }

    fn e_const(&mut self, d: u16, k: Konst) {
        match k {
            Konst::I32(v) => {
                self.mark(Op::ConstI32);
                self.fb.const_i32(d, v);
            }
            Konst::I64(v) => {
                self.mark(Op::ConstI64);
                self.fb.const_i64(d, v);
            }
            Konst::F32(v) => {
                self.mark(Op::ConstF32);
                self.fb.const_f32(d, v);
            }
            Konst::F64(v) => {
                self.mark(Op::ConstF64);
                self.fb.const_f64(d, v);
            }
        }
    }

    fn e_mov(&mut self, d: u16, a: u16) {
        self.mark(Op::Mov);
        self.fb.mov(d, a);
    }

    // ---- instructions ----

    fn inst(&mut self, i: &Inst) -> Res<()> {
        let op = match self.env.ops.get(i.mnemonic.as_str()) {
            Some(op) => *op,
            None => {
                let d = self.d(i.mnemonic_span, format!("unknown instruction `{}`", i.mnemonic));
                return Err(match did_you_mean(&i.mnemonic, Op::ALL.iter().map(|o| o.name())) {
                    Some(s) => d.with_note(format!("did you mean `{s}`?")),
                    None => d,
                });
            }
        };
        match op.shape() {
            Shape::None => {
                self.ops::<0>(i)?;
                self.mark(op);
                self.fb.op0(op);
            }
            Shape::D => {
                let [d] = self.ops::<1>(i)?;
                let d = self.dst(d)?;
                self.mark(op);
                self.fb.op1(op, d);
            }
            Shape::DA => {
                let [d, a] = self.ops::<2>(i)?;
                let d = self.dst(d)?;
                let a = if op == Op::Mov { self.src_auto(a)? } else { self.src(a, slot_ty(op))? };
                self.mark(op);
                self.fb.op2(op, d, a);
            }
            Shape::DAB => {
                let [d, a, b] = self.ops::<3>(i)?;
                let d = self.dst(d)?;
                let ty = slot_ty(op);
                let a = self.src(a, ty)?;
                let b = self.src(b, ty)?;
                self.mark(op);
                self.fb.op3(op, d, a, b);
            }
            Shape::ConstI32 | Shape::ConstI64 | Shape::ConstF32 | Shape::ConstF64 => {
                let [d, v] = self.ops::<2>(i)?;
                let d = self.dst(d)?;
                let ty = match op.shape() {
                    Shape::ConstI32 => Ty::I32,
                    Shape::ConstI64 => Ty::I64,
                    Shape::ConstF32 => Ty::F32,
                    _ => Ty::F64,
                };
                let val = self.imm_only(v)?;
                let k = self.konst(ty, &val, v.span())?;
                self.e_const(d, k);
            }
            Shape::Select => {
                let [d, c, a, b] = self.ops::<4>(i)?;
                let d = self.dst(d)?;
                let c = self.src(c, Ty::I32)?;
                let a = self.src_auto(a)?;
                let b = self.src_auto(b)?;
                self.mark(op);
                self.fb.select(d, c, a, b);
            }
            Shape::Load => {
                let (d, b, o) = match i.operands.as_slice() {
                    [d, b] => (d, b, None),
                    [d, b, o] => (d, b, Some(o)),
                    _ => return Err(self.arity_range(i, 2, 3)),
                };
                let d = self.dst(d)?;
                let off = self.offset(o)?;
                let base = self.src(b, Ty::I64)?;
                self.mark(op);
                self.fb.load(op, d, base, off);
            }
            Shape::Store => {
                let (s, b, o) = match i.operands.as_slice() {
                    [s, b] => (s, b, None),
                    [s, b, o] => (s, b, Some(o)),
                    _ => return Err(self.arity_range(i, 2, 3)),
                };
                let off = self.offset(o)?;
                let s = self.src(s, slot_ty(op))?;
                let base = self.src(b, Ty::I64)?;
                self.mark(op);
                self.fb.store(op, s, base, off);
            }
            Shape::ABC => {
                let [a, b, c] = self.ops::<3>(i)?;
                let a = self.src(a, Ty::I64)?;
                let b = self.src(b, Ty::I64)?;
                let c = self.src(c, Ty::I64)?;
                self.mark(op);
                self.fb.op3(op, a, b, c);
            }
            Shape::GlobalGet => {
                let [d, g] = self.ops::<2>(i)?;
                let d = self.dst(d)?;
                let (gi, _) = self.global(g)?;
                self.mark(op);
                self.fb.global_get(d, gi);
            }
            Shape::GlobalSet => {
                let [g, s] = self.ops::<2>(i)?;
                let (gi, ty) = self.global(g)?;
                let a = self.src(s, ty)?;
                self.mark(op);
                self.fb.global_set(gi, a);
            }
            Shape::Jmp => {
                let [l] = self.ops::<1>(i)?;
                let l = self.label(l)?;
                self.mark(op);
                self.fb.jmp(l);
            }
            Shape::Br => {
                let [a, l] = self.ops::<2>(i)?;
                let l = self.label(l)?;
                let a = self.src(a, Ty::I32)?;
                self.mark(op);
                if op == Op::BrIf {
                    self.fb.br_if(a, l);
                } else {
                    self.fb.br_ifz(a, l);
                }
            }
            Shape::BrTable => {
                let [a, list, def] = self.ops::<3>(i)?;
                let Operand::Labels(ids, _) = list else {
                    return Err(self.d(
                        list.span(),
                        format!("expected a label list `[a, b]`, found {}", describe(list)),
                    ));
                };
                let targets = ids.iter().map(|id| self.label_id(id)).collect::<Res<Vec<_>>>()?;
                let def = self.label(def)?;
                let a = self.src(a, Ty::I32)?;
                self.mark(op);
                self.fb.br_table(a, &targets, def);
            }
            Shape::Call => self.call(i)?,
            Shape::CallIndirect => self.call_indirect(i)?,
            Shape::Ret => self.ret(i)?,
        }
        Ok(())
    }

    fn callee(&self, op: &Operand) -> Res<(Ident, u32, Vec<Ty>, Vec<Ty>)> {
        match op {
            Operand::Name(id) => match self.env.funcs.get(&id.name) {
                Some(s) => Ok((id.clone(), s.index, s.params.clone(), s.results.clone())),
                None => Err(self.env.unknown_function(id)),
            },
            other => Err(self.d(
                other.span(),
                format!("expected a function name, found {}", describe(other)),
            )),
        }
    }

    /// Argument `k` of a window starting at `w`: `mov` of a register or `const.T` of an immediate.
    fn window_arg(&mut self, w: usize, k: usize, op: &Operand, ty: Ty) -> Res<()> {
        let dst = self.reg_of(w + k, op.span())?;
        if let Operand::Name(id) = op {
            if let Some(r) = self.lookup_reg(id, true) {
                let r = r?;
                self.e_mov(dst, r);
                return Ok(());
            }
        }
        let val = self.imm(op)?;
        let k = self.konst(ty, &val, op.span())?;
        self.e_const(dst, k);
        Ok(())
    }

    fn check_sugar(
        &self,
        what: &str,
        span: Span,
        nargs: usize,
        nres: usize,
        params: usize,
        results: usize,
    ) -> Res<()> {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        if nargs != params {
            return Err(self.d(
                span,
                format!("{what} takes {params} argument{}, found {nargs}", s(params)),
            ));
        }
        if nres != 0 && nres != results {
            return Err(self.d(
                span,
                format!("{what} returns {results} value{}, found {nres} result registers", s(results)),
            ));
        }
        Ok(())
    }

    fn result_moves(&mut self, w: usize, regs: &[u16], span: Span) -> Res<()> {
        for (k, r) in regs.iter().enumerate() {
            let src = self.reg_of(w + k, span)?;
            self.e_mov(*r, src);
        }
        Ok(())
    }

    fn call(&mut self, i: &Inst) -> Res<()> {
        match i.operands.as_slice() {
            [Operand::Call(cs)] => {
                let (id, index, params, results) = self.callee(&cs.callee)?;
                self.check_sugar(
                    &format!("`{}`", id.name),
                    cs.span,
                    cs.args.len(),
                    cs.results.len(),
                    params.len(),
                    results.len(),
                )?;
                let res = cs.results.iter().map(|r| self.dst(r)).collect::<Res<Vec<_>>>()?;
                let w = self.base;
                for (k, (a, ty)) in cs.args.iter().zip(params.iter()).enumerate() {
                    self.window_arg(w, k, a, *ty)?;
                }
                let wr = self.reg_u16(w, i.mnemonic_span)?;
                self.touch_to(w + params.len().max(results.len()));
                self.mark(Op::Call);
                self.fb.call(index, wr);
                self.result_moves(w, &res, i.mnemonic_span)
            }
            [f @ Operand::Name(_), b] => {
                let (_, index, params, results) = self.callee(f)?;
                let base = self.window_base(b)?;
                self.touch_to(usize::from(base) + params.len().max(results.len()));
                self.mark(Op::Call);
                self.fb.call(index, base);
                Ok(())
            }
            _ => Err(self.d(
                i.mnemonic_span,
                "`call` takes `f(args) -> results` or `f, BASE`".to_string(),
            )),
        }
    }

    fn call_indirect(&mut self, i: &Inst) -> Res<()> {
        match i.operands.as_slice() {
            [Operand::Signature(sig), Operand::Call(cs)] => {
                let params: Vec<Ty> = sig.params.iter().map(|t| t.ty).collect();
                let results: Vec<Ty> = sig.results.iter().map(|t| t.ty).collect();
                self.check_sugar(
                    "the signature",
                    cs.span,
                    cs.args.len(),
                    cs.results.len(),
                    params.len(),
                    results.len(),
                )?;
                let res = cs.results.iter().map(|r| self.dst(r)).collect::<Res<Vec<_>>>()?;
                // Index: a register, or an i32 immediate loaded after the arguments.
                let reg_idx = match &cs.callee {
                    Operand::Name(id) => self.lookup_reg(id, true).transpose()?,
                    _ => None,
                };
                let imm_idx = match reg_idx {
                    Some(_) => None,
                    None => {
                        let val = self.imm(&cs.callee)?;
                        Some(self.konst(Ty::I32, &val, cs.callee.span())?)
                    }
                };
                let w = self.base;
                for (k, (a, ty)) in cs.args.iter().zip(params.iter()).enumerate() {
                    self.window_arg(w, k, a, *ty)?;
                }
                let wsize = params.len().max(results.len());
                let idx_reg = match (reg_idx, imm_idx) {
                    (Some(r), _) => r,
                    (None, Some(k)) => {
                        let t = self.reg_of(w + wsize, i.mnemonic_span)?;
                        self.e_const(t, k);
                        t
                    }
                    (None, None) => return Ok(()),
                };
                let wr = self.reg_u16(w, i.mnemonic_span)?;
                self.touch_to(w + wsize);
                let ty = self.mb.add_type(func_type_of(&params, &results));
                self.mark(Op::CallIndirect);
                self.fb.call_indirect(ty, idx_reg, wr);
                self.result_moves(w, &res, i.mnemonic_span)
            }
            [Operand::Signature(sig), idx, b] => {
                let params: Vec<Ty> = sig.params.iter().map(|t| t.ty).collect();
                let results: Vec<Ty> = sig.results.iter().map(|t| t.ty).collect();
                let idx_reg = self.src(idx, Ty::I32)?;
                let base = self.window_base(b)?;
                self.touch_to(usize::from(base) + params.len().max(results.len()));
                let ty = self.mb.add_type(func_type_of(&params, &results));
                self.mark(Op::CallIndirect);
                self.fb.call_indirect(ty, idx_reg, base);
                Ok(())
            }
            _ => Err(self.d(
                i.mnemonic_span,
                "`call_indirect` takes `SIG, idx(args) -> results` or `SIG, idx, BASE`".to_string(),
            )),
        }
    }

    fn ret(&mut self, i: &Inst) -> Res<()> {
        let nres = self.results.len();
        match i.operands.as_slice() {
            [] => {
                self.touch_to(nres);
                self.mark(Op::Ret);
                self.fb.ret(0);
            }
            [Operand::Tuple(items, span)] => {
                if items.len() != nres {
                    return Err(self.d(
                        *span,
                        format!(
                            "this function returns {nres} value{}, found {} in the tuple",
                            if nres == 1 { "" } else { "s" },
                            items.len()
                        ),
                    ));
                }
                let w = self.base;
                let tys = self.results.clone();
                for (k, (a, ty)) in items.iter().zip(tys.iter()).enumerate() {
                    self.window_arg(w, k, a, *ty)?;
                }
                let wr = self.reg_u16(w, i.mnemonic_span)?;
                self.touch_to(w + nres);
                self.mark(Op::Ret);
                self.fb.ret(wr);
            }
            [op] => {
                if let Operand::Name(id) = op {
                    if let Some(r) = self.lookup_reg(id, false) {
                        let r = r?;
                        self.touch_to(usize::from(r) + nres);
                        self.mark(Op::Ret);
                        self.fb.ret(r);
                        return Ok(());
                    }
                }
                let val = self.imm(op)?;
                let ty = match self.results.as_slice() {
                    [t] => *t,
                    _ => {
                        return Err(self.d(
                            op.span(),
                            format!(
                                "`ret` with an immediate needs a function with exactly one result, this one has {nres}"
                            ),
                        ))
                    }
                };
                let k = self.konst(ty, &val, op.span())?;
                let w = self.reg_of(self.base, i.mnemonic_span)?;
                self.e_const(w, k);
                self.mark(Op::Ret);
                self.fb.ret(w);
            }
            _ => return Err(self.d(i.mnemonic_span, format!(
                "wrong number of operands for `ret`: expected 0 or 1, found {}",
                i.operands.len()
            ))),
        }
        Ok(())
    }
}
