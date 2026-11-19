//! The interpreter loop (SPEC.md sections 5-7).

use crate::backend::{FiberStack, Frame, RunCx, invoke_host};
use crate::imports::HostOk;
use crate::module::Inst;
use crate::op::Op;
use crate::trap::{Exit, Trap};
use crate::types::PAGE_SIZE;

/// Bytes per extra budget unit for `memory.copy`, `memory.fill` and `memory.grow`.
pub(crate) const BULK_UNIT_BYTES: u64 = 256;

#[inline(always)]
fn f32v(x: u64) -> f32 {
    f32::from_bits(x as u32)
}
#[inline(always)]
fn f64v(x: u64) -> f64 {
    f64::from_bits(x)
}
#[inline(always)]
fn pf32(x: f32) -> u64 {
    x.to_bits() as u64
}
#[inline(always)]
fn pf64(x: f64) -> u64 {
    x.to_bits()
}

fn fmin32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn fmax32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}
fn fmin64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn fmax64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}

// Trapping truncations. Each returns the raw register value.
fn t_i32_f32_s(x: f32) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -2147483904.0 && x < 2147483648.0 {
        Ok(x as i32 as u32 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i32_f32_u(x: f32) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -1.0 && x < 4294967296.0 {
        Ok(x as u32 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i32_f64_s(x: f64) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -2147483649.0 && x < 2147483648.0 {
        Ok(x as i32 as u32 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i32_f64_u(x: f64) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -1.0 && x < 4294967296.0 {
        Ok(x as u32 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i64_f32_s(x: f32) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if (-9223372036854775808.0..9223372036854775808.0).contains(&x) {
        Ok(x as i64 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i64_f32_u(x: f32) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -1.0 && x < 18446744073709551616.0 {
        Ok(x as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i64_f64_s(x: f64) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if (-9223372036854775808.0..9223372036854775808.0).contains(&x) {
        Ok(x as i64 as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}
fn t_i64_f64_u(x: f64) -> Result<u64, Trap> {
    if x.is_nan() {
        Err(Trap::InvalidConversion)
    } else if x > -1.0 && x < 18446744073709551616.0 {
        Ok(x as u64)
    } else {
        Err(Trap::IntegerOverflow)
    }
}

/// Runs a fiber until it exits. Returns the exit and the final budget counter (negative when
/// the last block or a bulk operation overran it; the caller turns that into debt).
/// The budget is a signed counter charged per basic block (SPEC section 5): at a leader,
/// a counter `<= 0` exits `OutOfBudget` there, otherwise the block's cost is subtracted.
///
/// With `STEP` the run executes exactly one instruction without charging anything and returns
/// `OutOfBudget` if the fiber is still runnable.
///
/// On `Exit::Done` the results are in `st.results`. On `Yield`/`OutOfBudget` the position is
/// saved in `st` so the fiber can be resumed.
pub(crate) fn run<H, const STEP: bool>(
    cx: &mut RunCx<'_, H>,
    st: &mut FiberStack,
    mut budget: i64,
) -> (Exit, i64) {
    let FiberStack { regs, frames, scratch, func: st_func, pc: st_pc, fp: st_fp, results, trace, debt: _ } = st;
    let RunCx { module, infos, type_canon, limits, memory, globals, fiber, host, hosts, gmut, gmask, res_masks } =
        cx;
    let (memory, globals, host): (&mut crate::memory::Memory, &mut [u64], &mut H) =
        (&mut **memory, &mut **globals, &mut **host);
    let module: &crate::module::Module = module;
    let data = module.data();
    let (infos, type_canon, limits, fiber): (&[_], &[u32], _, _) = (infos, type_canon, *limits, *fiber);
    let (hosts, gmut, gmask, res_masks) = (*hosts, *gmut, *gmask, *res_masks);
    let nimports = data.imports.len();
    let mut func = *st_func;
    let mut pc = *st_pc as usize;
    let mut fp = *st_fp as usize;

    let empty: &[Inst] = &[];
    let mut code: &[Inst] = match (func as usize).checked_sub(nimports).and_then(|i| data.functions.get(i)) {
        Some(f) => &f.code,
        None => empty,
    };
    // Cost per pc of the current function: nonzero exactly at block leaders.
    let mut cost: &[u32] = module.cost_table(func);
    let mut f: &mut [u64] = match regs.get_mut(fp..) {
        Some(s) => s,
        None => return (Exit::Trap(Trap::Unreachable), budget),
    };

    macro_rules! save {
        () => {{
            *st_func = func;
            *st_pc = pc as u32;
            *st_fp = fp as u32;
        }};
    }
    macro_rules! trap {
        ($t:expr) => {{
            let t: Trap = $t;
            trace.clear();
            trace.push((func, (pc as u32).saturating_sub(1)));
            for fr in frames.iter().rev() {
                trace.push((fr.func, fr.pc.saturating_sub(1)));
            }
            return (Exit::Trap(t), budget);
        }};
    }
    // Subtracts the extra units of a bulk operation (beyond its block cost). No immediate
    // exit: a counter that goes negative stops the fiber at the next leader.
    macro_rules! charge {
        ($units:expr) => {{
            let units: u64 = $units;
            budget = budget.saturating_sub(units.min(i64::MAX as u64) as i64);
        }};
    }
    macro_rules! tr {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(t) => trap!(t),
            }
        };
    }
    macro_rules! ra {
        ($i:expr) => {
            f[$i.a as usize]
        };
    }
    // d=a, x=b, y=c
    macro_rules! i32b {
        ($i:ident, |$x:ident, $y:ident| $e:expr) => {{
            let $x = f[$i.b as usize] as u32;
            let $y = f[$i.c as usize] as u32;
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! i64b {
        ($i:ident, |$x:ident, $y:ident| $e:expr) => {{
            let $x = f[$i.b as usize];
            let $y = f[$i.c as usize];
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! f32b {
        ($i:ident, |$x:ident, $y:ident| $e:expr) => {{
            let $x = f32v(f[$i.b as usize]);
            let $y = f32v(f[$i.c as usize]);
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! f64b {
        ($i:ident, |$x:ident, $y:ident| $e:expr) => {{
            let $x = f64v(f[$i.b as usize]);
            let $y = f64v(f[$i.c as usize]);
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! u32u {
        ($i:ident, |$x:ident| $e:expr) => {{
            let $x = f[$i.b as usize] as u32;
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! u64u {
        ($i:ident, |$x:ident| $e:expr) => {{
            let $x = f[$i.b as usize];
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! f32u {
        ($i:ident, |$x:ident| $e:expr) => {{
            let $x = f32v(f[$i.b as usize]);
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! f64u {
        ($i:ident, |$x:ident| $e:expr) => {{
            let $x = f64v(f[$i.b as usize]);
            f[$i.a as usize] = ($e) as u64;
        }};
    }
    macro_rules! load {
        ($i:ident, $n:literal, |$x:ident| $e:expr) => {{
            let $x = tr!(memory.load::<$n>(f[$i.b as usize], $i.c));
            f[$i.a as usize] = $e;
        }};
    }
    macro_rules! store {
        ($i:ident, $n:literal, |$x:ident| $e:expr) => {{
            let $x = f[$i.a as usize];
            tr!(memory.store::<$n>(f[$i.b as usize], $i.c, $e));
        }};
    }
    macro_rules! jump {
        ($l:expr) => {{
            pc = $l as usize;
        }};
    }

    loop {
        if !STEP {
            // A nonzero cost marks a leader; every instruction passes here once per execution,
            // so a block is charged exactly once per entry.
            let c = cost.get(pc).copied().unwrap_or(0);
            if c != 0 {
                if budget <= 0 {
                    save!();
                    return (Exit::OutOfBudget, budget);
                }
                budget -= c as i64;
            }
        }
        let inst: Inst = match code.get(pc) {
            Some(i) => *i,
            None => {
                pc += 1;
                trap!(Trap::Unreachable)
            }
        };
        pc += 1;
        match inst.op {
            Op::Nop => {}
            Op::Unreachable => trap!(Trap::Unreachable),
            Op::Yield => {
                save!();
                return (Exit::Yield, budget);
            }
            Op::Ret => {
                let base = inst.a as usize;
                let nr = infos.get(func as usize).map_or(0, |i| i.nr as usize);
                if base + nr > f.len() {
                    trap!(Trap::OutOfBounds);
                }
                match frames.pop() {
                    None => {
                        results.clear();
                        results.extend_from_slice(&f[base..base + nr]);
                        save!();
                        return (Exit::Done, budget);
                    }
                    Some(fr) => {
                        let dst = fr.fp as usize + fr.base as usize;
                        regs.copy_within(fp + base..fp + base + nr, dst);
                        regs.truncate(fp);
                        func = fr.func;
                        pc = fr.pc as usize;
                        fp = fr.fp as usize;
                        code = match (func as usize).checked_sub(nimports).and_then(|i| data.functions.get(i)) {
                            Some(fun) => &fun.code,
                            None => trap!(Trap::Unreachable),
                        };
                        cost = module.cost_table(func);
                        f = match regs.get_mut(fp..) {
                            Some(s) => s,
                            None => trap!(Trap::Unreachable),
                        };
                    }
                }
            }
            Op::Jmp => jump!(inst.a),
            Op::BrIf => {
                if f[inst.a as usize] as u32 != 0 {
                    jump!(inst.b);
                }
            }
            Op::BrIfz => {
                if f[inst.a as usize] as u32 == 0 {
                    jump!(inst.b);
                }
            }
            Op::BrTable => {
                let idx = f[inst.a as usize] as u32 as usize;
                let tab = match data
                    .functions
                    .get((func as usize).wrapping_sub(nimports))
                    .and_then(|fun| fun.br_tables.get(inst.b as usize))
                {
                    Some(t) => t,
                    None => trap!(Trap::Unreachable),
                };
                let l = tab.targets.get(idx).copied().unwrap_or(tab.default);
                jump!(l);
            }
            Op::Call | Op::CallIndirect => {
                let (g, base) = if inst.op == Op::Call {
                    (inst.a, inst.b as usize)
                } else {
                    let idx = f[inst.b as usize] as u32 as usize;
                    let g = match data.table.get(idx) {
                        Some(Some(g)) => *g,
                        _ => trap!(Trap::UndefinedElement),
                    };
                    let want = type_canon.get(inst.a as usize).copied();
                    let have = infos.get(g as usize).map(|i| i.canon);
                    if want.is_none() || want != have {
                        trap!(Trap::IndirectCallTypeMismatch);
                    }
                    (g, inst.c as usize)
                };
                let info = match infos.get(g as usize) {
                    Some(i) => *i,
                    None => trap!(Trap::Unreachable),
                };
                let (np, nr) = (info.np as usize, info.nr as usize);
                if base + np.max(nr) > f.len() {
                    trap!(Trap::OutOfBounds);
                }
                if (g as usize) < nimports {
                    // Host call.
                    scratch.clear();
                    scratch.resize(nr, 0);
                    let r = invoke_host(
                        hosts,
                        res_masks,
                        &mut *host,
                        &mut *memory,
                        &mut *globals,
                        gmut,
                        gmask,
                        fiber,
                        g,
                        &f[base..base + np],
                        &mut scratch[..],
                    );
                    match r {
                        Ok(k) => {
                            f[base..base + nr].copy_from_slice(&scratch[..nr]);
                            if k == HostOk::Yield {
                                save!();
                                return (Exit::Yield, budget);
                            }
                        }
                        Err(t) => trap!(t),
                    }
                } else {
                    if frames.len() as u64 + 2 > limits.max_call_depth as u64 {
                        trap!(Trap::StackOverflow);
                    }
                    let size = (info.nregs as usize).max(np);
                    let new_fp = fp + f.len();
                    let total = new_fp + size;
                    if total as u64 > limits.max_registers as u64 {
                        trap!(Trap::StackOverflow);
                    }
                    if total > regs.capacity() && regs.try_reserve(total.saturating_sub(regs.len())).is_err() {
                        trap!(Trap::StackOverflow);
                    }
                    frames.push(Frame { func, pc: pc as u32, fp: fp as u32, base: base as u32 });
                    regs.resize(total, 0);
                    regs.copy_within(fp + base..fp + base + np, new_fp);
                    func = g;
                    fp = new_fp;
                    pc = 0;
                    code = match data.functions.get(g as usize - nimports) {
                        Some(fun) => &fun.code,
                        None => trap!(Trap::Unreachable),
                    };
                    cost = module.cost_table(g);
                    f = match regs.get_mut(fp..) {
                        Some(s) => s,
                        None => trap!(Trap::Unreachable),
                    };
                }
            }
            Op::ConstI32 | Op::ConstF32 => ra!(inst) = inst.b as u64,
            Op::ConstI64 | Op::ConstF64 => ra!(inst) = inst.const_i64_value(),
            Op::Mov => f[inst.a as usize] = f[inst.b as usize],
            Op::Select => {
                let (x, y) = inst.select_regs();
                let v = if f[inst.b as usize] as u32 != 0 { f[x as usize] } else { f[y as usize] };
                ra!(inst) = v;
            }
            Op::GlobalGet => match globals.get(inst.b as usize) {
                Some(v) => ra!(inst) = *v,
                None => trap!(Trap::OutOfBounds),
            },
            Op::GlobalSet => {
                let v = f[inst.b as usize];
                match globals.get_mut(inst.a as usize) {
                    Some(g) => *g = v,
                    None => trap!(Trap::OutOfBounds),
                }
            }
            Op::MemorySize => ra!(inst) = memory.size_pages(),
            Op::MemoryGrow => {
                let delta = f[inst.b as usize];
                ra!(inst) = memory.grow(delta).unwrap_or(u64::MAX);
                // Cost 1 + pages * 65536 / 256, charged even when the grow fails.
                charge!(delta.saturating_mul(PAGE_SIZE / BULK_UNIT_BYTES));
            }
            Op::MemoryCopy => {
                let len = f[inst.c as usize];
                tr!(memory.copy(ra!(inst), f[inst.b as usize], len));
                charge!(len / BULK_UNIT_BYTES);
            }
            Op::MemoryFill => {
                let len = f[inst.c as usize];
                tr!(memory.fill(ra!(inst), f[inst.b as usize] as u8, len));
                charge!(len / BULK_UNIT_BYTES);
            }

            // ---- i32 ----
            Op::I32Add => i32b!(inst, |x, y| x.wrapping_add(y)),
            Op::I32Sub => i32b!(inst, |x, y| x.wrapping_sub(y)),
            Op::I32Mul => i32b!(inst, |x, y| x.wrapping_mul(y)),
            Op::I32DivS => {
                let (x, y) = (f[inst.b as usize] as u32 as i32, f[inst.c as usize] as u32 as i32);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                if x == i32::MIN && y == -1 {
                    trap!(Trap::IntegerOverflow);
                }
                ra!(inst) = (x / y) as u32 as u64;
            }
            Op::I32DivU => {
                let (x, y) = (f[inst.b as usize] as u32, f[inst.c as usize] as u32);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = (x / y) as u64;
            }
            Op::I32RemS => {
                let (x, y) = (f[inst.b as usize] as u32 as i32, f[inst.c as usize] as u32 as i32);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = x.wrapping_rem(y) as u32 as u64;
            }
            Op::I32RemU => {
                let (x, y) = (f[inst.b as usize] as u32, f[inst.c as usize] as u32);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = (x % y) as u64;
            }
            Op::I32And => i32b!(inst, |x, y| x & y),
            Op::I32Or => i32b!(inst, |x, y| x | y),
            Op::I32Xor => i32b!(inst, |x, y| x ^ y),
            Op::I32Shl => i32b!(inst, |x, y| x.wrapping_shl(y)),
            Op::I32ShrS => i32b!(inst, |x, y| (x as i32).wrapping_shr(y) as u32),
            Op::I32ShrU => i32b!(inst, |x, y| x.wrapping_shr(y)),
            Op::I32Rotl => i32b!(inst, |x, y| x.rotate_left(y & 31)),
            Op::I32Rotr => i32b!(inst, |x, y| x.rotate_right(y & 31)),
            Op::I32Clz => u32u!(inst, |x| x.leading_zeros()),
            Op::I32Ctz => u32u!(inst, |x| x.trailing_zeros()),
            Op::I32Popcnt => u32u!(inst, |x| x.count_ones()),
            Op::I32Eqz => u32u!(inst, |x| x == 0),
            Op::I32Eq => i32b!(inst, |x, y| x == y),
            Op::I32Ne => i32b!(inst, |x, y| x != y),
            Op::I32LtS => i32b!(inst, |x, y| (x as i32) < (y as i32)),
            Op::I32LtU => i32b!(inst, |x, y| x < y),
            Op::I32GtS => i32b!(inst, |x, y| (x as i32) > (y as i32)),
            Op::I32GtU => i32b!(inst, |x, y| x > y),
            Op::I32LeS => i32b!(inst, |x, y| (x as i32) <= (y as i32)),
            Op::I32LeU => i32b!(inst, |x, y| x <= y),
            Op::I32GeS => i32b!(inst, |x, y| (x as i32) >= (y as i32)),
            Op::I32GeU => i32b!(inst, |x, y| x >= y),

            // ---- i64 ----
            Op::I64Add => i64b!(inst, |x, y| x.wrapping_add(y)),
            Op::I64Sub => i64b!(inst, |x, y| x.wrapping_sub(y)),
            Op::I64Mul => i64b!(inst, |x, y| x.wrapping_mul(y)),
            Op::I64DivS => {
                let (x, y) = (f[inst.b as usize] as i64, f[inst.c as usize] as i64);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                if x == i64::MIN && y == -1 {
                    trap!(Trap::IntegerOverflow);
                }
                ra!(inst) = (x / y) as u64;
            }
            Op::I64DivU => {
                let (x, y) = (f[inst.b as usize], f[inst.c as usize]);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = x / y;
            }
            Op::I64RemS => {
                let (x, y) = (f[inst.b as usize] as i64, f[inst.c as usize] as i64);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = x.wrapping_rem(y) as u64;
            }
            Op::I64RemU => {
                let (x, y) = (f[inst.b as usize], f[inst.c as usize]);
                if y == 0 {
                    trap!(Trap::DivideByZero);
                }
                ra!(inst) = x % y;
            }
            Op::I64And => i64b!(inst, |x, y| x & y),
            Op::I64Or => i64b!(inst, |x, y| x | y),
            Op::I64Xor => i64b!(inst, |x, y| x ^ y),
            Op::I64Shl => i64b!(inst, |x, y| x.wrapping_shl(y as u32)),
            Op::I64ShrS => i64b!(inst, |x, y| (x as i64).wrapping_shr(y as u32) as u64),
            Op::I64ShrU => i64b!(inst, |x, y| x.wrapping_shr(y as u32)),
            Op::I64Rotl => i64b!(inst, |x, y| x.rotate_left((y & 63) as u32)),
            Op::I64Rotr => i64b!(inst, |x, y| x.rotate_right((y & 63) as u32)),
            Op::I64Clz => u64u!(inst, |x| x.leading_zeros()),
            Op::I64Ctz => u64u!(inst, |x| x.trailing_zeros()),
            Op::I64Popcnt => u64u!(inst, |x| x.count_ones()),
            Op::I64Eqz => u64u!(inst, |x| x == 0),
            Op::I64Eq => i64b!(inst, |x, y| x == y),
            Op::I64Ne => i64b!(inst, |x, y| x != y),
            Op::I64LtS => i64b!(inst, |x, y| (x as i64) < (y as i64)),
            Op::I64LtU => i64b!(inst, |x, y| x < y),
            Op::I64GtS => i64b!(inst, |x, y| (x as i64) > (y as i64)),
            Op::I64GtU => i64b!(inst, |x, y| x > y),
            Op::I64LeS => i64b!(inst, |x, y| (x as i64) <= (y as i64)),
            Op::I64LeU => i64b!(inst, |x, y| x <= y),
            Op::I64GeS => i64b!(inst, |x, y| (x as i64) >= (y as i64)),
            Op::I64GeU => i64b!(inst, |x, y| x >= y),

            // ---- loads / stores ----
            Op::I32Load | Op::F32Load => load!(inst, 4, |b| u32::from_le_bytes(b) as u64),
            Op::I64Load | Op::F64Load => load!(inst, 8, |b| u64::from_le_bytes(b)),
            Op::I32Store | Op::F32Store | Op::I64Store32 => {
                store!(inst, 4, |v| (v as u32).to_le_bytes())
            }
            Op::I64Store | Op::F64Store => store!(inst, 8, |v| v.to_le_bytes()),
            Op::I32Load8S => load!(inst, 1, |b| b[0] as i8 as i32 as u32 as u64),
            Op::I32Load8U | Op::I64Load8U => load!(inst, 1, |b| b[0] as u64),
            Op::I32Load16S => load!(inst, 2, |b| i16::from_le_bytes(b) as i32 as u32 as u64),
            Op::I32Load16U | Op::I64Load16U => load!(inst, 2, |b| u16::from_le_bytes(b) as u64),
            Op::I64Load8S => load!(inst, 1, |b| b[0] as i8 as i64 as u64),
            Op::I64Load16S => load!(inst, 2, |b| i16::from_le_bytes(b) as i64 as u64),
            Op::I64Load32S => load!(inst, 4, |b| i32::from_le_bytes(b) as i64 as u64),
            Op::I64Load32U => load!(inst, 4, |b| u32::from_le_bytes(b) as u64),
            Op::I32Store8 | Op::I64Store8 => store!(inst, 1, |v| [v as u8]),
            Op::I32Store16 | Op::I64Store16 => store!(inst, 2, |v| (v as u16).to_le_bytes()),

            // ---- f32 ----
            Op::F32Add => f32b!(inst, |x, y| pf32(x + y)),
            Op::F32Sub => f32b!(inst, |x, y| pf32(x - y)),
            Op::F32Mul => f32b!(inst, |x, y| pf32(x * y)),
            Op::F32Div => f32b!(inst, |x, y| pf32(x / y)),
            Op::F32Min => f32b!(inst, |x, y| pf32(fmin32(x, y))),
            Op::F32Max => f32b!(inst, |x, y| pf32(fmax32(x, y))),
            Op::F32Copysign => f32b!(inst, |x, y| pf32(x.copysign(y))),
            Op::F32Abs => u64u!(inst, |x| x & 0x7FFF_FFFF),
            Op::F32Neg => u64u!(inst, |x| (x ^ 0x8000_0000) & 0xFFFF_FFFF),
            Op::F32Sqrt => f32u!(inst, |x| pf32(x.sqrt())),
            Op::F32Ceil => f32u!(inst, |x| pf32(x.ceil())),
            Op::F32Floor => f32u!(inst, |x| pf32(x.floor())),
            Op::F32Trunc => f32u!(inst, |x| pf32(x.trunc())),
            Op::F32Nearest => f32u!(inst, |x| pf32(x.round_ties_even())),
            Op::F32Eq => f32b!(inst, |x, y| x == y),
            Op::F32Ne => f32b!(inst, |x, y| x != y),
            Op::F32Lt => f32b!(inst, |x, y| x < y),
            Op::F32Gt => f32b!(inst, |x, y| x > y),
            Op::F32Le => f32b!(inst, |x, y| x <= y),
            Op::F32Ge => f32b!(inst, |x, y| x >= y),

            // ---- f64 ----
            Op::F64Add => f64b!(inst, |x, y| pf64(x + y)),
            Op::F64Sub => f64b!(inst, |x, y| pf64(x - y)),
            Op::F64Mul => f64b!(inst, |x, y| pf64(x * y)),
            Op::F64Div => f64b!(inst, |x, y| pf64(x / y)),
            Op::F64Min => f64b!(inst, |x, y| pf64(fmin64(x, y))),
            Op::F64Max => f64b!(inst, |x, y| pf64(fmax64(x, y))),
            Op::F64Copysign => f64b!(inst, |x, y| pf64(x.copysign(y))),
            Op::F64Abs => u64u!(inst, |x| x & 0x7FFF_FFFF_FFFF_FFFF),
            Op::F64Neg => u64u!(inst, |x| x ^ 0x8000_0000_0000_0000),
            Op::F64Sqrt => f64u!(inst, |x| pf64(x.sqrt())),
            Op::F64Ceil => f64u!(inst, |x| pf64(x.ceil())),
            Op::F64Floor => f64u!(inst, |x| pf64(x.floor())),
            Op::F64Trunc => f64u!(inst, |x| pf64(x.trunc())),
            Op::F64Nearest => f64u!(inst, |x| pf64(x.round_ties_even())),
            Op::F64Eq => f64b!(inst, |x, y| x == y),
            Op::F64Ne => f64b!(inst, |x, y| x != y),
            Op::F64Lt => f64b!(inst, |x, y| x < y),
            Op::F64Gt => f64b!(inst, |x, y| x > y),
            Op::F64Le => f64b!(inst, |x, y| x <= y),
            Op::F64Ge => f64b!(inst, |x, y| x >= y),

            // ---- width conversions ----
            Op::I32WrapI64 => u64u!(inst, |x| x as u32),
            Op::I64ExtendI32S => u32u!(inst, |x| x as i32 as i64),
            Op::I64ExtendI32U => u32u!(inst, |x| x),
            Op::I32Extend8S => u32u!(inst, |x| x as i8 as i32 as u32),
            Op::I32Extend16S => u32u!(inst, |x| x as i16 as i32 as u32),
            Op::I64Extend8S => u64u!(inst, |x| x as i8 as i64),
            Op::I64Extend16S => u64u!(inst, |x| x as i16 as i64),
            Op::I64Extend32S => u64u!(inst, |x| x as i32 as i64),

            // ---- trunc ----
            Op::I32TruncF32S => ra!(inst) = tr!(t_i32_f32_s(f32v(f[inst.b as usize]))),
            Op::I32TruncF32U => ra!(inst) = tr!(t_i32_f32_u(f32v(f[inst.b as usize]))),
            Op::I32TruncF64S => ra!(inst) = tr!(t_i32_f64_s(f64v(f[inst.b as usize]))),
            Op::I32TruncF64U => ra!(inst) = tr!(t_i32_f64_u(f64v(f[inst.b as usize]))),
            Op::I64TruncF32S => ra!(inst) = tr!(t_i64_f32_s(f32v(f[inst.b as usize]))),
            Op::I64TruncF32U => ra!(inst) = tr!(t_i64_f32_u(f32v(f[inst.b as usize]))),
            Op::I64TruncF64S => ra!(inst) = tr!(t_i64_f64_s(f64v(f[inst.b as usize]))),
            Op::I64TruncF64U => ra!(inst) = tr!(t_i64_f64_u(f64v(f[inst.b as usize]))),
            Op::I32TruncSatF32S => f32u!(inst, |x| x as i32 as u32),
            Op::I32TruncSatF32U => f32u!(inst, |x| x as u32),
            Op::I32TruncSatF64S => f64u!(inst, |x| x as i32 as u32),
            Op::I32TruncSatF64U => f64u!(inst, |x| x as u32),
            Op::I64TruncSatF32S => f32u!(inst, |x| x as i64),
            Op::I64TruncSatF32U => f32u!(inst, |x| x as u64),
            Op::I64TruncSatF64S => f64u!(inst, |x| x as i64),
            Op::I64TruncSatF64U => f64u!(inst, |x| x as u64),

            // ---- convert / demote / promote / reinterpret ----
            Op::F32ConvertI32S => u32u!(inst, |x| pf32(x as i32 as f32)),
            Op::F32ConvertI32U => u32u!(inst, |x| pf32(x as f32)),
            Op::F32ConvertI64S => u64u!(inst, |x| pf32(x as i64 as f32)),
            Op::F32ConvertI64U => u64u!(inst, |x| pf32(x as f32)),
            Op::F64ConvertI32S => u32u!(inst, |x| pf64(x as i32 as f64)),
            Op::F64ConvertI32U => u32u!(inst, |x| pf64(x as f64)),
            Op::F64ConvertI64S => u64u!(inst, |x| pf64(x as i64 as f64)),
            Op::F64ConvertI64U => u64u!(inst, |x| pf64(x as f64)),
            Op::F32DemoteF64 => f64u!(inst, |x| pf32(x as f32)),
            Op::F64PromoteF32 => f32u!(inst, |x| pf64(x as f64)),
            Op::I32ReinterpretF32 | Op::F32ReinterpretI32 => u64u!(inst, |x| x & 0xFFFF_FFFF),
            Op::I64ReinterpretF64 | Op::F64ReinterpretI64 => u64u!(inst, |x| x),
        }
        if STEP {
            save!();
            return (Exit::OutOfBudget, budget);
        }
    }
}
