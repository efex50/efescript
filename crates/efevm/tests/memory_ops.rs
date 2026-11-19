//! Instruction tests for constants, moves, select, nop, linear memory loads/stores,
//! memory.size/grow/copy/fill, globals and data segments (SPEC.md sections 3, 5, 8).

use efevm::{
    Exit, FuncType, FunctionBuilder, Imports, Instance, Limits, Module, ModuleBuilder, Op,
    Trap, ValType,
};

const BUDGET: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Builds a module whose only function "main" takes no parameters and returns `nres`
/// registers `r0 .. r(nres-1)`. `setup` may add globals, data and so on before the
/// function is added. `body` fills the registers; `ret 0` is appended.
fn module_with(
    setup: impl FnOnce(&mut ModuleBuilder),
    nres: usize,
    nregs: u16,
    body: impl FnOnce(&mut FunctionBuilder),
) -> Module {
    let mut mb = ModuleBuilder::new();
    setup(&mut mb);
    let mut fb = FunctionBuilder::new(nregs);
    body(&mut fb);
    fb.ret(0);
    let f = mb.add_func(FuncType::new([], vec![ValType::I64; nres]), fb);
    mb.export_func("main", f);
    mb.finish().expect("module verifies")
}

/// `module_with` with one optional memory declaration `(min_pages, max_pages)`.
fn module(
    mem: Option<(u32, Option<u32>)>,
    nres: usize,
    nregs: u16,
    body: impl FnOnce(&mut FunctionBuilder),
) -> Module {
    module_with(
        |mb| {
            if let Some((min, max)) = mem {
                mb.memory(min, max);
            }
        },
        nres,
        nregs,
        body,
    )
}

fn instantiate(m: &Module, limits: Limits) -> Instance<()> {
    Instance::new(m, &Imports::<()>::new(), limits).expect("instantiate")
}

/// Spawns "main", runs it to completion or trap, returns the exit and the results.
fn exec(inst: &mut Instance<()>) -> (Exit, Vec<u64>) {
    let f = inst.spawn("main", &[]).expect("spawn main");
    let exit = inst.resume(f, &mut (), BUDGET);
    let results = inst.results(f).map(|r| r.to_vec()).unwrap_or_default();
    (exit, results)
}

/// Instantiates with default limits and runs; returns the instance, exit and results.
fn run(m: &Module) -> (Instance<()>, Exit, Vec<u64>) {
    let mut inst = instantiate(m, Limits::default());
    let (exit, results) = exec(&mut inst);
    (inst, exit, results)
}

/// Runs a module that must finish, returning its results.
fn run_ok(m: &Module) -> Vec<u64> {
    let (_, exit, results) = run(m);
    assert_eq!(exit, Exit::Done);
    results
}

/// Runs a module that must trap, returning the trap.
fn run_trap(m: &Module) -> Trap {
    match run(m).1 {
        Exit::Trap(t) => t,
        other => panic!("expected a trap, got {other:?}"),
    }
}

/// Loads `op` from address `base + off` in a 1-page memory whose first bytes are `init`.
/// Returns the register value or the trap.
fn load_case(op: Op, init: &[u8], base: u64, off: u32) -> Result<u64, Trap> {
    let m = module(Some((1, None)), 1, 4, |fb| {
        fb.const_i64(1, base as i64);
        fb.load(op, 0, 1, off);
    });
    let mut inst = instantiate(&m, Limits::default());
    inst.memory_mut().write(0, init).expect("init fits");
    match exec(&mut inst) {
        (Exit::Done, r) => Ok(r[0]),
        (Exit::Trap(t), _) => Err(t),
        (other, _) => panic!("unexpected exit {other:?}"),
    }
}

/// Byte pattern used to detect unintended writes.
const PATTERN: u8 = 0xAA;

/// Stores register value `value` with `op` at `base + off` in a 1-page memory filled with
/// `PATTERN`. Returns the exit and the whole memory afterwards.
fn store_case(op: Op, value: u64, base: u64, off: u32) -> (Exit, Vec<u8>) {
    let m = module(Some((1, None)), 0, 4, |fb| {
        fb.const_i64(0, value as i64);
        fb.const_i64(1, base as i64);
        fb.store(op, 0, 1, off);
    });
    let mut inst = instantiate(&m, Limits::default());
    inst.memory_mut().as_mut_slice().fill(PATTERN);
    let (exit, _) = exec(&mut inst);
    (exit, inst.memory().as_slice().to_vec())
}

/// Memory contents expected when only `PATTERN` was written.
fn pattern_memory() -> Vec<u8> {
    vec![PATTERN; 65536]
}

/// Runs a bulk-memory body on a 1-page memory. Memory starts as `0x5A` everywhere with
/// `init` written at address 0. Returns the exit and the whole memory afterwards.
const BULK_BASE: u8 = 0x5A;

fn bulk_case(init: &[u8], body: impl FnOnce(&mut FunctionBuilder)) -> (Exit, Vec<u8>) {
    let m = module(Some((1, None)), 0, 4, body);
    let mut inst = instantiate(&m, Limits::default());
    inst.memory_mut().as_mut_slice().fill(BULK_BASE);
    inst.memory_mut().write(0, init).expect("init fits");
    let (exit, _) = exec(&mut inst);
    (exit, inst.memory().as_slice().to_vec())
}

/// Memory contents when a bulk case has not run at all.
fn bulk_initial(init: &[u8]) -> Vec<u8> {
    let mut m = vec![BULK_BASE; 65536];
    m[..init.len()].copy_from_slice(init);
    m
}

// ---------------------------------------------------------------------------
// constants, mov, select, nop
// ---------------------------------------------------------------------------

#[test]
fn const_i32_is_zero_extended() {
    let r = run_ok(&module(None, 1, 1, |fb| fb.const_i32(0, -1)));
    assert_eq!(r, vec![0x0000_0000_FFFF_FFFF]);
}

#[test]
fn const_i64_minus_one_is_all_ones() {
    let r = run_ok(&module(None, 1, 1, |fb| fb.const_i64(0, -1)));
    assert_eq!(r, vec![u64::MAX]);
}

#[test]
fn const_f32_stores_bits_in_low_word() {
    let r = run_ok(&module(None, 1, 1, |fb| fb.const_f32(0, 1.5)));
    assert_eq!(r, vec![1.5f32.to_bits() as u64]);
}

#[test]
fn const_f64_bits_are_exact() {
    let nan_payload = f64::from_bits(0x7FF8_0000_0000_BEEF);
    let neg_zero = -0.0f64;
    let r = run_ok(&module(None, 3, 3, |fb| {
        fb.const_f64(0, 2.5);
        fb.const_f64(1, nan_payload);
        fb.const_f64(2, neg_zero);
    }));
    assert_eq!(r[0], 2.5f64.to_bits());
    assert_eq!(r[1], 0x7FF8_0000_0000_BEEF);
    assert_eq!(r[2], 0x8000_0000_0000_0000);
}

#[test]
fn mov_copies_all_64_bits() {
    let v: u64 = 0xDEAD_BEEF_0123_4567;
    let r = run_ok(&module(None, 1, 2, |fb| {
        fb.const_i64(1, v as i64);
        fb.mov(0, 1);
    }));
    assert_eq!(r, vec![v]);
}

fn select_with_cond(cond: u64) -> u64 {
    // r0 = select(cond = r2, a = r3, b = r4); a = 111, b = 222
    let r = run_ok(&module(None, 1, 5, |fb| {
        fb.const_i64(2, cond as i64);
        fb.const_i64(3, 111);
        fb.const_i64(4, 222);
        fb.select(0, 2, 3, 4);
    }));
    r[0]
}

#[test]
fn select_nonzero_cond_picks_a() {
    assert_eq!(select_with_cond(1), 111);
    assert_eq!(select_with_cond(u64::MAX), 111);
    assert_eq!(select_with_cond(0x8000_0000), 111);
    assert_eq!(select_with_cond(0xFFFF_FFFF_0000_0001), 111);
}

#[test]
fn select_zero_cond_picks_b() {
    assert_eq!(select_with_cond(0), 222);
}

#[test]
fn select_uses_only_low_32_bits_of_cond() {
    // 0x1_0000_0000 has zero low word, so it counts as false.
    assert_eq!(select_with_cond(0x1_0000_0000), 222);
    assert_eq!(select_with_cond(0xFFFF_FFFF_0000_0000), 222);
}

#[test]
fn nop_does_nothing_and_costs_one() {
    let m = module(None, 1, 1, |fb| {
        fb.nop();
        fb.const_i64(0, 7);
        fb.nop();
    });
    let mut inst = instantiate(&m, Limits::default());
    let f = inst.spawn("main", &[]).unwrap();
    let (exit, used) = inst.resume_units(f, &mut (), BUDGET);
    assert_eq!(exit, Exit::Done);
    assert_eq!(inst.results(f).unwrap(), &[7]);
    // nop, const, nop, ret
    assert_eq!(used, 4);
}

// ---------------------------------------------------------------------------
// loads
// ---------------------------------------------------------------------------

#[test]
fn i32_load_is_little_endian() {
    assert_eq!(load_case(Op::I32Load, &[4, 3, 2, 1], 0, 0), Ok(0x0102_0304));
}

#[test]
fn i32_load_high_bit_is_zero_extended() {
    assert_eq!(load_case(Op::I32Load, &[0, 0, 0, 0x80], 0, 0), Ok(0x8000_0000));
}

#[test]
fn i64_load_is_little_endian() {
    let bytes = 0x8877_6655_4433_2211u64.to_le_bytes();
    assert_eq!(load_case(Op::I64Load, &bytes, 0, 0), Ok(0x8877_6655_4433_2211));
}

#[test]
fn f32_load_reads_bits() {
    let bytes = 1.5f32.to_le_bytes();
    assert_eq!(load_case(Op::F32Load, &bytes, 0, 0), Ok(1.5f32.to_bits() as u64));
}

#[test]
fn f64_load_reads_bits() {
    let bytes = 2.5f64.to_le_bytes();
    assert_eq!(load_case(Op::F64Load, &bytes, 0, 0), Ok(2.5f64.to_bits()));
}

#[test]
fn i32_load8_s_sign_extends_into_32_then_64_bits() {
    assert_eq!(load_case(Op::I32Load8S, &[0x80], 0, 0), Ok(0xFFFF_FF80));
    assert_eq!(load_case(Op::I32Load8S, &[0x7F], 0, 0), Ok(0x7F));
}

#[test]
fn i32_load8_u_zero_extends() {
    assert_eq!(load_case(Op::I32Load8U, &[0x80], 0, 0), Ok(0x80));
}

#[test]
fn i32_load16_s_sign_extends_into_32_then_64_bits() {
    assert_eq!(load_case(Op::I32Load16S, &[0x00, 0x80], 0, 0), Ok(0xFFFF_8000));
    assert_eq!(load_case(Op::I32Load16S, &[0xFF, 0x7F], 0, 0), Ok(0x7FFF));
}

#[test]
fn i32_load16_u_zero_extends() {
    assert_eq!(load_case(Op::I32Load16U, &[0x00, 0x80], 0, 0), Ok(0x8000));
}

#[test]
fn i64_load8_s_sign_extends() {
    assert_eq!(load_case(Op::I64Load8S, &[0x80], 0, 0), Ok(0xFFFF_FFFF_FFFF_FF80));
    assert_eq!(load_case(Op::I64Load8S, &[0x7F], 0, 0), Ok(0x7F));
}

#[test]
fn i64_load8_u_zero_extends() {
    assert_eq!(load_case(Op::I64Load8U, &[0x80], 0, 0), Ok(0x80));
}

#[test]
fn i64_load16_s_sign_extends() {
    assert_eq!(load_case(Op::I64Load16S, &[0x00, 0x80], 0, 0), Ok(0xFFFF_FFFF_FFFF_8000));
    assert_eq!(load_case(Op::I64Load16S, &[0xFF, 0x7F], 0, 0), Ok(0x7FFF));
}

#[test]
fn i64_load16_u_zero_extends() {
    assert_eq!(load_case(Op::I64Load16U, &[0x00, 0x80], 0, 0), Ok(0x8000));
}

#[test]
fn i64_load32_s_sign_extends() {
    assert_eq!(
        load_case(Op::I64Load32S, &[0, 0, 0, 0x80], 0, 0),
        Ok(0xFFFF_FFFF_8000_0000)
    );
    assert_eq!(load_case(Op::I64Load32S, &[0xFF, 0xFF, 0xFF, 0x7F], 0, 0), Ok(0x7FFF_FFFF));
}

#[test]
fn i64_load32_u_zero_extends() {
    assert_eq!(load_case(Op::I64Load32U, &[0, 0, 0, 0x80], 0, 0), Ok(0x8000_0000));
}

#[test]
fn load_adds_offset_immediate_to_base() {
    // base 100 + off 4 reads address 104.
    let mut init = vec![0u8; 110];
    init[104..108].copy_from_slice(&0x1122_3344u32.to_le_bytes());
    init[100..104].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    assert_eq!(load_case(Op::I32Load, &init, 100, 4), Ok(0x1122_3344));
}

#[test]
fn load_unaligned_address_works() {
    let init = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    assert_eq!(load_case(Op::I32Load, &init, 1, 0), Ok(0x4433_2211));
    assert_eq!(load_case(Op::I64Load16U, &init, 3, 0), Ok(0x4433));
}

#[test]
fn load_of_narrow_widths_ignores_neighbouring_bytes() {
    let init = [0x80, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE];
    assert_eq!(load_case(Op::I32Load8U, &init, 0, 0), Ok(0x80));
    assert_eq!(load_case(Op::I64Load8S, &init, 0, 0), Ok(0xFFFF_FFFF_FFFF_FF80));
}

// Bounds: memory is exactly one page (65536 bytes).

#[test]
fn i32_load_bounds() {
    assert!(load_case(Op::I32Load, &[], 65532, 0).is_ok());
    assert_eq!(load_case(Op::I32Load, &[], 65533, 0), Err(Trap::OutOfBounds));
}

#[test]
fn i64_load_bounds() {
    assert!(load_case(Op::I64Load, &[], 65528, 0).is_ok());
    assert_eq!(load_case(Op::I64Load, &[], 65529, 0), Err(Trap::OutOfBounds));
}

#[test]
fn f32_and_f64_load_bounds() {
    assert!(load_case(Op::F32Load, &[], 65532, 0).is_ok());
    assert_eq!(load_case(Op::F32Load, &[], 65533, 0), Err(Trap::OutOfBounds));
    assert!(load_case(Op::F64Load, &[], 65528, 0).is_ok());
    assert_eq!(load_case(Op::F64Load, &[], 65529, 0), Err(Trap::OutOfBounds));
}

#[test]
fn narrow_load_bounds() {
    assert!(load_case(Op::I32Load8S, &[], 65535, 0).is_ok());
    assert_eq!(load_case(Op::I32Load8S, &[], 65536, 0), Err(Trap::OutOfBounds));
    assert!(load_case(Op::I32Load16U, &[], 65534, 0).is_ok());
    assert_eq!(load_case(Op::I32Load16U, &[], 65535, 0), Err(Trap::OutOfBounds));
    assert!(load_case(Op::I64Load32S, &[], 65532, 0).is_ok());
    assert_eq!(load_case(Op::I64Load32S, &[], 65533, 0), Err(Trap::OutOfBounds));
}

#[test]
fn load_overflowing_address_traps() {
    assert_eq!(load_case(Op::I32Load, &[], u64::MAX, 1), Err(Trap::OutOfBounds));
    assert_eq!(load_case(Op::I64Load8U, &[], u64::MAX, 0), Err(Trap::OutOfBounds));
}

#[test]
fn load_with_huge_offset_traps() {
    assert_eq!(load_case(Op::I32Load8U, &[], 0, u32::MAX), Err(Trap::OutOfBounds));
}

#[test]
fn zero_page_memory_traps_on_any_load() {
    let m = module(Some((0, None)), 1, 2, |fb| {
        fb.const_i64(1, 0);
        fb.load(Op::I32Load8U, 0, 1, 0);
    });
    assert_eq!(run_trap(&m), Trap::OutOfBounds);
    let no_memory = module(None, 1, 2, |fb| {
        fb.const_i64(1, 0);
        fb.load(Op::I64Load, 0, 1, 0);
    });
    assert_eq!(run_trap(&no_memory), Trap::OutOfBounds);
}

// ---------------------------------------------------------------------------
// stores
// ---------------------------------------------------------------------------

#[test]
fn i32_store_is_little_endian() {
    let (exit, mem) = store_case(Op::I32Store, 0x0102_0304, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..5], &[0x04, 0x03, 0x02, 0x01, PATTERN]);
}

#[test]
fn i32_store_writes_only_low_four_bytes_of_value() {
    let (exit, mem) = store_case(Op::I32Store, 0xFFFF_FFFF_1122_3344, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..6], &[0x44, 0x33, 0x22, 0x11, PATTERN, PATTERN]);
}

#[test]
fn i64_store_is_little_endian() {
    let (exit, mem) = store_case(Op::I64Store, 0x0102_0304_0506_0708, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..9], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, PATTERN]);
}

#[test]
fn f32_store_writes_bits() {
    let (exit, mem) = store_case(Op::F32Store, 1.5f32.to_bits() as u64, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..4], &1.5f32.to_le_bytes());
    assert_eq!(mem[4], PATTERN);
}

#[test]
fn f64_store_writes_bits() {
    let (exit, mem) = store_case(Op::F64Store, 2.5f64.to_bits(), 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..8], &2.5f64.to_le_bytes());
    assert_eq!(mem[8], PATTERN);
}

#[test]
fn i32_store8_writes_one_byte() {
    let (exit, mem) = store_case(Op::I32Store8, 0x1FF, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0xFF, PATTERN, PATTERN]);
}

#[test]
fn i32_store16_writes_two_bytes() {
    let (exit, mem) = store_case(Op::I32Store16, 0x1_2345, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0x45, 0x23, PATTERN]);
}

#[test]
fn i64_store8_writes_one_byte() {
    let (exit, mem) = store_case(Op::I64Store8, 0xABCD_EF00_0000_0042, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0x42, PATTERN, PATTERN]);
}

#[test]
fn i64_store16_writes_two_bytes() {
    let (exit, mem) = store_case(Op::I64Store16, 0xABCD_EF00_0000_1234, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0x34, 0x12, PATTERN]);
}

#[test]
fn i64_store32_writes_four_bytes() {
    let (exit, mem) = store_case(Op::I64Store32, 0xABCD_EF00_1122_3344, 0, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..5], &[0x44, 0x33, 0x22, 0x11, PATTERN]);
}

#[test]
fn store_leaves_neighbouring_bytes_untouched() {
    // An unaligned 16-bit store at address 2 must not touch bytes 1 or 4.
    let (exit, mem) = store_case(Op::I32Store16, 0xBEEF, 2, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..6], &[PATTERN, PATTERN, 0xEF, 0xBE, PATTERN, PATTERN]);
}

#[test]
fn store_adds_offset_immediate_to_base() {
    // base 8 + off 4 writes at 12.
    let (exit, mem) = store_case(Op::I32Store, 0x0403_0201, 8, 4);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[8..8 + 6], &[PATTERN, PATTERN, PATTERN, PATTERN, 0x01, 0x02]);
    assert_eq!(&mem[12..16], &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(mem[16], PATTERN);
}

#[test]
fn unaligned_store_works() {
    let (exit, mem) = store_case(Op::I64Store, 0x1122_3344_5566_7788, 3, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[3..11], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(mem[2], PATTERN);
    assert_eq!(mem[11], PATTERN);
}

#[test]
fn store_bounds_at_end_of_memory() {
    let (exit, mem) = store_case(Op::I32Store, 0x1122_3344, 65532, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[65532..], &[0x44, 0x33, 0x22, 0x11]);

    let (exit, mem) = store_case(Op::I32Store, 0x1122_3344, 65533, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory(), "trapping store must not modify memory");
}

#[test]
fn i64_store_bounds_at_end_of_memory() {
    let (exit, _) = store_case(Op::I64Store, 1, 65528, 0);
    assert_eq!(exit, Exit::Done);
    let (exit, mem) = store_case(Op::I64Store, 1, 65529, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());
}

#[test]
fn f32_and_f64_store_bounds() {
    let (exit, _) = store_case(Op::F32Store, 0, 65532, 0);
    assert_eq!(exit, Exit::Done);
    let (exit, _) = store_case(Op::F32Store, 0, 65533, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    let (exit, _) = store_case(Op::F64Store, 0, 65528, 0);
    assert_eq!(exit, Exit::Done);
    let (exit, mem) = store_case(Op::F64Store, 0, 65529, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());
}

#[test]
fn narrow_store_bounds() {
    let (exit, mem) = store_case(Op::I32Store8, 0xFF, 65535, 0);
    assert_eq!(exit, Exit::Done);
    assert_eq!(mem[65535], 0xFF);
    let (exit, mem) = store_case(Op::I32Store8, 0xFF, 65536, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());

    let (exit, _) = store_case(Op::I32Store16, 0xFFFF, 65534, 0);
    assert_eq!(exit, Exit::Done);
    let (exit, mem) = store_case(Op::I32Store16, 0xFFFF, 65535, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());

    let (exit, _) = store_case(Op::I64Store32, 0, 65532, 0);
    assert_eq!(exit, Exit::Done);
    let (exit, mem) = store_case(Op::I64Store32, 0, 65533, 0);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());
}

#[test]
fn store_overflowing_address_traps_without_writing() {
    let (exit, mem) = store_case(Op::I32Store, 0xFFFF_FFFF, u64::MAX, 1);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());

    let (exit, mem) = store_case(Op::I64Store8, 0xFF, 0, u32::MAX);
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, pattern_memory());
}

#[test]
fn zero_page_memory_traps_on_any_store() {
    let m = module(Some((0, None)), 0, 2, |fb| {
        fb.const_i64(0, 1);
        fb.const_i64(1, 0);
        fb.store(Op::I32Store8, 0, 1, 0);
    });
    assert_eq!(run_trap(&m), Trap::OutOfBounds);
}

// ---------------------------------------------------------------------------
// memory.size, memory.grow
// ---------------------------------------------------------------------------

#[test]
fn memory_size_returns_pages() {
    let m = module(Some((3, None)), 1, 1, |fb| fb.memory_size(0));
    assert_eq!(run_ok(&m), vec![3]);
}

#[test]
fn memory_grow_returns_old_size_and_zeroes_new_bytes() {
    let m = module(Some((1, None)), 2, 2, |fb| {
        fb.const_i64(1, 2);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    let (inst, exit, r) = run(&m);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r, vec![1, 3]);
    assert_eq!(inst.memory().size_pages(), 3);
    assert!(inst.memory().as_slice()[65536..].iter().all(|&b| b == 0));
}

#[test]
fn memory_grow_by_zero_returns_current_size() {
    let m = module(Some((2, None)), 2, 2, |fb| {
        fb.const_i64(1, 0);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    assert_eq!(run_ok(&m), vec![2, 2]);
}

#[test]
fn memory_grow_beyond_module_max_fails_and_keeps_size() {
    let m = module(Some((1, Some(2))), 2, 2, |fb| {
        fb.const_i64(1, 2);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    assert_eq!(run_ok(&m), vec![u64::MAX, 1]);
}

#[test]
fn memory_grow_within_module_max_succeeds() {
    let m = module(Some((1, Some(2))), 2, 2, |fb| {
        fb.const_i64(1, 1);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    assert_eq!(run_ok(&m), vec![1, 2]);
}

#[test]
fn memory_grow_huge_request_fails() {
    let m = module(Some((1, None)), 2, 2, |fb| {
        fb.const_i64(1, -1);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
        // a leader after the grow, where the overrun stops the fiber
        let after = fb.new_label();
        fb.jmp(after);
        fb.bind(after);
    });
    // The failed grow is charged for the requested size (saturated), so the fiber owes a
    // huge debt; the result is still correct and the instance usable.
    let mut inst = instantiate(&m, Limits::default());
    let f = inst.spawn("main", &[]).expect("spawn main");
    assert_eq!(inst.resume(f, &mut (), BUDGET), Exit::OutOfBudget);
    let mut exit = Exit::OutOfBudget;
    for _ in 0..4 {
        exit = inst.resume(f, &mut (), u64::MAX);
        if exit == Exit::Done {
            break;
        }
    }
    assert_eq!(exit, Exit::Done);
    assert_eq!(inst.results(f).unwrap(), &[u64::MAX, 1]);
}

#[test]
fn memory_grow_beyond_embedder_limit_fails() {
    let m = module(Some((1, None)), 2, 2, |fb| {
        fb.const_i64(1, 2);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    let limits = Limits { max_memory_pages: 2, ..Limits::default() };
    let mut inst = instantiate(&m, limits);
    let (exit, r) = exec(&mut inst);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r, vec![u64::MAX, 1]);
    assert_eq!(inst.memory().size_pages(), 1);
}

#[test]
fn memory_grow_up_to_embedder_limit_succeeds() {
    let m = module(Some((1, None)), 2, 2, |fb| {
        fb.const_i64(1, 1);
        fb.memory_grow(0, 1);
        fb.memory_size(1);
    });
    let limits = Limits { max_memory_pages: 2, ..Limits::default() };
    let mut inst = instantiate(&m, limits);
    let (exit, r) = exec(&mut inst);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r, vec![1, 2]);
}

// ---------------------------------------------------------------------------
// memory.copy, memory.fill
// ---------------------------------------------------------------------------

/// Body: copy(dst, src, len) with the values in r0, r1, r2.
fn copy_body(dst: u64, src: u64, len: u64) -> impl FnOnce(&mut FunctionBuilder) {
    move |fb: &mut FunctionBuilder| {
        fb.const_i64(0, dst as i64);
        fb.const_i64(1, src as i64);
        fb.const_i64(2, len as i64);
        fb.memory_copy(0, 1, 2);
    }
}

fn counting_init() -> Vec<u8> {
    (0u8..16).collect()
}

#[test]
fn memory_copy_non_overlapping() {
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(100, 0, 16));
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[100..116], init.as_slice());
    assert_eq!(&mem[..16], init.as_slice());
    assert_eq!(mem[116], BULK_BASE);
}

#[test]
fn memory_copy_overlapping_forward_has_memmove_semantics() {
    // dst 4 > src 0: destination gets the original source bytes.
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(4, 0, 8));
    assert_eq!(exit, Exit::Done);
    assert_eq!(
        &mem[..16],
        &[0, 1, 2, 3, 0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15]
    );
}

#[test]
fn memory_copy_overlapping_backward_has_memmove_semantics() {
    // dst 0 < src 4: copy runs forward.
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(0, 4, 8));
    assert_eq!(exit, Exit::Done);
    assert_eq!(
        &mem[..16],
        &[4, 5, 6, 7, 8, 9, 10, 11, 8, 9, 10, 11, 12, 13, 14, 15]
    );
}

#[test]
fn memory_copy_len_zero_at_exact_end_succeeds() {
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(65536, 65536, 0));
    assert_eq!(exit, Exit::Done);
    assert_eq!(mem, bulk_initial(&init));

    let (exit, _) = bulk_case(&init, copy_body(65536, 0, 0));
    assert_eq!(exit, Exit::Done);
    let (exit, _) = bulk_case(&init, copy_body(0, 65536, 0));
    assert_eq!(exit, Exit::Done);
}

#[test]
fn memory_copy_len_zero_past_end_traps() {
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(65537, 0, 0));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));
}

#[test]
fn memory_copy_out_of_bounds_traps_and_copies_nothing() {
    let init = counting_init();
    // destination range runs past the end
    let (exit, mem) = bulk_case(&init, copy_body(65530, 0, 8));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));

    // source range runs past the end
    let (exit, mem) = bulk_case(&init, copy_body(0, 65530, 8));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));

    // both ranges individually fine but the length is too large
    let (exit, mem) = bulk_case(&init, copy_body(0, 1, 65536));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));
}

#[test]
fn memory_copy_huge_length_traps() {
    let init = counting_init();
    let (exit, mem) = bulk_case(&init, copy_body(0, 0, u64::MAX));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));
}

/// Body: fill(dst, value, len) with the values in r0, r1, r2.
fn fill_body(dst: u64, value: u64, len: u64) -> impl FnOnce(&mut FunctionBuilder) {
    move |fb: &mut FunctionBuilder| {
        fb.const_i64(0, dst as i64);
        fb.const_i64(1, value as i64);
        fb.const_i64(2, len as i64);
        fb.memory_fill(0, 1, 2);
    }
}

#[test]
fn memory_fill_writes_value_in_range_only() {
    let init: Vec<u8> = Vec::new();
    let (exit, mem) = bulk_case(&init, fill_body(10, 0xAB, 5));
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[9..16], &[BULK_BASE, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, BULK_BASE]);
}

#[test]
fn memory_fill_uses_only_low_eight_bits() {
    let init: Vec<u8> = Vec::new();
    let (exit, mem) = bulk_case(&init, fill_body(0, 0x1_0000_00C3, 2));
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0xC3, 0xC3, BULK_BASE]);

    let (exit, mem) = bulk_case(&init, fill_body(0, 0xFFFF_FF00, 2));
    assert_eq!(exit, Exit::Done);
    assert_eq!(&mem[..3], &[0x00, 0x00, BULK_BASE]);
}

#[test]
fn memory_fill_len_zero_at_exact_end_succeeds() {
    let init: Vec<u8> = Vec::new();
    let (exit, mem) = bulk_case(&init, fill_body(65536, 0x11, 0));
    assert_eq!(exit, Exit::Done);
    assert_eq!(mem, bulk_initial(&init));
}

#[test]
fn memory_fill_out_of_bounds_traps_and_writes_nothing() {
    let init: Vec<u8> = Vec::new();
    let (exit, mem) = bulk_case(&init, fill_body(65535, 0x11, 2));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));

    let (exit, mem) = bulk_case(&init, fill_body(65537, 0x11, 0));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));

    let (exit, mem) = bulk_case(&init, fill_body(0, 0x11, u64::MAX));
    assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    assert_eq!(mem, bulk_initial(&init));
}

// ---------------------------------------------------------------------------
// globals
// ---------------------------------------------------------------------------

/// A module with four mutable globals (i32, i64, f32, f64) and their exports.
fn four_globals(mb: &mut ModuleBuilder) {
    let g_i32 = mb.global(ValType::I32, true, 0x1234);
    let g_i64 = mb.global(ValType::I64, true, u64::MAX);
    let g_f32 = mb.global(ValType::F32, true, 1.5f32.to_bits() as u64);
    let g_f64 = mb.global(ValType::F64, true, 2.5f64.to_bits());
    mb.export_global("g_i32", g_i32);
    mb.export_global("g_i64", g_i64);
    mb.export_global("g_f32", g_f32);
    mb.export_global("g_f64", g_f64);
}

#[test]
fn global_get_returns_initial_values() {
    let m = module_with(four_globals, 4, 4, |fb| {
        fb.global_get(0, 0);
        fb.global_get(1, 1);
        fb.global_get(2, 2);
        fb.global_get(3, 3);
    });
    let (inst, exit, r) = run(&m);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r, vec![0x1234, u64::MAX, 1.5f32.to_bits() as u64, 2.5f64.to_bits()]);
    assert_eq!(inst.global(0), Some(0x1234));
    assert_eq!(inst.export_global("g_i64"), Some(u64::MAX));
    assert_eq!(inst.export_global("g_f32"), Some(1.5f32.to_bits() as u64));
    assert_eq!(inst.export_global("g_f64"), Some(2.5f64.to_bits()));
    assert_eq!(inst.export_global("missing"), None);
}

#[test]
fn global_set_is_visible_to_later_gets_and_to_host() {
    let m = module_with(four_globals, 4, 4, |fb| {
        fb.const_i64(0, 99);
        fb.global_set(0, 0);
        fb.global_get(1, 0);
        fb.const_f32(2, 2.25);
        fb.global_set(2, 2);
        fb.global_get(3, 2);
    });
    let (inst, exit, r) = run(&m);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r[1], 99);
    assert_eq!(r[3], 2.25f32.to_bits() as u64);
    assert_eq!(inst.global(0), Some(99));
    assert_eq!(inst.global(2), Some(2.25f32.to_bits() as u64));
    assert_eq!(inst.export_global("g_f32"), Some(2.25f32.to_bits() as u64));
}

#[test]
fn global_set_f64_and_i64_round_trip() {
    let m = module_with(four_globals, 2, 4, |fb| {
        fb.const_f64(0, -0.0);
        fb.global_set(3, 0);
        fb.global_get(1, 3);
    });
    let (inst, exit, r) = run(&m);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r[0], (-0.0f64).to_bits());
    assert_eq!(inst.export_global("g_f64"), Some((-0.0f64).to_bits()));
}

#[test]
fn host_set_global_is_visible_to_guest() {
    let m = module_with(four_globals, 1, 2, |fb| fb.global_get(0, 1));
    let mut inst = instantiate(&m, Limits::default());
    assert!(inst.set_global(1, 42));
    let (exit, r) = exec(&mut inst);
    assert_eq!(exit, Exit::Done);
    assert_eq!(r, vec![42]);
    assert_eq!(inst.global(1), Some(42));
}

#[test]
fn global_set_on_immutable_global_fails_verification() {
    let mut mb = ModuleBuilder::new();
    let g = mb.global(ValType::I32, false, 1);
    let mut fb = FunctionBuilder::new(2);
    fb.const_i64(0, 5);
    fb.global_set(g, 0);
    fb.ret(0);
    let f = mb.add_func(FuncType::new([], []), fb);
    mb.export_func("main", f);
    assert!(mb.finish().is_err());
}

// ---------------------------------------------------------------------------
// data segments
// ---------------------------------------------------------------------------

#[test]
fn data_segment_bytes_appear_at_offset_on_instantiation() {
    let m = module_with(
        |mb| {
            mb.memory(1, None);
            mb.data(16, b"hello");
        },
        0,
        1,
        |_| {},
    );
    let inst = instantiate(&m, Limits::default());
    assert_eq!(&inst.memory().as_slice()[16..21], b"hello");
    assert_eq!(inst.memory().as_slice()[15], 0);
    assert_eq!(inst.memory().as_slice()[21], 0);
}

#[test]
fn data_segment_is_readable_by_loads() {
    let m = module_with(
        |mb| {
            mb.memory(1, None);
            mb.data(16, &0x1122_3344u32.to_le_bytes());
        },
        1,
        3,
        |fb| {
            fb.const_i64(1, 16);
            fb.load(Op::I32Load, 0, 1, 0);
        },
    );
    assert_eq!(run_ok(&m), vec![0x1122_3344]);
}

#[test]
fn data_segment_ending_exactly_at_end_of_memory_is_fine() {
    let m = module_with(
        |mb| {
            mb.memory(1, None);
            mb.data(65536 - 4, &[1, 2, 3, 4]);
        },
        0,
        1,
        |_| {},
    );
    let inst = instantiate(&m, Limits::default());
    assert_eq!(&inst.memory().as_slice()[65532..], &[1, 2, 3, 4]);
}

#[test]
fn data_segment_that_does_not_fit_is_rejected() {
    let mut mb = ModuleBuilder::new();
    mb.memory(1, None);
    mb.data(65536 - 3, &[1, 2, 3, 4]);
    let mut fb = FunctionBuilder::new(1);
    fb.ret(0);
    let f = mb.add_func(FuncType::new([], []), fb);
    mb.export_func("main", f);
    match mb.finish() {
        Err(_) => {}
        Ok(m) => assert!(
            Instance::new(&m, &Imports::<()>::new(), Limits::default()).is_err(),
            "segment past the end of memory must be rejected"
        ),
    }
}

// ---------------------------------------------------------------------------
// coverage
// ---------------------------------------------------------------------------

/// Every op this file is responsible for, by its SPEC.md mnemonic.
const REQUIRED: &[&str] = &[
    "const.i32", "const.i64", "const.f32", "const.f64", "mov", "select", "nop",
    "i32.load", "i64.load", "f32.load", "f64.load",
    "i32.load8_s", "i32.load8_u", "i32.load16_s", "i32.load16_u",
    "i64.load8_s", "i64.load8_u", "i64.load16_s", "i64.load16_u",
    "i64.load32_s", "i64.load32_u",
    "i32.store", "i64.store", "f32.store", "f64.store",
    "i32.store8", "i32.store16", "i64.store8", "i64.store16", "i64.store32",
    "memory.size", "memory.grow", "memory.copy", "memory.fill",
    "global.get", "global.set",
];

/// The builder call or `Op` variant through which a test exercises each required op.
/// Loads and stores are reached through `load_case` / `store_case` with an `Op::` argument;
/// the rest are emitted by a `FunctionBuilder` method.
fn exercising_call(name: &str) -> &'static str {
    match name {
        "const.i32" => "fb.const_i32",
        "const.i64" => "fb.const_i64",
        "const.f32" => "fb.const_f32",
        "const.f64" => "fb.const_f64",
        "mov" => "fb.mov",
        "select" => "fb.select",
        "nop" => "fb.nop",
        "memory.size" => "fb.memory_size",
        "memory.grow" => "fb.memory_grow",
        "memory.copy" => "fb.memory_copy",
        "memory.fill" => "fb.memory_fill",
        "global.get" => "fb.global_get",
        "global.set" => "fb.global_set",
        _ => "",
    }
}

#[test]
fn every_required_op_is_covered() {
    // This file's own source: each op must be named either as `Op::<Variant>` (loads and
    // stores) or through the builder call that emits it.
    let source = include_str!("memory_ops.rs");
    for name in REQUIRED {
        let op = Op::ALL
            .iter()
            .copied()
            .find(|op| op.name() == *name)
            .unwrap_or_else(|| panic!("no op named {name} in Op::ALL"));
        let call = exercising_call(name);
        let referenced = if call.is_empty() {
            source.contains(&format!("Op::{op:?}"))
        } else {
            source.contains(call)
        };
        assert!(referenced, "op {name} is not exercised by memory_ops.rs");
    }
}
