//! A seeded generator of random valid wasm modules within the translator's subset, as WAT text.
//!
//! Every module terminates: loops run a fixed number of times on a private counter and are
//! never branch targets, direct calls only go to higher function indices, and a fuel global
//! (global 0) bounds the number of function entries (indirect calls may recurse). NaN payloads
//! are canonicalised after every float operation that can produce a NaN, so results can be
//! compared bit for bit between engines. Calls cover: all numeric operators, conversions
//! (trapping and saturating), loads and stores of every width, memory.size/grow/copy/fill,
//! globals, locals, select, blocks with parameters and results, `br`/`br_if`/`br_table` with
//! values, multi-value calls, `call_indirect` (valid, null, out of range, wrong type), traps.
#![allow(dead_code)]

#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    pub fn range(&mut self, lo: u64, hi_incl: u64) -> u64 {
        lo + self.below(hi_incl - lo + 1)
    }
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    I32,
    I64,
    F32,
    F64,
}

impl Ty {
    pub const ALL: [Ty; 4] = [Ty::I32, Ty::I64, Ty::F32, Ty::F64];
    pub fn name(self) -> &'static str {
        match self {
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::F32 => "f32",
            Ty::F64 => "f64",
        }
    }
    fn is_float(self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    pub params: Vec<Ty>,
    pub results: Vec<Ty>,
}

impl Sig {
    fn wat(&self) -> String {
        let mut s = String::new();
        if !self.params.is_empty() {
            s.push_str("(param");
            for t in &self.params {
                s.push(' ');
                s.push_str(t.name());
            }
            s.push(')');
        }
        if !self.results.is_empty() {
            s.push_str(" (result");
            for t in &self.results {
                s.push(' ');
                s.push_str(t.name());
            }
            s.push(')');
        }
        s
    }
}

const I32S: &[i32] = &[0, 1, -1, 2, 3, 7, 8, 15, 16, 31, 32, 33, 63, 64, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0xffff, 0x1_0000, 0x7fff_ffff, i32::MIN, i32::MIN + 1, -2, -128, -32768, 0x1234_5678, 100, 255, 1000, 65535, 65536, 65532, 65533, 131071, 4];
const I64S: &[i64] = &[
    0, 1, -1, 2, 3, 7, 8, 31, 32, 33, 63, 64, 65, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0x1_0000_0000, i64::MAX, i64::MIN, i64::MIN + 1, -2, -128,
    0x0123_4567_89ab_cdef, 1000, 65536,
];
const F64S: &[f64] = &[
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 0.1, 1e10, -1e10, 1e300, 5e-324, f64::MIN_POSITIVE, f64::MAX, f64::MIN, f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
    2147483647.0, 2147483648.0, -2147483648.0, -2147483649.0, 4294967295.0, 4294967296.0, 9223372036854775807.0, 9223372036854775808.0, -9223372036854775808.0, 18446744073709551615.0,
    18446744073709551616.0, -0.9, 0.9, 100.0, 255.5,
];
const F32S: &[f32] = &[
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 0.1, 1e10, -1e10, 1e30, 1e-45, f32::MIN_POSITIVE, f32::MAX, f32::MIN, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2147483648.0,
    -2147483648.0, 4294967296.0, 9223372036854775808.0, -9223372036854775808.0, 18446744073709551616.0, -0.9, 0.9, 16777216.0, 100.0, 255.5,
];

pub fn rand_arg(r: &mut Rng, t: Ty) -> u64 {
    match t {
        Ty::I32 => {
            if r.chance(1, 8) { r.next() as u32 as u64 } else { r.pick(I32S) as u32 as u64 }
        }
        Ty::I64 => {
            if r.chance(1, 8) { r.next() } else { r.pick(I64S) as u64 }
        }
        Ty::F32 => {
            if r.chance(1, 10) { r.next() as u32 as u64 } else { r.pick(F32S).to_bits() as u64 }
        }
        Ty::F64 => {
            if r.chance(1, 10) { r.next() } else { r.pick(F64S).to_bits() }
        }
    }
}

fn f32_lit(v: f32) -> String {
    if v.is_nan() {
        "nan".into()
    } else if v.is_infinite() {
        if v > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        format!("{v:?}")
    }
}
fn f64_lit(v: f64) -> String {
    if v.is_nan() {
        "nan".into()
    } else if v.is_infinite() {
        if v > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        format!("{v:?}")
    }
}

pub struct Gen {
    pub wat: String,
    /// Exported functions: name and signature.
    pub funcs: Vec<(String, Sig)>,
    /// Exported mutable globals (global index, name, type), including the fuel global.
    pub globals: Vec<(u32, String, Ty)>,
}

struct Label {
    arity: Vec<Ty>,
    /// `br` to it is allowed (forward targets only: blocks, ifs, the function; never loops).
    targetable: bool,
}

struct Module<'a> {
    r: &'a mut Rng,
    sigs: Vec<Sig>,
    gtypes: Vec<Ty>, // global i+1
    table: Vec<u32>, // function index at table slot 1+k
    /// `(memory 1)` without a maximum: grows only by 0 or 1 page (the real limit is 4 GiB).
    unbounded: bool,
}

struct F<'m, 'a> {
    m: &'m mut Module<'a>,
    me: usize,
    locals: Vec<Ty>,
    out: Vec<String>,
    labels: Vec<Label>,
    budget: i32,
    /// Loop counters: never written by the random productions, so every loop terminates.
    reserved: Vec<usize>,
}

pub fn gen_module(seed: u64) -> Gen {
    let mut rng = Rng::new(seed);
    let r = &mut rng;
    let nf = r.range(2, 7) as usize;
    let mut sigs = Vec::new();
    for i in 0..nf {
        let np = if i == 0 { r.range(0, 3) } else { r.range(0, 4) } as usize;
        let params = (0..np).map(|_| r.pick(&Ty::ALL)).collect();
        let nr = match r.below(100) {
            0..=29 => 0,
            30..=79 => 1,
            80..=94 => 2,
            _ => 3,
        };
        let results = (0..nr).map(|_| r.pick(&Ty::ALL)).collect();
        sigs.push(Sig { params, results });
    }
    let nglob = r.range(2, 5) as usize;
    let gtypes: Vec<Ty> = (0..nglob).map(|_| r.pick(&Ty::ALL)).collect();
    let tn = r.range(2, 6) as usize;
    let table: Vec<u32> = (0..tn).map(|_| r.below(nf as u64) as u32).collect();

    let mut wat = String::from("(module\n");
    for (i, s) in sigs.iter().enumerate() {
        wat.push_str(&format!("  (type $t{i} (func {}))\n", s.wat()));
    }
    // Mostly 1..3 pages; sometimes no pages at all (and no data), sometimes no declared maximum.
    let mem_kind = r.below(10);
    wat.push_str(match mem_kind {
        0 => "  (memory (export \"mem\") 0 2)\n",
        1 => "  (memory (export \"mem\") 1)\n",
        _ => "  (memory (export \"mem\") 1 3)\n",
    });
    for _ in 0..if mem_kind == 0 { 0 } else { r.range(1, 3) } {
        let off = r.below(65536 - 64) as u32;
        let len = r.range(1, 48) as usize;
        let bytes: String = (0..len).map(|_| format!("\\{:02x}", r.next() as u8)).collect();
        wat.push_str(&format!("  (data (i32.const {off}) \"{bytes}\")\n"));
    }
    wat.push_str("  (table 8 funcref)\n");
    wat.push_str(&format!("  (elem (i32.const 1) {})\n", table.iter().map(|f| format!("$f{f}")).collect::<Vec<_>>().join(" ")));
    wat.push_str("  (global $fuel (export \"g0\") (mut i32) (i32.const 600))\n");
    for (i, t) in gtypes.iter().enumerate() {
        let init = match t {
            Ty::I32 => format!("(i32.const {})", r.pick(I32S)),
            Ty::I64 => format!("(i64.const {})", r.pick(I64S)),
            Ty::F32 => format!("(f32.const {})", f32_lit(r.pick(F32S))),
            Ty::F64 => format!("(f64.const {})", f64_lit(r.pick(F64S))),
        };
        wat.push_str(&format!("  (global (export \"g{}\") (mut {}) {init})\n", i + 1, t.name()));
    }
    // NaN canonicalisers.
    wat.push_str("  (func $canon32 (param f32) (result f32) f32.const nan local.get 0 local.get 0 local.get 0 f32.ne select)\n");
    wat.push_str("  (func $canon64 (param f64) (result f64) f64.const nan local.get 0 local.get 0 local.get 0 f64.ne select)\n");

    let mut m = Module { r, sigs: sigs.clone(), gtypes: gtypes.clone(), table, unbounded: mem_kind == 1 };
    let mut funcs = Vec::new();
    for i in 0..nf {
        let body = gen_func(&mut m, i);
        wat.push_str(&body);
        funcs.push((format!("f{i}"), sigs[i].clone()));
    }
    // A start function (run at instantiation by wasmi, lazily by the translation): it only
    // sets globals and memory to constants, so it cannot trap and leaves the fuel alone.
    if m.r.chance(1, 3) {
        let mut body = String::from("  (func $start\n");
        for _ in 0..m.r.range(1, 3) {
            let g = m.r.below(gtypes.len() as u64) as usize;
            let c = match gtypes[g] {
                Ty::I32 => format!("i32.const {}", m.r.pick(I32S)),
                Ty::I64 => format!("i64.const {}", m.r.pick(I64S)),
                Ty::F32 => format!("f32.const {}", f32_lit(m.r.pick(F32S))),
                Ty::F64 => format!("f64.const {}", f64_lit(m.r.pick(F64S))),
            };
            body.push_str(&format!("    {c}\n    global.set {}\n", g + 1));
        }
        if mem_kind != 0 {
            let a = m.r.below(60000);
            let v = m.r.pick(I32S);
            body.push_str(&format!("    i32.const {a}\n    i32.const {v}\n    i32.store\n"));
        }
        body.push_str("  )\n  (start $start)\n");
        wat.push_str(&body);
    }
    wat.push_str(")\n");
    let mut globals = vec![(0, "g0".to_string(), Ty::I32)];
    for (i, t) in gtypes.iter().enumerate() {
        globals.push((i as u32 + 1, format!("g{}", i + 1), *t));
    }
    Gen { wat, funcs, globals }
}

fn gen_func(m: &mut Module<'_>, me: usize) -> String {
    let sig = m.sigs[me].clone();
    let mut locals = sig.params.clone();
    let nextra = m.r.range(0, 4);
    for _ in 0..nextra {
        let t = m.r.pick(&Ty::ALL);
        locals.push(t);
    }
    let budget = m.r.range(10, 70) as i32;
    let mut f = F { m, me, locals, out: Vec::new(), labels: vec![Label { arity: sig.results.clone(), targetable: true }], budget, reserved: Vec::new() };
    // Fuel: every function entry costs one; at zero the function traps.
    f.op("global.get $fuel");
    f.op("i32.eqz");
    f.op("if");
    f.op("unreachable");
    f.op("end");
    f.op("global.get $fuel");
    f.op("i32.const 1");
    f.op("i32.sub");
    f.op("global.set $fuel");
    let n = f.m.r.range(1, 6);
    for _ in 0..n {
        f.stmt(3);
    }
    if sig.results.len() >= 2 && f.m.r.chance(1, 4) {
        // results produced by dead code only (the result window must still exist)
        let rs: Vec<&str> = sig.results.iter().map(|t| t.name()).collect();
        f.op(format!("block (result {})", rs.join(" ")));
        f.op("unreachable");
        f.op("end");
    } else {
        f.values(&sig.results, 3);
    }
    let extra: Vec<Ty> = f.locals[sig.params.len()..].to_vec();
    let mut s = format!("  (func $f{me} (export \"f{me}\") (type $t{me})");
    if !extra.is_empty() {
        s.push_str(" (local");
        for t in &extra {
            s.push(' ');
            s.push_str(t.name());
        }
        s.push(')');
    }
    s.push('\n');
    for o in &f.out {
        s.push_str("    ");
        s.push_str(o);
        s.push('\n');
    }
    s.push_str("  )\n");
    s
}

impl<'m, 'a> F<'m, 'a> {
    fn op(&mut self, s: impl Into<String>) {
        self.out.push(s.into());
    }
    fn r(&mut self) -> &mut Rng {
        self.m.r
    }

    fn new_local(&mut self, t: Ty) -> usize {
        self.locals.push(t);
        self.locals.len() - 1
    }

    fn values(&mut self, tys: &[Ty], d: u32) {
        for t in tys {
            self.expr(*t, d);
        }
    }

    fn leaf(&mut self, t: Ty) {
        let k = self.r().below(100);
        // local.get
        if k < 35 {
            let cands: Vec<usize> = (0..self.locals.len()).filter(|i| self.locals[*i] == t).collect();
            if !cands.is_empty() {
                let i = self.r().pick(&cands);
                self.op(format!("local.get {i}"));
                return;
            }
        }
        if k < 50 {
            let cands: Vec<usize> = (0..self.m.gtypes.len()).filter(|i| self.m.gtypes[*i] == t).collect();
            if !cands.is_empty() {
                let i = self.r().pick(&cands);
                self.op(format!("global.get {}", i + 1));
                return;
            }
        }
        self.constant(t);
    }

    fn constant(&mut self, t: Ty) {
        let s = match t {
            Ty::I32 => format!("i32.const {}", self.r().pick(I32S)),
            Ty::I64 => format!("i64.const {}", self.r().pick(I64S)),
            Ty::F32 => {
                if self.r().chance(1, 12) {
                    let b = self.r().next() as u32 as i32;
                    self.op(format!("i32.const {b}"));
                    "f32.reinterpret_i32".to_string()
                } else {
                    format!("f32.const {}", f32_lit(self.r().pick(F32S)))
                }
            }
            Ty::F64 => {
                if self.r().chance(1, 12) {
                    let b = self.r().next() as i64;
                    self.op(format!("i64.const {b}"));
                    "f64.reinterpret_i64".to_string()
                } else {
                    format!("f64.const {}", f64_lit(self.r().pick(F64S)))
                }
            }
        };
        self.op(s);
    }

    /// One value of type `t`.
    fn expr(&mut self, t: Ty, d: u32) {
        self.budget -= 1;
        if self.budget <= 0 || d == 0 {
            return self.leaf(t);
        }
        let k = self.r().below(100);
        match k {
            0..=17 => self.leaf(t),
            18..=57 => self.typed(t, d),
            _ => self.generic(t, d),
        }
    }

    /// Productions available for every type.
    fn generic(&mut self, t: Ty, d: u32) {
        match self.r().below(26) {
            0 => {
                // local.tee
                let cands: Vec<usize> = (0..self.locals.len()).filter(|i| self.locals[*i] == t && !self.reserved.contains(i)).collect();
                if cands.is_empty() {
                    return self.leaf(t);
                }
                let i = self.r().pick(&cands);
                self.expr(t, d.saturating_sub(1));
                self.op(format!("local.tee {i}"));
            }
            1 | 2 => {
                self.expr(t, d.saturating_sub(1));
                self.expr(t, d.saturating_sub(1));
                self.expr(Ty::I32, d.saturating_sub(1));
                if self.r().chance(1, 2) {
                    self.op(format!("select (result {})", t.name()));
                } else {
                    self.op("select");
                }
            }
            3 | 4 => {
                // if-else expression; branches to the if see arity [t]
                self.expr(Ty::I32, d.saturating_sub(1));
                self.op(format!("if (result {})", t.name()));
                self.labels.push(Label { arity: vec![t], targetable: true });
                self.stmts(d.saturating_sub(1), 0, 2);
                self.expr(t, d.saturating_sub(1));
                self.op("else");
                self.stmts(d.saturating_sub(1), 0, 2);
                self.expr(t, d.saturating_sub(1));
                self.labels.pop();
                self.op("end");
            }
            5 | 6 => {
                // block expression with a possible early exit carrying a value
                self.op(format!("block (result {})", t.name()));
                self.labels.push(Label { arity: vec![t], targetable: true });
                self.stmts(d.saturating_sub(1), 0, 2);
                if self.r().chance(2, 3) {
                    self.expr(t, d.saturating_sub(1));
                    self.expr(Ty::I32, d.saturating_sub(1));
                    let n = self.labels.len() as u64;
                    let depth = self.r().below(n);
                    self.br_if_to_arity(&[t], depth as usize);
                    // br_if leaves the value when not taken: drop it and make a fresh one
                    self.op("drop");
                }
                self.expr(t, d.saturating_sub(1));
                self.labels.pop();
                self.op("end");
            }
            7 => {
                // br_if from an enclosing label whose arity is exactly [t]
                let cands: Vec<usize> = (0..self.labels.len()).filter(|i| self.labels[*i].targetable && self.labels[*i].arity == [t]).collect();
                if cands.is_empty() {
                    return self.leaf(t);
                }
                let i = self.r().pick(&cands);
                self.expr(t, d.saturating_sub(1));
                self.expr(Ty::I32, d.saturating_sub(1));
                self.op(format!("br_if {}", self.labels.len() - 1 - i));
            }
            8 | 9 => self.call(Some(t), d),
            10 => {
                // br_table with a value
                self.op(format!("block (result {})", t.name()));
                self.labels.push(Label { arity: vec![t], targetable: true });
                self.op(format!("block (result {})", t.name()));
                self.labels.push(Label { arity: vec![t], targetable: true });
                self.expr(t, d.saturating_sub(1));
                self.expr(Ty::I32, d.saturating_sub(1));
                let n = self.labels.len();
                let ok: Vec<usize> = (0..n).filter(|i| self.labels[*i].targetable && self.labels[*i].arity == [t]).collect();
                let cnt = self.r().range(1, 4);
                let targets: Vec<String> = (0..=cnt).map(|_| format!("{}", n - 1 - self.m.r.pick(&ok))).collect();
                self.op(format!("br_table {}", targets.join(" ")));
                self.labels.pop();
                self.op("end");
                self.expr_modify(t, d.saturating_sub(1));
                self.labels.pop();
                self.op("end");
            }
            11 => {
                if self.r().chance(1, 6) {
                    self.op("unreachable");
                } else {
                    self.leaf(t)
                }
            }
            12 => self.param_block(t, d),
            13 => {
                // result through a multi-value function when t is the first result
                self.call(Some(t), d)
            }
            18..=20 => self.multi_block(t, d),
            14..=17 => {
                // `local.get L ... local.tee L / local.set L ... binop`: the first operand must keep the old value
                let cands: Vec<usize> = (0..self.locals.len()).filter(|i| self.locals[*i] == t && !self.reserved.contains(i)).collect();
                if cands.is_empty() {
                    return self.leaf(t);
                }
                let i = self.r().pick(&cands);
                self.op(format!("local.get {i}"));
                self.expr(t, d.saturating_sub(1));
                if self.r().chance(1, 2) {
                    self.op(format!("local.tee {i}"));
                } else {
                    self.op(format!("local.set {i}"));
                    self.op(format!("local.get {i}"));
                }
                let op = self.binop(t);
                self.canon_op(t, &op);
            }
            _ => self.leaf(t),
        }
    }

    /// Blocks and ifs with two results `[t u]`: a value below the carried ones (so a branch has to
    /// move them), branches into the block, dead code before the end or `else`. Leaves `t`
    /// (`u` is parked in a scratch local).
    fn multi_block(&mut self, t: Ty, d: u32) {
        let u = self.r().pick(&Ty::ALL);
        let d1 = d.saturating_sub(1);
        let (tn, un) = (t.name(), u.name());
        let park = self.new_local(u);
        match self.r().below(4) {
            0 | 1 => {
                self.op(format!("block (result {tn} {un})"));
                self.labels.push(Label { arity: vec![t, u], targetable: true });
                self.stmts(d1, 0, 1);
                let junk = self.r().pick(&Ty::ALL);
                self.expr(junk, d1);
                self.values(&[t, u], d1);
                self.expr(Ty::I32, d1);
                let cands: Vec<usize> = (0..self.labels.len()).filter(|i| self.labels[*i].targetable && self.labels[*i].arity == [t, u]).collect();
                let i = self.r().pick(&cands);
                self.op(format!("br_if {}", self.labels.len() - 1 - i));
                // not taken: [junk t u] -> [t u]
                let (lt, lu) = (self.new_local(t), self.new_local(u));
                self.op(format!("local.set {lu}"));
                self.op(format!("local.set {lt}"));
                self.op("drop");
                if self.r().chance(1, 4) {
                    self.op("unreachable");
                }
                self.op(format!("local.get {lt}"));
                self.op(format!("local.get {lu}"));
                self.labels.pop();
                self.op("end");
            }
            2 => {
                self.expr(Ty::I32, d1);
                self.op(format!("if (result {tn} {un})"));
                self.labels.push(Label { arity: vec![t, u], targetable: true });
                if self.r().chance(1, 3) {
                    self.op("unreachable");
                } else {
                    self.values(&[t, u], d1);
                }
                self.op("else");
                self.stmts(d1, 0, 1);
                self.values(&[t, u], d1);
                self.labels.pop();
                self.op("end");
            }
            _ => {
                // params that pass through unchanged, with and without else
                self.values(&[t, u], d1);
                self.expr(Ty::I32, d1);
                self.op(format!("if (param {tn} {un}) (result {tn} {un})"));
                self.labels.push(Label { arity: vec![t, u], targetable: true });
                if self.r().chance(1, 2) {
                    self.op("else");
                    self.op("nop");
                }
                self.labels.pop();
                self.op("end");
            }
        }
        self.op(format!("local.set {park}"));
    }

    /// Something that consumes one `t` from the stack and leaves one `t` (or just keeps it).
    fn expr_modify(&mut self, t: Ty, d: u32) {
        if self.r().chance(1, 2) {
            self.expr(t, d);
            let op = self.binop(t);
            self.canon_op(t, &op);
        }
    }

    /// `br_if` to the label at `idx` carrying `arity` values that are on the stack; only emitted
    /// when the label can take them, else the values are dropped.
    fn br_if_to_arity(&mut self, arity: &[Ty], idx: usize) {
        let ok = self.labels[idx].targetable && self.labels[idx].arity == arity;
        if ok {
            self.op(format!("br_if {}", self.labels.len() - 1 - idx));
        } else {
            // cond consumed, nothing branches
            self.op("drop");
        }
    }

    fn binop(&mut self, t: Ty) -> String {
        let ops: &[&str] = match t {
            Ty::I32 => &["i32.add", "i32.sub", "i32.mul", "i32.and", "i32.or", "i32.xor", "i32.shl", "i32.shr_s", "i32.shr_u", "i32.rotl", "i32.rotr", "i32.div_s", "i32.div_u", "i32.rem_s", "i32.rem_u"],
            Ty::I64 => &["i64.add", "i64.sub", "i64.mul", "i64.and", "i64.or", "i64.xor", "i64.shl", "i64.shr_s", "i64.shr_u", "i64.rotl", "i64.rotr", "i64.div_s", "i64.div_u", "i64.rem_s", "i64.rem_u"],
            Ty::F32 => &["f32.add", "f32.sub", "f32.mul", "f32.div", "f32.min", "f32.max", "f32.copysign"],
            Ty::F64 => &["f64.add", "f64.sub", "f64.mul", "f64.div", "f64.min", "f64.max", "f64.copysign"],
        };
        self.r().pick(ops).to_string()
    }

    /// `[t t] block (param t t) (result t) binop end` and the `if` form.
    fn param_block(&mut self, t: Ty, d: u32) {
        self.expr(t, d.saturating_sub(1));
        self.expr(t, d.saturating_sub(1));
        let n = t.name();
        let k = self.r().below(6);
        if k == 5 {
            // `if (param t t) (result t t)` without else: the operands pass through when the
            // condition is false; then consume both
            self.expr(Ty::I32, d.saturating_sub(1));
            self.op(format!("if (param {n} {n}) (result {n} {n})"));
            self.labels.push(Label { arity: vec![t, t], targetable: true });
            self.op("nop");
            self.labels.pop();
            self.op("end");
            let op = self.binop(t);
            self.canon_op(t, &op);
        } else if k < 3 {
            self.op(format!("block (param {n} {n}) (result {n})"));
            self.labels.push(Label { arity: vec![t], targetable: true });
            let op = self.binop(t);
            self.canon_op(t, &op);
            self.labels.pop();
            self.op("end");
        } else {
            self.expr(Ty::I32, d.saturating_sub(1));
            self.op(format!("if (param {n} {n}) (result {n})"));
            self.labels.push(Label { arity: vec![t], targetable: true });
            let op = self.binop(t);
            self.canon_op(t, &op);
            self.op("else");
            let op = self.binop(t);
            self.canon_op(t, &op);
            self.labels.pop();
            self.op("end");
        }
    }

    /// Emits a float op and canonicalises a possible NaN result.
    fn canon_op(&mut self, t: Ty, op: &str) {
        self.op(op.to_string());
        let nan_prone = |o: &str| !(o.ends_with("copysign") || o.ends_with(".abs") || o.ends_with(".neg"));
        if t.is_float() && nan_prone(op) {
            self.op(if t == Ty::F32 { "call $canon32" } else { "call $canon64" });
        }
    }

    fn addr(&mut self, d: u32) {
        match self.r().below(100) {
            0..=69 => {
                self.expr(Ty::I32, d.saturating_sub(1));
                self.op("i32.const 65535");
                self.op("i32.and");
            }
            70..=89 => {
                self.expr(Ty::I32, d.saturating_sub(1));
                self.op("i32.const 131071");
                self.op("i32.and");
            }
            90..=95 => self.constant(Ty::I32),
            _ => self.expr(Ty::I32, d.saturating_sub(1)),
        }
    }

    fn offset(&mut self) -> u32 {
        match self.r().below(25) {
            0..=15 => 0,
            16..=23 => self.r().below(32) as u32,
            _ => self.r().pick(&[0xffff, 0x1_0000, 0xffff_fff0u32, 0x7fff_ffff]),
        }
    }

    fn typed(&mut self, t: Ty, d: u32) {
        match t {
            Ty::I32 => self.typed_i32(d),
            Ty::I64 => self.typed_i64(d),
            Ty::F32 => self.typed_f(Ty::F32, d),
            Ty::F64 => self.typed_f(Ty::F64, d),
        }
    }

    fn unary_float_ops(&mut self, t: Ty) -> &'static str {
        let o: &[&str] = if t == Ty::F32 {
            &["f32.abs", "f32.neg", "f32.sqrt", "f32.ceil", "f32.floor", "f32.trunc", "f32.nearest"]
        } else {
            &["f64.abs", "f64.neg", "f64.sqrt", "f64.ceil", "f64.floor", "f64.trunc", "f64.nearest"]
        };
        self.r().pick(o)
    }

    fn typed_i32(&mut self, d: u32) {
        let d1 = d.saturating_sub(1);
        match self.r().below(100) {
            0..=24 => {
                self.expr(Ty::I32, d1);
                self.expr(Ty::I32, d1);
                let o = self.binop(Ty::I32);
                self.op(o);
            }
            25..=34 => {
                let o = self.r().pick(&["eq", "ne", "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u"]);
                let t = self.r().pick(&[Ty::I32, Ty::I64]);
                self.expr(t, d1);
                self.expr(t, d1);
                self.op(format!("{}.{o}", t.name()));
            }
            35..=39 => {
                let o = self.r().pick(&["eq", "ne", "lt", "gt", "le", "ge"]);
                let t = self.r().pick(&[Ty::F32, Ty::F64]);
                self.expr(t, d1);
                self.expr(t, d1);
                self.op(format!("{}.{o}", t.name()));
            }
            40..=47 => {
                self.expr(Ty::I32, d1);
                let o = self.r().pick(&["i32.clz", "i32.ctz", "i32.popcnt", "i32.eqz", "i32.extend8_s", "i32.extend16_s"]);
                self.op(o);
            }
            48..=50 => {
                self.expr(Ty::I64, d1);
                let o = if self.r().chance(1, 2) { "i64.eqz" } else { "i32.wrap_i64" };
                self.op(o);
            }
            51..=58 => {
                // truncation: trapping and saturating, from f32 and f64
                let s = self.r().pick(&[Ty::F32, Ty::F64]);
                self.expr(s, d1);
                let sat = if self.r().chance(1, 2) { "_sat" } else { "" };
                let sg = self.r().pick(&["s", "u"]);
                self.op(format!("i32.trunc{sat}_{}_{sg}", s.name()));
            }
            59..=61 => {
                self.expr(Ty::F32, d1);
                self.op("i32.reinterpret_f32");
            }
            62..=79 => {
                let o = self.r().pick(&["i32.load", "i32.load8_s", "i32.load8_u", "i32.load16_s", "i32.load16_u"]);
                self.addr(d1);
                let off = self.offset();
                self.op(format!("{o} offset={off}"));
            }
            80..=84 => self.op("memory.size"),
            85..=87 => {
                if self.m.unbounded {
                    let n = if self.r().chance(1, 6) { 1 } else { 0 };
                    self.op(format!("i32.const {n}"));
                } else if self.r().chance(1, 3) {
                    // large, negative and expression deltas: mostly failures (-1)
                    self.expr(Ty::I32, 1);
                } else {
                    let n = self.r().below(3);
                    self.op(format!("i32.const {n}"));
                }
                self.op("memory.grow");
            }
            _ => self.generic(Ty::I32, d),
        }
    }

    fn typed_i64(&mut self, d: u32) {
        let d1 = d.saturating_sub(1);
        match self.r().below(100) {
            0..=29 => {
                self.expr(Ty::I64, d1);
                self.expr(Ty::I64, d1);
                let o = self.binop(Ty::I64);
                self.op(o);
            }
            30..=39 => {
                self.expr(Ty::I64, d1);
                let o = self.r().pick(&["i64.clz", "i64.ctz", "i64.popcnt", "i64.extend8_s", "i64.extend16_s", "i64.extend32_s"]);
                self.op(o);
            }
            40..=47 => {
                self.expr(Ty::I32, d1);
                let o = if self.r().chance(1, 2) { "i64.extend_i32_s" } else { "i64.extend_i32_u" };
                self.op(o);
            }
            48..=57 => {
                let s = self.r().pick(&[Ty::F32, Ty::F64]);
                self.expr(s, d1);
                let sat = if self.r().chance(1, 2) { "_sat" } else { "" };
                let sg = self.r().pick(&["s", "u"]);
                self.op(format!("i64.trunc{sat}_{}_{sg}", s.name()));
            }
            58..=60 => {
                self.expr(Ty::F64, d1);
                self.op("i64.reinterpret_f64");
            }
            61..=79 => {
                let o = self.r().pick(&["i64.load", "i64.load8_s", "i64.load8_u", "i64.load16_s", "i64.load16_u", "i64.load32_s", "i64.load32_u"]);
                self.addr(d1);
                let off = self.offset();
                self.op(format!("{o} offset={off}"));
            }
            80..=89 => {
                // a counted loop with a parameter: [n] loop (param i32) (result i32) ... end, widened
                self.loop_param();
                self.op("i64.extend_i32_u");
            }
            _ => self.generic(Ty::I64, d),
        }
    }

    fn typed_f(&mut self, t: Ty, d: u32) {
        let d1 = d.saturating_sub(1);
        let n = t.name();
        match self.r().below(100) {
            0..=29 => {
                self.expr(t, d1);
                self.expr(t, d1);
                let o = self.binop(t);
                self.canon_op(t, &o);
            }
            30..=44 => {
                self.expr(t, d1);
                let o = self.unary_float_ops(t);
                self.canon_op(t, o);
            }
            45..=56 => {
                let s = self.r().pick(&[Ty::I32, Ty::I64]);
                self.expr(s, d1);
                let sg = self.r().pick(&["s", "u"]);
                self.op(format!("{n}.convert_{}_{sg}", s.name()));
            }
            57..=62 => {
                let (from, op) = if t == Ty::F32 { (Ty::F64, "f32.demote_f64") } else { (Ty::F32, "f64.promote_f32") };
                self.expr(from, d1);
                self.canon_op(t, op);
            }
            63..=67 => {
                let (from, op) = if t == Ty::F32 { (Ty::I32, "f32.reinterpret_i32") } else { (Ty::I64, "f64.reinterpret_i64") };
                self.expr(from, d1);
                self.op(op);
            }
            68..=85 => {
                self.addr(d1);
                let off = self.offset();
                self.op(format!("{n}.load offset={off}"));
            }
            _ => self.generic(t, d),
        }
    }

    /// `i32.const K; loop (param i32) (result i32) [stmts] i32.const 1; i32.sub; local.tee T; local.get T; br_if 0; end`
    fn loop_param(&mut self) {
        let k = self.r().range(1, 5);
        let tmp = self.new_local(Ty::I32);
        self.reserved.push(tmp);
        self.op(format!("i32.const {k}"));
        self.op("loop (param i32) (result i32)");
        self.labels.push(Label { arity: vec![Ty::I32], targetable: true });
        self.op("global.get $fuel");
        self.op("i32.eqz");
        self.op("if");
        self.op("unreachable");
        self.op("end");
        self.op("global.get $fuel");
        self.op("i32.const 1");
        self.op("i32.sub");
        self.op("global.set $fuel");
        self.stmts(1, 0, 2);
        self.op("i32.const 1");
        self.op("i32.sub");
        self.op(format!("local.tee {tmp}"));
        self.op(format!("local.get {tmp}"));
        self.op("br_if 0");
        self.labels.pop();
        self.op("end");
    }

    /// A call. With `want = Some(t)`: leaves exactly one value of type `t` (the first result
    /// of a function whose first result is `t`; extra results go to scratch locals);
    /// with `None`: leaves nothing (results dropped).
    fn call(&mut self, want: Option<Ty>, d: u32) {
        let nf = self.m.sigs.len();
        let direct = self.r().chance(2, 3);
        let cands: Vec<usize> = if direct {
            (self.me + 1..nf).filter(|i| want.is_none_or(|t| self.m.sigs[*i].results.first() == Some(&t))).collect()
        } else {
            (0..nf).filter(|i| want.is_none_or(|t| self.m.sigs[*i].results.first() == Some(&t))).collect()
        };
        if cands.is_empty() {
            return match want {
                Some(t) => self.leaf(t),
                None => self.op("nop"),
            };
        }
        let callee = self.r().pick(&cands);
        let sig = self.m.sigs[callee].clone();
        if direct {
            self.values(&sig.params, d.saturating_sub(1));
            self.op(format!("call $f{callee}"));
        } else {
            // call_indirect: mostly a valid slot of the right type, sometimes null, out of range or another type
            self.values(&sig.params, d.saturating_sub(1));
            let slots: Vec<usize> = (0..self.m.table.len()).filter(|k| self.m.sigs[self.m.table[*k] as usize] == sig).collect();
            let k = self.r().below(100);
            let idx: String = if k < 65 && !slots.is_empty() {
                let s = self.r().pick(&slots) + 1;
                format!("i32.const {s}")
            } else if k < 75 {
                "i32.const 0".to_string()
            } else if k < 83 {
                let v = self.r().range(7, 9);
                format!("i32.const {v}")
            } else if k < 89 {
                "i32.const -1".to_string()
            } else if k < 95 {
                let hi = self.m.table.len() as u64;
                let v = self.r().range(1, hi);
                format!("i32.const {v}")
            } else {
                self.expr(Ty::I32, 1);
                String::new()
            };
            if !idx.is_empty() {
                self.op(idx);
            }
            self.op(format!("call_indirect (type $t{callee})"));
        }
        match want {
            None => {
                for _ in &sig.results {
                    self.op("drop");
                }
            }
            Some(_) => {
                // keep the first result, park the others in scratch locals
                for t in sig.results.iter().skip(1).rev() {
                    let l = self.new_local(*t);
                    self.op(format!("local.set {l}"));
                }
            }
        }
    }

    fn stmts(&mut self, d: u32, lo: u64, hi: u64) {
        let n = self.r().range(lo, hi);
        for _ in 0..n {
            self.stmt(d);
        }
    }

    fn stmt(&mut self, d: u32) {
        self.budget -= 1;
        if self.budget <= 0 || d == 0 {
            // a cheap statement
            let t = self.r().pick(&Ty::ALL);
            self.leaf(t);
            self.op("drop");
            return;
        }
        let d1 = d.saturating_sub(1);
        match self.r().below(100) {
            0..=14 if self.locals.len() > self.reserved.len() => {
                let free: Vec<usize> = (0..self.locals.len()).filter(|i| !self.reserved.contains(i)).collect();
                let i = self.r().pick(&free);
                let t = self.locals[i];
                self.expr(t, d1);
                self.op(format!("local.set {i}"));
            }
            15..=22 => {
                let n = self.m.gtypes.len() as u64;
                let g = self.r().below(n) as usize;
                let t = self.m.gtypes[g];
                self.expr(t, d1);
                self.op(format!("global.set {}", g + 1));
            }
            23..=38 => {
                let (t, op) = self.r().pick(&[
                    (Ty::I32, "i32.store"), (Ty::I64, "i64.store"), (Ty::F32, "f32.store"), (Ty::F64, "f64.store"),
                    (Ty::I32, "i32.store8"), (Ty::I32, "i32.store16"), (Ty::I64, "i64.store8"), (Ty::I64, "i64.store16"), (Ty::I64, "i64.store32"),
                ]);
                self.addr(d1);
                self.expr(t, d1);
                let off = self.offset();
                self.op(format!("{op} offset={off}"));
            }
            39..=42 => {
                let t = self.r().pick(&Ty::ALL);
                self.expr(t, d1);
                self.op("drop");
            }
            43..=46 => {
                self.addr(d1);
                self.expr(Ty::I32, d1);
                self.small_len(d1);
                self.op("memory.fill");
            }
            47..=50 => {
                self.addr(d1);
                self.addr(d1);
                self.small_len(d1);
                self.op("memory.copy");
            }
            51..=58 => self.call(None, d),
            59..=64 => {
                self.expr(Ty::I32, d1);
                self.op("if");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.stmts(d1, 0, 2);
                if self.r().chance(1, 2) {
                    self.op("else");
                    self.stmts(d1, 0, 2);
                }
                self.labels.pop();
                self.op("end");
            }
            65..=70 => {
                self.op("block");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.stmts(d1, 1, 3);
                self.labels.pop();
                self.op("end");
            }
            71..=75 => self.counted_loop(d1),
            76..=82 => {
                // conditional branch to a forward label, carrying its values
                let cands: Vec<usize> = (0..self.labels.len()).filter(|i| self.labels[*i].targetable).collect();
                let i = self.r().pick(&cands);
                let arity = self.labels[i].arity.clone();
                self.values(&arity, d1);
                self.expr(Ty::I32, d1);
                self.op(format!("br_if {}", self.labels.len() - 1 - i));
                for _ in &arity {
                    self.op("drop");
                }
            }
            83..=86 => {
                // unconditional branch/return guarded by an `if`, so only some paths are dead
                let cands: Vec<usize> = (0..self.labels.len()).filter(|i| self.labels[*i].targetable).collect();
                let i = self.r().pick(&cands);
                let arity = self.labels[i].arity.clone();
                self.expr(Ty::I32, d1);
                self.op("if");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.values(&arity, d1);
                self.op(format!("br {}", self.labels.len() - 1 - i));
                self.labels.pop();
                self.op("end");
            }
            87..=91 => {
                // br_table over nested blocks (no values)
                self.op("block");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.op("block");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.op("block");
                self.labels.push(Label { arity: vec![], targetable: true });
                self.stmts(d1, 0, 1);
                self.expr(Ty::I32, d1);
                let n = self.labels.len();
                let ok: Vec<usize> = (0..n).filter(|i| self.labels[*i].targetable && self.labels[*i].arity.is_empty()).collect();
                let cnt = self.r().range(1, 4);
                let targets: Vec<String> = (0..=cnt).map(|_| format!("{}", n - 1 - self.m.r.pick(&ok))).collect();
                self.op(format!("br_table {}", targets.join(" ")));
                self.labels.pop();
                self.op("end");
                self.stmts(d1, 0, 1);
                self.labels.pop();
                self.op("end");
                self.stmts(d1, 0, 1);
                self.labels.pop();
                self.op("end");
            }
            92..=94 => {
                // parameter block as a statement: [t t] block (param t t) (result t) ... end drop
                let t = self.r().pick(&Ty::ALL);
                self.param_block(t, d1);
                self.op("drop");
            }
            95..=96 => {
                self.expr(Ty::I32, d1);
                self.op("if");
                self.op("unreachable");
                self.op("end");
            }
            _ => {
                let t = self.r().pick(&Ty::ALL);
                self.expr(t, d1);
                self.op("drop");
            }
        }
    }

    fn small_len(&mut self, d: u32) {
        self.expr(Ty::I32, d);
        let m = self.r().pick(&[3, 15, 63, 255, 0xffff]);
        self.op(format!("i32.const {m}"));
        self.op("i32.and");
    }

    fn counted_loop(&mut self, d: u32) {
        let c = self.new_local(Ty::I32);
        self.reserved.push(c);
        let k = self.r().range(1, 3);
        self.op(format!("i32.const {k}"));
        self.op(format!("local.set {c}"));
        self.op("loop");
        self.labels.push(Label { arity: vec![], targetable: true });
        self.op("global.get $fuel");
        self.op("i32.eqz");
        self.op("if");
        self.op("unreachable");
        self.op("end");
        self.op("global.get $fuel");
        self.op("i32.const 1");
        self.op("i32.sub");
        self.op("global.set $fuel");
        self.stmts(d, 1, 3);
        self.op(format!("local.get {c}"));
        self.op("i32.const 1");
        self.op("i32.sub");
        self.op(format!("local.tee {c}"));
        self.op("br_if 0");
        self.labels.pop();
        self.op("end");
    }
}
