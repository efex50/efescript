//! Function bodies: structured wasm control flow to efevm basic blocks and labels.
//!
//! Register model. A wasm function with `P` parameters and `L` other locals gets registers
//! `r0 .. r(P+L-1)` for its locals (the parameters arrive in `r0..`, which is the efevm calling
//! convention), one scratch register, and then one register per operand stack slot: the slot at
//! height `h` is `sbase + h`. Wasm validation fixes the height at every point, so the mapping
//! needs no allocator, and the arguments of a call are the top slots, which are contiguous: a
//! call needs no copying (`call f, base` with `base` = the first argument's slot, results land
//! in the same registers). `local.get` pushes a *symbolic* entry (`Ent::Local`) instead of
//! copying; it is written to its slot only when it has to be (before the local is overwritten,
//! before a call or branch that reads slots, at control-flow joins).

use crate::error::{Loc, TranslateError, unsupported};
use efevm::{FuncType, FunctionBuilder, Inst, Label, Op};
use std::collections::{HashMap, HashSet};
use wasmparser::{BlockType, BrTable, FunctionBody, MemArg, Operator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvKind {
    Send,
    Count,
    Read,
}

/// What a wasm function import turns into.
#[derive(Clone, Debug)]
pub(crate) enum Host {
    /// An efevm import. `wrap[k]` is set when result `k` is a wasm `i32` that the host returns
    /// as an `i64` (a `ptr`), so it is wrapped back.
    Fn { idx: u32, np: u32, nr: u32, wrap: Vec<bool> },
    /// `<event>.send|count|read`, lowered to `engine.events.*` (design doc, section 3.4).
    Event { kind: EvKind, id: u32, payload: bool, idx: u32 },
}

/// The lazy initialisation (`initialized` flag) of modules with a start function or ctors.
pub(crate) struct Init {
    pub global: u32,
    /// Wasm indices of the functions in `targets`.
    pub target_funcs: Vec<u32>,
    /// efevm functions (`() -> ()`) to call the first time, in order.
    pub targets: Vec<u32>,
}

/// Read-only translation context.
pub(crate) struct Ctx {
    pub types: Vec<FuncType>,
    /// efevm type index of every wasm type.
    pub etypes: Vec<u32>,
    pub func_types: Vec<u32>,
    pub nimports: u32,
    pub hosts: Vec<Host>,
    /// efevm index of the first wasm-defined function.
    pub def_base: u32,
    pub init: Option<Init>,
    /// Wasm functions that get the initialisation prologue.
    pub prologue: HashSet<u32>,
    pub names: HashMap<u32, String>,
}

/// Wraps a `FunctionBuilder` with `u32` register numbers and an overflow flag.
pub(crate) struct Emit {
    pub fb: FunctionBuilder,
    overflow: bool,
}

impl Emit {
    pub fn new() -> Emit {
        Emit { fb: FunctionBuilder::new(0), overflow: false }
    }
    fn r(&mut self, n: u32) -> u16 {
        if n > 65534 {
            self.overflow = true;
            return 0;
        }
        self.fb.ensure_reg(n as u16);
        n as u16
    }
    pub fn mov(&mut self, d: u32, a: u32) {
        let (d, a) = (self.r(d), self.r(a));
        self.fb.mov(d, a);
    }
    fn op2(&mut self, op: Op, d: u32, a: u32) {
        let (d, a) = (self.r(d), self.r(a));
        self.fb.op2(op, d, a);
    }
    fn op3(&mut self, op: Op, d: u32, a: u32, b: u32) {
        let (d, a, b) = (self.r(d), self.r(a), self.r(b));
        self.fb.op3(op, d, a, b);
    }
    pub fn const_i32(&mut self, d: u32, v: i32) {
        let d = self.r(d);
        self.fb.const_i32(d, v);
    }
    pub fn const_i64(&mut self, d: u32, v: i64) {
        let d = self.r(d);
        self.fb.const_i64(d, v);
    }
    pub fn wrap(&mut self, d: u32, a: u32) {
        self.op2(Op::I32WrapI64, d, a);
    }
    pub fn call(&mut self, f: u32, base: u32, window: u32) {
        if window > 0 {
            self.r(base + window - 1);
        }
        let b = self.r(base);
        self.fb.call(f, b);
    }
    /// `ret base` of `n` results: the whole window must exist (the verifier checks it).
    pub fn ret(&mut self, base: u32, n: u32) {
        if n > 0 {
            self.r(base + n - 1);
        }
        let b = self.r(base);
        self.fb.ret(b);
    }
    pub fn global_get(&mut self, d: u32, g: u32) {
        let d = self.r(d);
        self.fb.global_get(d, g);
    }
    pub fn global_set(&mut self, g: u32, a: u32) {
        let a = self.r(a);
        self.fb.global_set(g, a);
    }
    pub fn br_if(&mut self, a: u32, l: Label) {
        let a = self.r(a);
        self.fb.br_if(a, l);
    }
    pub fn br_ifz(&mut self, a: u32, l: Label) {
        let a = self.r(a);
        self.fb.br_ifz(a, l);
    }
    pub fn finish_check(&self) -> bool {
        !self.overflow
    }
}

/// Emits the call of a host import (or an event helper) with the arguments in `base..`.
/// Results are left at `base..`, with `ptr` results wrapped to `i32`.
pub(crate) fn lower_host(e: &mut Emit, h: &Host, base: u32) {
    match h {
        Host::Fn { idx, np, nr, wrap } => {
            e.call(*idx, base, (*np).max(*nr));
            for (k, w) in wrap.iter().enumerate() {
                if *w {
                    e.op2(Op::I32WrapI64, base + k as u32, base + k as u32);
                }
            }
        }
        Host::Event { kind, id, payload, idx } => match kind {
            EvKind::Send => {
                if *payload {
                    e.mov(base + 1, base);
                } else {
                    e.const_i64(base + 1, 0);
                }
                e.const_i32(base, *id as i32);
                e.call(*idx, base, 2);
            }
            EvKind::Count => {
                e.const_i32(base, *id as i32);
                e.call(*idx, base, 1);
            }
            EvKind::Read => {
                e.mov(base + 2, base + 1);
                e.mov(base + 1, base);
                e.const_i32(base, *id as i32);
                e.call(*idx, base, 3);
            }
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ent {
    Slot,
    Local(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fn,
    Block,
    Loop,
    If,
}

struct Frame {
    kind: Kind,
    /// Stack height at entry, parameters excluded.
    h0: u32,
    nparams: u32,
    nresults: u32,
    /// The end label (block, if) or the start label (loop).
    label: Label,
    else_label: Option<Label>,
}

impl Frame {
    /// Values a branch to this frame carries.
    fn arity(&self) -> u32 {
        if self.kind == Kind::Loop { self.nparams } else { self.nresults }
    }
}

enum K {
    Un(Op),
    Bin(Op),
    Load(Op, MemArg),
    Store(Op, MemArg),
}

macro_rules! simple_ops {
    ($op:expr; bin [$($b:ident),* $(,)?] un [$($u:ident),* $(,)?] load [$($l:ident),* $(,)?] store [$($s:ident),* $(,)?]) => {
        match $op {
            $(Operator::$b => Some(K::Bin(Op::$b)),)*
            $(Operator::$u => Some(K::Un(Op::$u)),)*
            $(Operator::$l { memarg } => Some(K::Load(Op::$l, *memarg)),)*
            $(Operator::$s { memarg } => Some(K::Store(Op::$s, *memarg)),)*
            _ => None,
        }
    };
}

fn simple(op: &Operator<'_>) -> Option<K> {
    simple_ops! { op;
        bin [
            I32Add, I32Sub, I32Mul, I32DivS, I32DivU, I32RemS, I32RemU, I32And, I32Or, I32Xor, I32Shl, I32ShrS, I32ShrU, I32Rotl, I32Rotr,
            I64Add, I64Sub, I64Mul, I64DivS, I64DivU, I64RemS, I64RemU, I64And, I64Or, I64Xor, I64Shl, I64ShrS, I64ShrU, I64Rotl, I64Rotr,
            I32Eq, I32Ne, I32LtS, I32LtU, I32GtS, I32GtU, I32LeS, I32LeU, I32GeS, I32GeU,
            I64Eq, I64Ne, I64LtS, I64LtU, I64GtS, I64GtU, I64LeS, I64LeU, I64GeS, I64GeU,
            F32Add, F32Sub, F32Mul, F32Div, F32Min, F32Max, F32Copysign,
            F64Add, F64Sub, F64Mul, F64Div, F64Min, F64Max, F64Copysign,
            F32Eq, F32Ne, F32Lt, F32Gt, F32Le, F32Ge,
            F64Eq, F64Ne, F64Lt, F64Gt, F64Le, F64Ge,
        ]
        un [
            I32Clz, I32Ctz, I32Popcnt, I32Eqz, I64Clz, I64Ctz, I64Popcnt, I64Eqz,
            F32Abs, F32Neg, F32Sqrt, F32Ceil, F32Floor, F32Trunc, F32Nearest,
            F64Abs, F64Neg, F64Sqrt, F64Ceil, F64Floor, F64Trunc, F64Nearest,
            I32WrapI64, I64ExtendI32S, I64ExtendI32U,
            I32Extend8S, I32Extend16S, I64Extend8S, I64Extend16S, I64Extend32S,
            I32TruncF32S, I32TruncF32U, I32TruncF64S, I32TruncF64U, I64TruncF32S, I64TruncF32U, I64TruncF64S, I64TruncF64U,
            I32TruncSatF32S, I32TruncSatF32U, I32TruncSatF64S, I32TruncSatF64U,
            I64TruncSatF32S, I64TruncSatF32U, I64TruncSatF64S, I64TruncSatF64U,
            F32ConvertI32S, F32ConvertI32U, F32ConvertI64S, F32ConvertI64U,
            F64ConvertI32S, F64ConvertI32U, F64ConvertI64S, F64ConvertI64U,
            F32DemoteF64, F64PromoteF32,
            I32ReinterpretF32, I64ReinterpretF64, F32ReinterpretI32, F64ReinterpretI64,
        ]
        load [
            I32Load, I64Load, F32Load, F64Load,
            I32Load8S, I32Load8U, I32Load16S, I32Load16U,
            I64Load8S, I64Load8U, I64Load16S, I64Load16U, I64Load32S, I64Load32U,
        ]
        store [
            I32Store, I64Store, F32Store, F64Store, I32Store8, I32Store16, I64Store8, I64Store16, I64Store32,
        ]
    }
}

/// Names the construct of an operator outside the subset.
fn describe_unsupported(op: &Operator<'_>) -> String {
    let dbg = format!("{op:?}");
    let name = dbg.split([' ', '{', '(']).next().unwrap_or("").to_string();
    let has = |s: &str| name.contains(s);
    let kind = if name.starts_with("V128") || has("x16") || has("x8") || has("x4") || has("x2") || has("Simd") {
        "SIMD".to_string()
    } else if has("Atomic") {
        "atomics (threads)".to_string()
    } else if name.starts_with("ReturnCall") {
        "tail calls".to_string()
    } else if matches!(name.as_str(), "Try" | "Catch" | "CatchAll" | "Throw" | "Rethrow" | "Delegate" | "TryTable" | "ThrowRef") {
        "exceptions".to_string()
    } else if matches!(name.as_str(), "MemoryInit" | "DataDrop") {
        "`memory.init`/`data.drop` (passive data segments)".to_string()
    } else if matches!(name.as_str(), "CallRef" | "RefAsNonNull" | "BrOnNull" | "BrOnNonNull") {
        "function references".to_string()
    } else if name.starts_with("Table") || matches!(name.as_str(), "RefNull" | "RefFunc" | "RefIsNull" | "ElemDrop") {
        "reference types beyond one funcref table 0".to_string()
    } else if name.starts_with("Struct") || name.starts_with("Array") || name.starts_with("Ref") || name.starts_with("I31") || name.contains("Convert") && name.contains("Extern") || name.starts_with("Any") || name.starts_with("Extern") || name.starts_with("BrOnCast") {
        "GC".to_string()
    } else {
        "operator".to_string()
    };
    let shown = to_snake(&name);
    format!("{kind} (`{shown}`)")
}

fn to_snake(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            s.push('_');
        }
        s.push(c.to_ascii_lowercase());
    }
    s
}

fn is_num(t: wasmparser::ValType) -> bool {
    matches!(t, wasmparser::ValType::I32 | wasmparser::ValType::I64 | wasmparser::ValType::F32 | wasmparser::ValType::F64)
}

/// `Ok` if the operator is in the supported subset (and memory/table 0 is meant).
fn check_supported(op: &Operator<'_>) -> Result<(), String> {
    use Operator::*;
    let multi = |m: u32, what: &str| if m != 0 { Err(format!("multi-memory ({what} on memory {m})")) } else { Ok(()) };
    match op {
        Block { blockty } | Loop { blockty } | If { blockty } => match blockty {
            BlockType::Type(t) if !is_num(*t) => Err(format!("a block of type `{t}` (SIMD or reference types)")),
            _ => Ok(()),
        },
        TypedSelect { ty } if !is_num(*ty) => Err(format!("`select` of type `{ty}` (SIMD or reference types)")),
        Unreachable | Nop | Else | End | Br { .. } | BrIf { .. } | BrTable { .. }
        | Return | Call { .. } | Drop | Select | TypedSelect { .. } | LocalGet { .. } | LocalSet { .. } | LocalTee { .. }
        | GlobalGet { .. } | GlobalSet { .. } | I32Const { .. } | I64Const { .. } | F32Const { .. } | F64Const { .. } => Ok(()),
        CallIndirect { table_index, .. } => {
            if *table_index != 0 {
                Err("reference types (`call_indirect` through a table other than table 0)".into())
            } else {
                Ok(())
            }
        }
        MemorySize { mem } => multi(*mem, "memory.size"),
        MemoryGrow { mem } => multi(*mem, "memory.grow"),
        MemoryFill { mem } => multi(*mem, "memory.fill"),
        MemoryCopy { dst_mem, src_mem } => multi(*dst_mem, "memory.copy").and(multi(*src_mem, "memory.copy")),
        other => match simple(other) {
            Some(K::Load(_, m) | K::Store(_, m)) => multi(m.memory, "a load or store"),
            Some(_) => Ok(()),
            None => Err(describe_unsupported(other)),
        },
    }
}

pub(crate) struct FnCx<'a> {
    cx: &'a Ctx,
    fidx: u32,
    e: Emit,
    stack: Vec<Ent>,
    frames: Vec<Frame>,
    sbase: u32,
    scratch: u32,
    nresults: u32,
    /// `Some(depth)` while skipping unreachable code.
    dead: Option<u32>,
    off: usize,
    done: bool,
}

impl<'a> FnCx<'a> {
    fn loc(&self) -> Loc {
        Loc { func: Some(self.fidx), name: self.cx.names.get(&self.fidx).cloned(), offset: Some(self.off) }
    }

    fn block_sig(&self, bt: BlockType) -> (u32, u32) {
        match bt {
            BlockType::Empty => (0, 0),
            BlockType::Type(_) => (0, 1),
            BlockType::FuncType(t) => {
                let ty = &self.cx.types[t as usize];
                (ty.params.len() as u32, ty.results.len() as u32)
            }
        }
    }

    fn slot(&self, i: usize) -> u32 {
        self.sbase + i as u32
    }

    fn opreg(&self, i: usize) -> u32 {
        match self.stack[i] {
            Ent::Slot => self.slot(i),
            Ent::Local(x) => x,
        }
    }

    /// Writes a symbolic entry into its slot.
    fn materialize(&mut self, i: usize) {
        if let Ent::Local(x) = self.stack[i] {
            let s = self.slot(i);
            self.e.mov(s, x);
            self.stack[i] = Ent::Slot;
        }
    }

    fn materialize_from(&mut self, from: usize) {
        for i in from..self.stack.len() {
            self.materialize(i);
        }
    }

    /// Writes every entry that reads local `x` into its slot (before `x` changes).
    fn invalidate(&mut self, x: u32, upto: usize) {
        for i in 0..upto {
            if self.stack[i] == Ent::Local(x) {
                self.materialize(i);
            }
        }
    }

    fn pop_n<const N: usize>(&mut self) -> [u32; N] {
        let len = self.stack.len();
        let mut regs = [0u32; N];
        for (k, r) in regs.iter_mut().enumerate() {
            *r = self.opreg(len - N + k);
        }
        self.stack.truncate(len - N);
        regs
    }

    fn push_slot(&mut self) -> u32 {
        self.stack.push(Ent::Slot);
        self.slot(self.stack.len() - 1)
    }

    fn reset_stack(&mut self, h: u32, extra: u32) {
        self.stack.truncate(h as usize);
        while self.stack.len() < (h + extra) as usize {
            self.stack.push(Ent::Slot);
        }
    }

    fn new_frame(&mut self, kind: Kind, nparams: u32, nresults: u32, else_label: Option<Label>) -> Label {
        let label = self.e.fb.new_label();
        let h0 = (self.stack.len() as u32).saturating_sub(nparams);
        self.frames.push(Frame { kind, h0, nparams, nresults, label, else_label });
        label
    }

    /// Moves the `n` values at `src..` to `dst..` (ascending: `dst <= src`, so no overlap hazard).
    fn moves(&mut self, src: u32, dst: u32, n: u32) {
        if src == dst {
            return;
        }
        for k in 0..n {
            self.e.mov(dst + k, src + k);
        }
    }

    fn ret_top(&mut self, n: u32) {
        let len = self.stack.len();
        self.materialize_from(len - n as usize);
        let base = if n == 0 { 0 } else { self.slot(len - n as usize) };
        self.e.ret(base, n);
    }

    /// `br` (`cond` = `None`) or `br_if`.
    fn branch(&mut self, depth: u32, cond: Option<u32>) {
        let fi = self.frames.len() - 1 - depth as usize;
        let (kind, label, h0, arity) = {
            let f = &self.frames[fi];
            (f.kind, f.label, f.h0, f.arity())
        };
        let len = self.stack.len();
        self.materialize_from(len - arity as usize);
        let src = self.slot(len - arity as usize);
        if kind == Kind::Fn {
            let base = if arity == 0 { 0 } else { src };
            match cond {
                None => self.e.ret(base, arity),
                Some(c) => {
                    let skip = self.e.fb.new_label();
                    self.e.br_ifz(c, skip);
                    self.e.ret(base, arity);
                    self.e.fb.bind(skip);
                }
            }
            return;
        }
        let dst = self.sbase + h0;
        let need_moves = arity > 0 && src != dst;
        match cond {
            None => {
                self.moves(src, dst, arity);
                self.e.fb.jmp(label);
            }
            Some(c) if !need_moves => self.e.br_if(c, label),
            Some(c) => {
                let skip = self.e.fb.new_label();
                self.e.br_ifz(c, skip);
                self.moves(src, dst, arity);
                self.e.fb.jmp(label);
                self.e.fb.bind(skip);
            }
        }
    }

    fn br_table(&mut self, t: &BrTable<'_>) -> Result<(), TranslateError> {
        let idx = self.pop_n::<1>()[0];
        let mut depths = Vec::new();
        for d in t.targets() {
            depths.push(d.map_err(|x| crate::parse::invalid(x.offset(), x.message()))?);
        }
        let default = t.default();
        let arity = self.frames[self.frames.len() - 1 - default as usize].arity();
        let len = self.stack.len();
        self.materialize_from(len - arity as usize);
        let src = self.slot(len - arity as usize);
        // Direct when no target needs a move or a `ret`.
        let direct = depths.iter().chain(std::iter::once(&default)).all(|d| {
            let f = &self.frames[self.frames.len() - 1 - *d as usize];
            f.kind != Kind::Fn && (arity == 0 || src == self.sbase + f.h0)
        });
        if direct {
            let labels: Vec<Label> = depths.iter().map(|d| self.frames[self.frames.len() - 1 - *d as usize].label).collect();
            let dl = self.frames[self.frames.len() - 1 - default as usize].label;
            let r = self.e.r(idx);
            self.e.fb.br_table(r, &labels, dl);
            return Ok(());
        }
        // One trampoline per distinct depth.
        let mut uniq: Vec<u32> = Vec::new();
        for d in depths.iter().chain(std::iter::once(&default)) {
            if !uniq.contains(d) {
                uniq.push(*d);
            }
        }
        let tramps: Vec<Label> = uniq.iter().map(|_| self.e.fb.new_label()).collect();
        let tl = |d: u32| tramps[uniq.iter().position(|x| *x == d).unwrap_or(0)];
        let labels: Vec<Label> = depths.iter().map(|d| tl(*d)).collect();
        let r = self.e.r(idx);
        self.e.fb.br_table(r, &labels, tl(default));
        for (d, l) in uniq.iter().zip(&tramps) {
            self.e.fb.bind(*l);
            let (kind, label, h0) = {
                let f = &self.frames[self.frames.len() - 1 - *d as usize];
                (f.kind, f.label, f.h0)
            };
            if kind == Kind::Fn {
                self.e.ret(if arity == 0 { 0 } else { src }, arity);
            } else {
                self.moves(src, self.sbase + h0, arity);
                self.e.fb.jmp(label);
            }
        }
        Ok(())
    }

    fn do_else(&mut self) {
        let (h0, np, end, el) = {
            let f = self.frames.last().expect("frame");
            (f.h0, f.nparams, f.label, f.else_label)
        };
        if self.dead.is_none() {
            self.materialize_from(0);
            self.e.fb.jmp(end);
        }
        if let Some(l) = el {
            self.e.fb.bind(l);
        }
        if let Some(f) = self.frames.last_mut() {
            f.else_label = None;
        }
        self.reset_stack(h0, np);
        self.dead = None;
    }

    fn do_end(&mut self) {
        let f = self.frames.pop().expect("frame");
        let live = self.dead.is_none();
        match f.kind {
            Kind::Fn => {
                if live {
                    self.ret_top(f.nresults);
                } else {
                    self.e.fb.unreachable();
                }
                self.done = true;
            }
            kind => {
                if live {
                    self.materialize_from(0);
                }
                if let Some(l) = f.else_label {
                    self.e.fb.bind(l);
                }
                if kind != Kind::Loop {
                    self.e.fb.bind(f.label);
                }
                self.reset_stack(f.h0, f.nresults);
                self.dead = None;
            }
        }
    }

    fn call_func(&mut self, f: u32) {
        let ty = &self.cx.types[self.cx.func_types[f as usize] as usize];
        let (p, r) = (ty.params.len(), ty.results.len());
        let len = self.stack.len();
        self.materialize_from(len - p);
        let base = self.slot(len - p);
        if f < self.cx.nimports {
            let h = self.cx.hosts[f as usize].clone();
            lower_host(&mut self.e, &h, base);
        } else {
            self.e.call(self.cx.def_base + f - self.cx.nimports, base, p.max(r) as u32);
        }
        self.stack.truncate(len - p);
        for _ in 0..r {
            self.stack.push(Ent::Slot);
        }
    }

    fn call_indirect(&mut self, type_index: u32) {
        let ty = &self.cx.types[type_index as usize];
        let (p, r) = (ty.params.len(), ty.results.len());
        let a = self.pop_n::<1>()[0];
        let len = self.stack.len();
        self.materialize_from(len - p);
        let base = self.slot(len - p);
        let window = p.max(r) as u32;
        if window > 0 {
            self.e.r(base + window - 1);
        }
        let (a16, b16) = (self.e.r(a), self.e.r(base));
        self.e.fb.call_indirect(self.cx.etypes[type_index as usize], a16, b16);
        self.stack.truncate(len - p);
        for _ in 0..r {
            self.stack.push(Ent::Slot);
        }
    }

    fn live_op(&mut self, op: Operator<'_>) -> Result<(), TranslateError> {
        use Operator::*;
        match op {
            Unreachable => {
                self.e.fb.unreachable();
                self.dead = Some(0);
            }
            Nop => {}
            Block { blockty } => {
                let (p, r) = self.block_sig(blockty);
                self.materialize_from(0);
                self.new_frame(Kind::Block, p, r, None);
            }
            Loop { blockty } => {
                let (p, r) = self.block_sig(blockty);
                self.materialize_from(0);
                let l = self.new_frame(Kind::Loop, p, r, None);
                self.e.fb.bind(l);
            }
            If { blockty } => {
                let (p, r) = self.block_sig(blockty);
                let c = self.pop_n::<1>()[0];
                self.materialize_from(0);
                let el = self.e.fb.new_label();
                self.new_frame(Kind::If, p, r, Some(el));
                self.e.br_ifz(c, el);
            }
            Else => self.do_else(),
            End => self.do_end(),
            Br { relative_depth } => {
                self.branch(relative_depth, None);
                self.dead = Some(0);
            }
            BrIf { relative_depth } => {
                let c = self.pop_n::<1>()[0];
                self.branch(relative_depth, Some(c));
            }
            BrTable { targets } => {
                self.br_table(&targets)?;
                self.dead = Some(0);
            }
            Return => {
                let n = self.nresults;
                self.ret_top(n);
                self.dead = Some(0);
            }
            Call { function_index } => self.call_func(function_index),
            CallIndirect { type_index, .. } => self.call_indirect(type_index),
            Drop => {
                self.stack.pop();
            }
            Select | TypedSelect { .. } => {
                let [a, b, c] = self.pop_n::<3>();
                let d = self.push_slot();
                let (d, c, a, b) = (self.e.r(d), self.e.r(c), self.e.r(a), self.e.r(b));
                self.e.fb.select(d, c, a, b);
            }
            LocalGet { local_index } => self.stack.push(Ent::Local(local_index)),
            LocalSet { local_index } => {
                let v = self.pop_n::<1>()[0];
                let n = self.stack.len();
                self.invalidate(local_index, n);
                if v != local_index {
                    self.e.mov(local_index, v);
                }
            }
            LocalTee { local_index } => {
                let n = self.stack.len();
                let v = self.opreg(n - 1);
                self.invalidate(local_index, n - 1);
                if v != local_index {
                    self.e.mov(local_index, v);
                }
                self.stack[n - 1] = Ent::Local(local_index);
            }
            GlobalGet { global_index } => {
                let d = self.push_slot();
                self.e.global_get(d, global_index);
            }
            GlobalSet { global_index } => {
                let v = self.pop_n::<1>()[0];
                self.e.global_set(global_index, v);
            }
            I32Const { value } => {
                let d = self.push_slot();
                self.e.const_i32(d, value);
            }
            I64Const { value } => {
                let d = self.push_slot();
                self.e.const_i64(d, value);
            }
            F32Const { value } => {
                let d = self.push_slot();
                let d = self.e.r(d);
                self.e.fb.emit(Inst::new(Op::ConstF32, u32::from(d), value.bits(), 0));
            }
            F64Const { value } => {
                let d = self.push_slot();
                let d = self.e.r(d);
                let b = value.bits();
                self.e.fb.emit(Inst::new(Op::ConstF64, u32::from(d), b as u32, (b >> 32) as u32));
            }
            MemorySize { .. } => {
                let d = self.push_slot();
                let d = self.e.r(d);
                self.e.fb.memory_size(d);
            }
            MemoryGrow { .. } => {
                let [a] = self.pop_n::<1>();
                let d = self.push_slot();
                let (d16, a16) = (self.e.r(d), self.e.r(a));
                self.e.fb.memory_grow(d16, a16);
                // efevm returns the old size or -1 as an i64; wasm wants an i32.
                self.e.op2(Op::I32WrapI64, d, d);
            }
            MemoryCopy { .. } => {
                let [a, b, c] = self.pop_n::<3>();
                let (a, b, c) = (self.e.r(a), self.e.r(b), self.e.r(c));
                self.e.fb.memory_copy(a, b, c);
            }
            MemoryFill { .. } => {
                let [a, b, c] = self.pop_n::<3>();
                let (a, b, c) = (self.e.r(a), self.e.r(b), self.e.r(c));
                self.e.fb.memory_fill(a, b, c);
            }
            other => match simple(&other) {
                Some(K::Un(op)) => {
                    let [a] = self.pop_n::<1>();
                    let d = self.push_slot();
                    self.e.op2(op, d, a);
                }
                Some(K::Bin(op)) => {
                    let [a, b] = self.pop_n::<2>();
                    let d = self.push_slot();
                    self.e.op3(op, d, a, b);
                }
                Some(K::Load(op, m)) => {
                    let [a] = self.pop_n::<1>();
                    let d = self.push_slot();
                    let (d, a) = (self.e.r(d), self.e.r(a));
                    self.e.fb.load(op, d, a, self.offset(&m)?);
                }
                Some(K::Store(op, m)) => {
                    let [a, v] = self.pop_n::<2>();
                    let (a, v) = (self.e.r(a), self.e.r(v));
                    self.e.fb.store(op, v, a, self.offset(&m)?);
                }
                None => return Err(unsupported(describe_unsupported(&other), self.loc())),
            },
        }
        Ok(())
    }

    fn offset(&self, m: &MemArg) -> Result<u32, TranslateError> {
        u32::try_from(m.offset).map_err(|_| unsupported("a memory offset above 32 bits (memory64)", self.loc()))
    }
}

/// Rejects unsupported operators of a body (before validation, so the error names the construct).
pub(crate) fn scan_body(fidx: u32, name: Option<&String>, body: &FunctionBody<'_>) -> Result<(), TranslateError> {
    let loc = |off: usize| Loc { func: Some(fidx), name: name.cloned(), offset: Some(off) };
    let werr = |x: wasmparser::BinaryReaderError| crate::parse::invalid(x.offset(), x.message());
    let ops = body.get_operators_reader().map_err(werr)?;
    for item in ops.into_iter_with_offsets() {
        let (op, off) = item.map_err(werr)?;
        if let Err(what) = check_supported(&op) {
            return Err(unsupported(what, loc(off)));
        }
    }
    for l in body.get_locals_reader().map_err(werr)? {
        let (_, t) = l.map_err(werr)?;
        match t {
            wasmparser::ValType::V128 => return Err(unsupported("SIMD (a `v128` local)", loc(body.range().start))),
            wasmparser::ValType::Ref(_) => return Err(unsupported("reference types (a reference-typed local)", loc(body.range().start))),
            _ => {}
        }
    }
    Ok(())
}

/// The lazy initialisation prologue of an exported function (`s1`, `s2`: two free registers).
/// `target`: this function is the start function or `__wasm_call_ctors` itself. The flag is
/// 0 (not initialised), 1 (running the initialisers) or 2 (done). A target called while the
/// flag is 2 returns at once, and one that triggers the initialisation returns after it (the
/// initialisers include itself), so each runs exactly once.
pub(crate) fn emit_prologue(e: &mut Emit, init: &Init, s1: u32, s2: u32, target: bool) {
    let (inited, body) = (e.fb.new_label(), e.fb.new_label());
    e.global_get(s1, init.global);
    e.br_if(s1, inited);
    e.const_i32(s2, 1);
    e.global_set(init.global, s2);
    for t in &init.targets {
        e.call(*t, 0, 0);
    }
    e.const_i32(s2, 2);
    e.global_set(init.global, s2);
    if target {
        e.ret(0, 0);
    } else {
        e.fb.jmp(body);
    }
    e.fb.bind(inited);
    if target {
        // flag 1: we are being run by the initialisation; flag 2: nothing left to do
        e.const_i32(s2, 2);
        let (a, b) = (e.r(s1), e.r(s2));
        e.fb.op3(Op::I32Eq, a, a, b);
        e.br_ifz(s1, body);
        e.ret(0, 0);
    }
    e.fb.bind(body);
}

/// Translates one wasm function body.
pub(crate) fn translate_body(cx: &Ctx, fidx: u32, body: &FunctionBody<'_>) -> Result<FunctionBuilder, TranslateError> {
    let ty = &cx.types[cx.func_types[fidx as usize] as usize];
    let loc = |off: usize| Loc { func: Some(fidx), name: cx.names.get(&fidx).cloned(), offset: Some(off) };
    let werr = |x: wasmparser::BinaryReaderError| crate::parse::invalid(x.offset(), x.message());

    let mut nlocals = ty.params.len() as u64;
    for l in body.get_locals_reader().map_err(werr)? {
        let (n, _) = l.map_err(werr)?;
        nlocals += u64::from(n);
    }
    if nlocals + 3 > 65535 {
        return Err(TranslateError::TooLarge { what: format!("{nlocals} locals exceed efevm's 65535 registers"), at: loc(body.range().start) });
    }
    let nlocals = nlocals as u32;
    let mut f = FnCx {
        cx,
        fidx,
        e: Emit::new(),
        stack: Vec::new(),
        frames: Vec::new(),
        sbase: nlocals + 2,
        scratch: nlocals,
        nresults: ty.results.len() as u32,
        dead: None,
        off: body.range().start,
        done: false,
    };
    f.e.fb.set_nregs((nlocals + 2) as u16);
    f.frames.push(Frame { kind: Kind::Fn, h0: 0, nparams: 0, nresults: ty.results.len() as u32, label: f.e.fb.new_label(), else_label: None });

    if cx.prologue.contains(&fidx) {
        if let Some(init) = &cx.init {
            let target = init.target_funcs.contains(&fidx);
            emit_prologue(&mut f.e, init, f.scratch, f.scratch + 1, target);
        }
    }

    // Translate.
    let ops = body.get_operators_reader().map_err(werr)?;
    for item in ops.into_iter_with_offsets() {
        let (op, off) = item.map_err(werr)?;
        f.off = off;
        if f.done {
            break;
        }
        if let Some(d) = f.dead {
            use Operator::*;
            match op {
                Block { .. } | Loop { .. } | If { .. } => f.dead = Some(d + 1),
                Else if d == 0 => f.do_else(),
                End if d == 0 => f.do_end(),
                End => f.dead = Some(d - 1),
                _ => {}
            }
        } else {
            f.live_op(op)?;
        }
        if !f.e.finish_check() {
            return Err(TranslateError::TooLarge {
                what: "the function needs more than 65535 registers (locals plus operand stack); efevm functions cannot be split".into(),
                at: loc(off),
            });
        }
    }
    Ok(f.e.fb)
}
