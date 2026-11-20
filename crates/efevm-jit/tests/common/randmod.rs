//! A seeded generator of random *verified* modules, built with `ModuleBuilder` (not by mutating
//! bytes). Every program terminates: loops have constant trip counts, direct calls only go to
//! higher function indices, and a fuel global (global 0) bounds the number of function entries
//! (indirect calls can recurse). Programs use integer ops of all widths, floats (NaN, +-0,
//! infinities), loads and stores of every width (in and out of bounds), memory size/grow/copy/
//! fill, globals of every type, branches, `br_table`, direct, indirect and host calls, yields
//! and traps.
#![allow(dead_code)]

use super::*;
use efevm::ValType::{F32, F64, I32, I64};

/// SplitMix64: tiny, deterministic, good enough.
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

const INTS: &[i64] = &[
    0,
    1,
    -1,
    2,
    3,
    7,
    8,
    15,
    16,
    31,
    32,
    63,
    64,
    0x7f,
    0x80,
    0xff,
    0x100,
    0x7fff,
    0x8000,
    0xffff,
    0x1_0000,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    0x1_0000_0000,
    i64::MAX,
    i64::MIN,
    i64::MIN + 1,
    -2,
    -128,
    -32768,
    0x0123_4567_89ab_cdef,
];

const F64S: &[f64] = &[
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    1.5,
    2.5,
    -2.5,
    3.5,
    0.1,
    f64::NAN,
    -f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::MIN_POSITIVE,
    f64::MAX,
    f64::MIN,
    2147483647.0,
    2147483648.0,
    -2147483648.0,
    -2147483649.0,
    4294967295.0,
    4294967296.0,
    9223372036854775807.0,
    9223372036854775808.0,
    -9223372036854775808.0,
    18446744073709551615.0,
    18446744073709551616.0,
    1e300,
    5e-324,
    4503599627370496.0,
    4503599627370497.5,
];

const F32S: &[f32] = &[
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    1.5,
    2.5,
    -2.5,
    f32::NAN,
    -f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::MIN_POSITIVE,
    f32::MAX,
    2147483648.0,
    -2147483648.0,
    4294967296.0,
    9223372036854775808.0,
    18446744073709551616.0,
    8388608.5,
    16777216.0,
    1e-45,
    3.4028235e38,
];

/// The host functions of `prog::registry`, in the order they are imported.
const IMPORTS: &[(&str, usize, usize)] = &[
    ("ping", 1, 1),
    ("hy", 1, 1),
    ("memw", 2, 0),
    ("setg", 1, 1),
    ("bad", 1, 1),
];

pub struct Case {
    pub module: Module,
    pub args: Vec<u64>,
    pub budgets: Vec<u64>,
    pub limits: Limits,
    pub opt_none: bool,
    /// The same RNG continues into the run (backend switching pattern).
    pub rng: Rng,
}

struct Env {
    nimports: u32,
    nfuncs: u32,
    /// `(type index, params, results)` of every declared function (module function order).
    ftypes: Vec<(u32, usize, usize)>,
    /// Module type indexes available to `call_indirect`, with `(np, nr)`.
    types: Vec<(u32, usize, usize)>,
    nglobals: u32,
    /// Global index, whether it is mutable.
    mutable: Vec<bool>,
    table_len: u32,
    /// The table, as global function indices.
    table: Vec<Option<u32>>,
    import_tys: Vec<(u32, usize, usize)>,
    safe: bool,
}

struct Fg<'a> {
    r: &'a mut Rng,
    b: FunctionBuilder,
    env: &'a Env,
    /// This function's index among the module's own functions.
    me: u32,
    nregs: u16,
    /// Registers `0..nfree` are the random pool; the rest are scratch and loop counters.
    nfree: u16,
    left: i32,
}

impl Fg<'_> {
    fn reg(&mut self) -> u16 {
        self.r.below(self.nfree as u64) as u16
    }
    fn tmp(&self) -> u16 {
        self.nregs - 1
    }
    fn tmp2(&self) -> u16 {
        self.nregs - 2
    }
    fn tmp3(&self) -> u16 {
        self.nregs - 3
    }
    fn counter(&self, depth: u32) -> u16 {
        self.nregs - 4 - depth as u16
    }
    fn imm(&mut self) -> i64 {
        if self.r.chance(1, 5) {
            self.r.next() as i64
        } else {
            self.r.pick(INTS)
        }
    }

    fn const_any(&mut self, d: u16) {
        match self.r.below(5) {
            0 => {
                let v = self.imm() as i32;
                self.b.const_i32(d, v)
            }
            1 | 2 => {
                let v = self.imm();
                self.b.const_i64(d, v)
            }
            3 => {
                let v = if self.r.chance(1, 6) {
                    f32::from_bits(self.r.next() as u32)
                } else {
                    self.r.pick(F32S)
                };
                self.b.const_f32(d, v)
            }
            _ => {
                let v = if self.r.chance(1, 6) {
                    f64::from_bits(self.r.next())
                } else {
                    self.r.pick(F64S)
                };
                self.b.const_f64(d, v)
            }
        }
    }

    fn alu(&mut self) {
        let d = self.reg();
        let (a, b) = (self.reg(), self.reg());
        let ops = ops();
        match self.r.below(10) {
            0..=5 => {
                let op = self.r.pick(&ops.dab);
                let trapping = ops.trapping.contains(&op);
                if trapping && self.env.safe {
                    // divisor |= 1
                    let (t, one) = (self.tmp(), self.tmp2());
                    self.b.const_i64(one, 1);
                    let or = if op.name().starts_with("i32") {
                        Op::I32Or
                    } else {
                        Op::I64Or
                    };
                    self.b.op3(or, t, b, one);
                    self.b.op3(op, d, a, t);
                } else {
                    self.b.op3(op, d, a, b);
                }
                self.canon_nan(op, d);
            }
            6..=8 => {
                let op = self.r.pick(&ops.da);
                if ops.trapping.contains(&op) && self.env.safe {
                    // clamp through abs so most inputs are in range; NaN and huge values still trap
                    let abs = if op.name().contains("f32") {
                        Op::F32Abs
                    } else {
                        Op::F64Abs
                    };
                    let t = self.tmp();
                    self.b.op2(abs, t, a);
                    self.b.op2(op, d, t);
                } else {
                    self.b.op2(op, d, a);
                }
                self.canon_nan(op, d);
            }
            _ => self.b.mov(d, a),
        }
    }

    /// SPEC: NaN payloads of arithmetic results are "some NaN", so the backends may differ in
    /// them. After a float operation that can produce a NaN, a NaN result is replaced by one
    /// fixed NaN (`select d, d != d, CANON, d`). Whether the result is a NaN, its sign and zero
    /// handling, and everything else stay compared bit for bit.
    fn canon_nan(&mut self, op: Op, d: u16) {
        let n = op.name();
        let (is32, is64) = match op {
            Op::F32DemoteF64 => (true, false),
            Op::F64PromoteF32 => (false, true),
            _ => {
                let nan_op = [
                    "add", "sub", "mul", "div", "min", "max", "sqrt", "ceil", "floor", "trunc",
                    "nearest",
                ]
                .iter()
                .any(|k| n.ends_with(k));
                (
                    nan_op && n.starts_with("f32."),
                    nan_op && n.starts_with("f64."),
                )
            }
        };
        if !(is32 || is64) {
            return;
        }
        let (t, c) = (self.tmp(), self.tmp2());
        if is32 {
            self.b.const_f32(c, f32::NAN);
            self.b.op3(Op::F32Ne, t, d, d);
        } else {
            self.b.const_f64(c, f64::NAN);
            self.b.op3(Op::F64Ne, t, d, d);
        }
        self.b.select(d, t, c, d);
    }

    /// Computes a base register for a memory op and returns it.
    fn base(&mut self) -> u16 {
        let a = self.reg();
        let t = self.tmp();
        let m = if self.env.safe {
            0x7fffi64
        } else {
            self.r.pick(&[0x7fff, 0xffff, 0x1ffff, 0x3fffff])
        };
        let mask = self.tmp2();
        self.b.const_i64(mask, m);
        self.b.op3(Op::I64And, t, a, mask);
        t
    }

    fn offset(&mut self) -> u32 {
        if self.env.safe {
            return self.r.below(0x200) as u32;
        }
        match self.r.below(6) {
            0 => 0,
            1 => self.r.below(16) as u32,
            2 => self.r.range(0xfff0, 0x10010) as u32,
            3 => self
                .r
                .pick(&[0xffff_fff8u32, u32::MAX, 0x8000_0000, 0x1_0000 - 1]),
            _ => self.r.below(0x200) as u32,
        }
    }

    fn mem(&mut self) {
        let ops = ops();
        match self.r.below(12) {
            0..=3 => {
                let op = self.r.pick(&ops.loads);
                let d = self.reg();
                let base = if !self.env.safe && self.r.chance(1, 12) {
                    self.reg()
                } else {
                    self.base()
                };
                let off = self.offset();
                self.b.load(op, d, base, off);
            }
            4..=7 => {
                let op = self.r.pick(&ops.stores);
                let s = self.reg();
                let base = if !self.env.safe && self.r.chance(1, 12) {
                    self.reg()
                } else {
                    self.base()
                };
                let off = self.offset();
                self.b.store(op, s, base, off);
            }
            8 => {
                let d = self.reg();
                self.b.memory_size(d);
            }
            9 => {
                // memory.grow by 0..2 pages (the register holds the page count)
                let (d, t) = (self.reg(), self.tmp());
                let pages = self.r.below(3) as i64;
                self.b.const_i64(t, pages);
                self.b.memory_grow(d, t);
            }
            10 | 11 => {
                let (dst, src, len) = (self.tmp(), self.tmp3(), self.tmp2());
                let (a, c) = (self.reg(), self.reg());
                let m = if self.env.safe {
                    0x7fffi64
                } else {
                    self.r.pick(&[0x7fff, 0xffff, 0x1ffff])
                };
                self.b.const_i64(len, m);
                self.b.op3(Op::I64And, dst, a, len);
                self.b.op3(Op::I64And, src, c, len);
                let l = self.r.below(if self.env.safe { 200 } else { 70000 }) as i64;
                self.b.const_i64(len, l);
                if self.r.chance(1, 2) {
                    self.b.memory_copy(dst, src, len);
                } else {
                    let v = self.reg();
                    self.b.memory_fill(dst, v, len);
                }
            }
            _ => unreachable!(),
        }
    }

    fn global(&mut self) {
        let g = self.r.below(self.env.nglobals as u64) as u32;
        if self.env.mutable[g as usize] && g != 0 && self.r.chance(1, 2) {
            let a = self.reg();
            self.b.global_set(g, a);
        } else {
            let d = self.reg();
            self.b.global_get(d, g);
        }
    }

    fn window(&mut self, np: usize, nr: usize) -> u16 {
        let n = np.max(nr).max(1) as u16;
        let top = self.nfree.saturating_sub(n);
        let base = self.r.below(top as u64 + 1) as u16;
        for i in 0..np as u16 {
            let s = self.reg();
            self.b.mov(base + i, s);
        }
        base
    }

    fn call_direct(&mut self) {
        let env = self.env;
        if self.me + 1 >= env.nfuncs {
            return self.alu();
        }
        let callee = self.r.range(self.me as u64 + 1, env.nfuncs as u64 - 1) as u32;
        let (_, np, nr) = env.ftypes[callee as usize];
        let base = self.window(np, nr);
        self.b.call(env.nimports + callee, base);
    }

    fn call_import(&mut self) {
        let i = self.r.below(self.env.nimports as u64) as usize;
        let (_, np, nr) = IMPORTS[i];
        let base = self.window(np, nr);
        // a few constant arguments make `bad` (13), `hy` (multiples of 3) and `memw` (56) fire
        if np >= 1 && self.r.chance(1, 2) {
            let v = self.r.pick(&[13i64, 3, 6, 9, 56, 0, 12, 24, 100]);
            self.b.const_i64(base, v);
        }
        self.b.call(i as u32, base);
    }

    fn call_indirect(&mut self) {
        let env = self.env;
        // Mostly a slot with the matching type; sometimes a random type or index (traps).
        if self.r.chance(4, 5) {
            let slot = self.r.below(env.table_len as u64) as usize;
            if let Some(g) = env.table[slot] {
                let (ty, np, nr) = if g < env.nimports {
                    env.import_tys[g as usize]
                } else {
                    env.ftypes[(g - env.nimports) as usize]
                };
                let base = self.window(np, nr);
                let t = self.tmp();
                self.b.const_i64(t, slot as i64);
                self.b.call_indirect(ty, t, base);
                return;
            }
        }
        let (ty, np, nr) = self.r.pick(&env.types);
        let base = self.window(np, nr);
        let idx = if env.safe || self.r.chance(3, 4) {
            let (t, m) = (self.tmp(), self.tmp2());
            let a = self.reg();
            self.b.const_i64(
                m,
                env.table_len as i64 - 1 + (!env.safe && self.r.chance(1, 5)) as i64,
            );
            self.b.op3(Op::I64And, t, a, m);
            t
        } else {
            self.reg()
        };
        self.b.call_indirect(ty, idx, base);
    }

    fn nested(&mut self, depth: u32, lo: u64, hi: u64) {
        let n = self.r.range(lo, hi) as u32;
        self.stmts(depth, n);
    }

    fn stmts(&mut self, depth: u32, n: u32) {
        for _ in 0..n {
            if self.left <= 0 {
                return;
            }
            self.stmt(depth);
        }
    }

    fn stmt(&mut self, depth: u32) {
        self.left -= 1;
        let can_nest = depth < 3 && self.left > 4;
        let w = self.r.below(100);
        match w {
            0..=29 => self.alu(),
            30..=37 => {
                let d = self.reg();
                self.const_any(d)
            }
            38..=42 => {
                let (d, c, a, b) = (self.reg(), self.reg(), self.reg(), self.reg());
                self.b.select(d, c, a, b)
            }
            43..=56 => self.mem(),
            57..=62 => self.global(),
            63..=68 => self.call_direct(),
            69..=73 => self.call_import(),
            74..=77 => self.call_indirect(),
            78..=81 if can_nest => {
                let (lelse, lend) = (self.b.new_label(), self.b.new_label());
                let c = self.reg();
                self.b.br_ifz(c, lelse);
                self.nested(depth + 1, 1, 4);
                self.b.jmp(lend);
                self.b.bind(lelse);
                self.nested(depth + 1, 0, 4);
                self.b.bind(lend);
            }
            82..=84 if can_nest && depth < 2 => {
                let cnt = self.counter(depth);
                let l = self.b.new_label();
                self.b.const_i64(cnt, self.r.range(1, 6) as i64);
                self.b.bind(l);
                self.nested(depth + 1, 1, 6);
                let t = self.tmp();
                self.b.const_i64(t, 1);
                self.b.op3(Op::I64Sub, cnt, cnt, t);
                self.b.br_if(cnt, l);
            }
            85..=86 if can_nest => {
                let n = self.r.range(1, 4) as usize;
                let arms: Vec<Label> = (0..n).map(|_| self.b.new_label()).collect();
                let (ldef, lend) = (self.b.new_label(), self.b.new_label());
                let a = self.reg();
                let idx = if self.r.chance(3, 4) {
                    let (t, m) = (self.tmp(), self.tmp2());
                    self.b.const_i64(m, 3);
                    self.b.op3(Op::I64And, t, a, m);
                    t
                } else {
                    a
                };
                self.b.br_table(idx, &arms, ldef);
                for l in arms.iter().chain(std::iter::once(&ldef)) {
                    self.b.bind(*l);
                    self.nested(depth + 1, 0, 3);
                    self.b.jmp(lend);
                }
                self.b.bind(lend);
            }
            87..=89 => self.b.yield_(),
            90..=91 => {
                // conditional early return
                let (c, v) = (self.reg(), self.reg());
                let skip = self.b.new_label();
                self.b.br_ifz(c, skip);
                self.ret_with(v);
                self.b.bind(skip);
            }
            92 if !self.env.safe => {
                let c = self.reg();
                let skip = self.b.new_label();
                self.b.br_ifz(c, skip);
                self.b.unreachable();
                self.b.bind(skip);
            }
            _ => self.alu(),
        }
    }

    fn ret_with(&mut self, v: u16) {
        let (_, _, nr) = self.env.ftypes[self.me as usize];
        if nr == 0 {
            self.b.ret(0);
        } else {
            self.b.ret(v);
        }
    }
}

struct Ops {
    dab: Vec<Op>,
    da: Vec<Op>,
    loads: Vec<Op>,
    stores: Vec<Op>,
    trapping: Vec<Op>,
}

fn ops() -> &'static Ops {
    use std::sync::OnceLock;
    static OPS: OnceLock<Ops> = OnceLock::new();
    OPS.get_or_init(|| {
        let by = |s: Shape| {
            Op::ALL
                .iter()
                .copied()
                .filter(move |o| o.shape() == s)
                .collect::<Vec<_>>()
        };
        let da: Vec<Op> = by(Shape::DA)
            .into_iter()
            .filter(|o| !matches!(o, Op::Mov | Op::MemoryGrow))
            .collect();
        let trapping: Vec<Op> = Op::ALL
            .iter()
            .copied()
            .filter(|o| {
                let n = o.name();
                n.contains("div_")
                    || n.contains("rem_")
                    || (n.contains("trunc_f") && !n.contains("sat"))
            })
            .collect();
        Ops {
            dab: by(Shape::DAB),
            da,
            loads: by(Shape::Load),
            stores: by(Shape::Store),
            trapping,
        }
    })
}

pub fn gen_case(seed: u64) -> Case {
    let mut r = Rng::new(seed);
    let safe = r.chance(3, 5);
    let mut mb = ModuleBuilder::new();
    mb.memory(
        1,
        if r.chance(1, 3) {
            None
        } else {
            Some(r.range(1, 4) as u32)
        },
    );
    let mut data = vec![0u8; 64];
    for b in data.iter_mut() {
        *b = r.next() as u8;
    }
    mb.data(r.below(1000), &data);

    let mut import_tys = Vec::new();
    for (name, np, nr) in IMPORTS {
        mb.import("env", name, ty64(*np, *nr));
        import_tys.push((mb.add_type(ty64(*np, *nr)), *np, *nr));
    }
    // globals: 0 = fuel
    let mut mutable = vec![true];
    mb.global(I64, true, r.range(4, 60));
    let nglob = r.range(2, 6) as usize;
    for _ in 0..nglob {
        let m = r.chance(3, 4);
        let init = match r.below(4) {
            0 => mb.global(I32, m, r.next() & 0xffff_ffff),
            1 => mb.global(I64, m, r.next()),
            2 => mb.global(F32, m, f32::from_bits(r.next() as u32).to_bits() as u64),
            _ => mb.global(F64, m, r.next()),
        };
        let _ = init;
        mutable.push(m);
    }

    let t1 = mb.add_type(ty(&[I64], &[I64]));
    let t2 = mb.add_type(ty(&[I64, I64], &[I64]));
    let t3 = mb.add_type(ty(&[F64], &[I32]));
    let t4 = mb.add_type(ty(&[I64], &[]));
    let tys = [(t1, 1usize, 1usize), (t2, 2, 1), (t3, 1, 1), (t4, 1, 0)];
    let nfuncs = r.range(2, 5) as u32;
    let nimports = IMPORTS.len() as u32;
    let mut ftypes = Vec::new();
    let mut fids = Vec::new();
    for i in 0..nfuncs {
        let (ty_idx, np, nr) = if i == 0 { tys[0] } else { r.pick(&tys) };
        let ft = match ty_idx {
            x if x == t1 => ty(&[I64], &[I64]),
            x if x == t2 => ty(&[I64, I64], &[I64]),
            x if x == t3 => ty(&[F64], &[I32]),
            _ => ty(&[I64], &[]),
        };
        fids.push(mb.declare_func(ft));
        ftypes.push((ty_idx, np, nr));
    }
    let table_len = 8u32;
    let table: Vec<Option<u32>> = (0..table_len)
        .map(|_| match r.below(8) {
            0 => None,
            1 => Some(r.below(nimports as u64) as u32),
            _ => Some(nimports + r.below(nfuncs as u64) as u32),
        })
        .collect();
    mb.table(table.clone());
    let mut types = vec![(t1, 1, 1), (t2, 2, 1), (t3, 1, 1), (t4, 1, 0)];
    types.push((mb.add_type(ty64(2, 0)), 2, 0));
    let env = Env {
        nimports,
        nfuncs,
        ftypes: ftypes.clone(),
        types,
        nglobals: mutable.len() as u32,
        mutable,
        table_len,
        table,
        import_tys,
        safe,
    };

    for me in 0..nfuncs {
        let nregs = r.range(14, 26) as u16;
        let (_, np, nr) = ftypes[me as usize];
        let mut g = Fg {
            r: &mut r,
            b: FunctionBuilder::new(nregs),
            env: &env,
            me,
            nregs,
            nfree: nregs - 5,
            left: 0,
        };
        // fuel check
        let out = g.b.new_label();
        let t = g.tmp();
        g.b.global_get(t, 0);
        g.b.br_ifz(t, out);
        let one = g.tmp2();
        g.b.const_i64(one, 1);
        g.b.op3(Op::I64Sub, t, t, one);
        g.b.global_set(0, t);
        // random initial values in the registers that are not parameters
        for _ in 0..6 {
            let d = g.r.range(np as u64, g.nfree as u64 - 1) as u16;
            g.const_any(d);
        }
        g.left = g.r.range(15, 80) as i32;
        let n = g.left as u32;
        g.stmts(0, n);
        let v = g.reg();
        if nr == 0 {
            g.b.ret(0);
        } else {
            g.b.ret(v);
        }
        g.b.bind(out);
        let z = g.tmp();
        g.b.const_i64(z, 0);
        if nr == 0 {
            g.b.ret(0);
        } else {
            g.b.ret(z);
        }
        mb.define(fids[me as usize], g.b);
    }
    mb.export_func("main", fids[0]);
    let module = mb
        .finish()
        .unwrap_or_else(|e| panic!("seed {seed}: the generated module does not verify: {e}"));
    let args = vec![r.pick(INTS) as u64];
    let nb = r.range(1, 6);
    let mut budgets: Vec<u64> = (0..nb)
        .map(|_| match r.below(14) {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => r.range(3, 12),
            4 | 5 => r.range(10, 64),
            6..=8 => r.range(30, 2000),
            9..=10 => r.range(100, 600),
            _ => u64::MAX,
        })
        .collect();
    if budgets.iter().all(|b| *b == 0) {
        budgets.push(r.range(1, 50));
    }
    let limits = Limits {
        max_call_depth: r.pick(&[2, 4, 8, 20, 4096, 4096, 4096, 4096, 4096, 4096]),
        max_registers: r.pick(&[60, 200, 262_144, 262_144, 262_144, 262_144]),
        max_memory_pages: r.pick(&[2, 3, 1024]),
        ..Limits::default()
    };
    Case {
        module,
        args,
        budgets,
        limits,
        opt_none: r.chance(1, 4),
        rng: r,
    }
}
