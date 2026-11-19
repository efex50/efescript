//! Constant expressions in `.efa` (efa-syntax 18): where they are accepted, how they are
//! evaluated and narrowed, the names they can read, and their errors.

use efa::asm::ast::Operand;
use efa::asm::{assemble, assemble_program, assemble_with, parse, AsmOptions, Diagnostic, MemSource};
use efevm::api::{ApiIndex, FieldDesc, HostRegistry, Prim, StructDesc};
use efevm::{Inst, Module, Op};

fn ok(src: &str) -> Module {
    match assemble(src, "t.efa") {
        Ok(m) => m,
        Err(e) => panic!("assembly failed:\n{e}\n--- source ---\n{src}"),
    }
}

fn errs(src: &str) -> Vec<Diagnostic> {
    match assemble(src, "t.efa") {
        Ok(_) => panic!("expected errors for:\n{src}"),
        Err(e) => e.0,
    }
}

/// The only diagnostic: (message, line, column, len).
fn one(src: &str) -> (String, u32, u32, u32) {
    let v = errs(src);
    assert_eq!(v.len(), 1, "expected one diagnostic, got:\n{}\n--- source ---\n{src}", efa::asm::Diagnostics(v.clone()));
    (v[0].message.clone(), v[0].line, v[0].column, v[0].len)
}

fn code(m: &Module) -> Vec<Inst> {
    m.data().functions[0].code.clone()
}

/// Wraps `body` (instruction lines) in `export fn f()`.
fn func(body: &str) -> String {
    format!("export fn f() {{\n    reg d, p, q\n{body}\n    ret\n}}\n")
}

/// `a` and `b` assemble to the same code, memory, globals and data.
fn same(a: &str, b: &str) {
    let (ma, mb) = (ok(a), ok(b));
    let (da, db) = (ma.data(), mb.data());
    assert_eq!(da.functions.iter().map(|f| &f.code).collect::<Vec<_>>(), db.functions.iter().map(|f| &f.code).collect::<Vec<_>>(), "code differs:\n{a}\n--\n{b}");
    assert_eq!(da.memory, db.memory, "memory differs:\n{a}\n--\n{b}");
    assert_eq!(da.globals, db.globals, "globals differs:\n{a}\n--\n{b}");
    assert_eq!(da.data, db.data, "data differs:\n{a}\n--\n{b}");
}

fn first_op(m: &Module, op: Op) -> Inst {
    *code(m).iter().find(|i| i.op == op).unwrap_or_else(|| panic!("no {op:?} in {:?}", code(m)))
}

// ---- positions ---------------------------------------------------------------------------

#[test]
fn const_decl_takes_an_expression() {
    same(
        &(func("    const.i64 d, K") + "const K = 2 + 3 * 4\n"),
        &(func("    const.i64 d, 14")),
    );
    same(
        &(func("    const.i64 d, K") + "pub const K = (2 + 3) * 4\n"),
        &(func("    const.i64 d, 20")),
    );
}

#[test]
fn const_immediate_takes_an_expression() {
    same(&func("    const.i32 d, 3 * 7"), &func("    const.i32 d, 21"));
    same(&func("    const.i64 d, 1 << 40"), &func("    const.i64 d, 1099511627776"));
    same(&func("    const.f32 d, 1.5 * 2.0"), &func("    const.f32 d, 3.0"));
    same(&func("    const.f64 d, 1.0 / 4.0"), &func("    const.f64 d, 0.25"));
}

#[test]
fn load_store_offsets_take_expressions() {
    same(&func("    f32.load d, p, 4 * 2"), &func("    f32.load d, p, 8"));
    same(&func("    f32.store p, d, (1 + 1) << 2"), &func("    f32.store p, d, 8"));
    same(
        &(func("    i64.load d, p, K + 4") + "const K = 4\n"),
        &func("    i64.load d, p, 8"),
    );
    // a comma ends the expression, so the operand list is unchanged
    let m = ok(&func("    i32.load d, p, 2 * 2"));
    assert_eq!(first_op(&m, Op::I32Load).c, 4);
}

#[test]
fn data_sizes_and_values_take_expressions() {
    same("buf resb 4 * 16\n", "buf resb 64\n");
    same("t db 1 + 1, 'a' + 1, 0x10 >> 2, \"s\", 255 & 15\n", "t db 2, 98, 4, \"s\", 15\n");
    same("t dw 1 << 10, -1 * 3\n", "t dw 1024, -3\n");
    same("t dd 2 * 3, 1.5 * 2.0\n", "t dd 6, 3.0\n");
    same("t dq 1 << 40, 1.0 / 8.0\n", "t dq 1099511627776, 0.125\n");
}

#[test]
fn memory_takes_expressions() {
    same("memory 1 + 1 2 * 4\n", "memory 2 8\n");
    same("memory (1 + 1) (2 * 4)\n", "memory 2 8\n");
    same("memory 1 + 1\n", "memory 2\n");
    same("memory 1 2\n", "memory (1) (2)\n");
    same("memory N N * 2\nconst N = 3\n", "memory 3 6\n");
}

#[test]
fn scratch_takes_an_expression() {
    same("scratch 16 * 4\n", "scratch 64\n");
}

#[test]
fn global_init_takes_an_expression() {
    same("global g: i32 = 6 * 7\n", "global g: i32 = 42\n");
    same("global mut g: f32 = 1.0 / 4.0\n", "global mut g: f32 = 0.25\n");
    same("global g: i64 = 1 << 40\n", "global g: i64 = 1099511627776\n");
    same("global g: f64 = K * 2.0\nconst K = 1.25\n", "global g: f64 = 2.5\n");
}

#[test]
fn struct_array_counts_take_expressions() {
    same(
        &(func("    const.i64 d, S.b + S.size") + "struct S {\n    a: u8[2 * 4]\n    b: u32\n}\n"),
        &(func("    const.i64 d, 20")),
    );
    same(
        &(func("    const.i64 d, S.b") + "const N = 3\nstruct S {\n    a: u8[N + 1]\n    b: u32\n}\n"),
        &(func("    const.i64 d, 4")),
    );
}

// ---- evaluation --------------------------------------------------------------------------

fn val_i64(expr: &str) -> i64 {
    let m = ok(&func(&format!("    const.i64 d, {expr}")));
    let i = first_op(&m, Op::ConstI64);
    ((u64::from(i.c) << 32) | u64::from(i.b)) as i64
}

#[test]
fn precedence_and_associativity() {
    assert_eq!(val_i64("2 + 3 * 4"), 14);
    assert_eq!(val_i64("(2 + 3) * 4"), 20);
    assert_eq!(val_i64("10 - 4 - 3"), 3);
    assert_eq!(val_i64("100 / 5 / 2"), 10);
    assert_eq!(val_i64("7 % 4 * 2"), 6);
    assert_eq!(val_i64("1 << 2 + 1"), 8);
    assert_eq!(val_i64("256 >> 2 + 2"), 16);
    assert_eq!(val_i64("1 | 6 & 3 ^ 1"), 3);
    assert_eq!(val_i64("6 & 3 | 8"), 10);
    assert_eq!(val_i64("5 ^ 1 | 8"), 12);
    assert_eq!(val_i64("-3 * 2"), -6);
    assert_eq!(val_i64("~0"), -1);
    assert_eq!(val_i64("~5 & 0xff"), 250);
    assert_eq!(val_i64("- -5"), 5);
    assert_eq!(val_i64("-(2 + 3)"), -5);
    assert_eq!(val_i64("7 / 2"), 3);
    assert_eq!(val_i64("'a' + 1"), 98);
    assert_eq!(val_i64("0b101 + 0o7 + 0xf + 1_0"), 37);
}

#[test]
fn minus_before_a_literal_is_a_subtraction_after_an_operand() {
    assert_eq!(val_i64("5-3"), 2);
    assert_eq!(val_i64("5 -3"), 2);
    assert_eq!(val_i64("5 - 3"), 2);
    assert_eq!(val_i64("5 - -3"), 8);
    assert_eq!(val_i64("(5)-3"), 2);
}

#[test]
fn minus_with_a_name() {
    let m = ok(&(func("    const.i64 d, K-1\n    const.i64 d, K -1\n    const.i64 d, -K\n    const.i64 d, K-1 * 2 +K -1") + "const K = 7\n"));
    let consts: Vec<i64> = code(&m)
        .iter()
        .filter(|i| i.op == Op::ConstI64)
        .map(|i| ((u64::from(i.c) << 32) | u64::from(i.b)) as i64)
        .collect();
    assert_eq!(consts, vec![6, 6, -7, 7 - 2 + 7 - 1]);
}

#[test]
fn float_arithmetic_is_folded_wide_then_narrowed() {
    // exact in f64, rounded once to f32
    same(&func("    const.f32 d, 0.1 + 0.2"), &func("    const.f32 d, 0.30000000000000004"));
    same(&func("    const.f32 d, 1 + 0.5"), &func("    const.f32 d, 1.5"));
    same(&func("    const.f64 d, 3 * 0.5"), &func("    const.f64 d, 1.5"));
    // an integer expression in a float slot is converted
    same(&func("    const.f32 d, 1 << 3"), &func("    const.f32 d, 8.0"));
    same(&func("    const.f64 d, 2 * 3"), &func("    const.f64 d, 6.0"));
    same(&func("    const.f32 d, -inf + 1"), &func("    const.f32 d, -inf"));
}

#[test]
fn a_single_float_literal_is_untouched() {
    // not folded through f64: parsed directly as f32
    let a = ok(&func("    const.f32 d, 16777217.0"));
    let b = ok(&func("    const.f32 d, 16777216.0"));
    assert_eq!(code(&a), code(&b));
    same(&func("    const.f32 d, nan:0x7fc00001"), &func("    const.f32 d, nan:0x7fc00001"));
}

#[test]
fn integers_are_narrowed_at_the_use_site() {
    same(&func("    const.i32 d, 0xffff_ffff + 0"), &func("    const.i32 d, -1"));
    same(&func("    const.i32 d, 3000000000 - 0"), &func("    const.i32 d, 3000000000"));
    same(&func("    const.i64 d, 0 - 1"), &func("    const.i64 d, -1"));
}

// ---- names -------------------------------------------------------------------------------

#[test]
fn consts_forward_references_and_chains() {
    same(
        &(func("    const.i64 d, C") + "const C = B * 2\nconst B = A + 1\nconst A = 4\n"),
        &func("    const.i64 d, 10"),
    );
    same(
        &(func("    const.f32 d, R2") + "const R = 5.0\nconst R2 = (R + 6.0) * (R + 6.0)\n"),
        &func("    const.f32 d, 121.0"),
    );
}

#[test]
fn a_const_may_use_data_lengths_and_struct_constants() {
    same(
        &(func("    const.i64 d, N") + "msg db \"hello\"\nconst N = msg.len * 2 + S.size\nstruct S {\n    a: u32\n    b: u8\n}\n"),
        &(func("    const.i64 d, 18") + "msg db \"hello\"\n"),
    );
    // used before the declarations they read
    same(
        &("const N = msg.len + 1\nmsg resb M\nconst M = S.b * 3 + 4\nstruct S {\n    a: u32\n    b: u32\n}\n".to_string()
            + &func("    const.i64 d, N")),
        &("msg resb 16\n".to_string() + &func("    const.i64 d, 17")),
    );
}

#[test]
fn data_symbols_in_immediates_and_late_consts() {
    // an address is known after the layout: fine as an immediate or in a late const
    let a = ok(&("a db 1\nb db 2\nconst E = b + 3\n".to_string() + &func("    const.i64 d, E\n    const.i64 q, b + 3")));
    let consts: Vec<(u32, u32)> =
        code(&a).iter().filter(|i| i.op == Op::ConstI64).map(|i| (i.b, i.c)).collect();
    assert_eq!(consts.len(), 2);
    assert_eq!(consts[0], consts[1]);
    // and in a global initial value and in data
    let g = ok("a db 1\nb db 2\nglobal g: i64 = b + 3\nt dq a + b\n");
    let addr_b = g.data().data.iter().find(|x| x.bytes == [2]).map(|x| x.offset).unwrap();
    assert_eq!(g.data().globals[0].init, addr_b + 3);
    let addr_a = g.data().data.iter().find(|x| x.bytes == [1]).map(|x| x.offset).unwrap();
    let t = g.data().data.iter().find(|x| x.bytes.len() == 8).unwrap();
    assert_eq!(u64::from_le_bytes(t.bytes[..8].try_into().unwrap()), addr_a + addr_b);
}

#[test]
fn late_names_cannot_be_used_where_the_layout_needs_a_number() {
    let (m, l, c, _) = one("a db 1\nbuf resb a + 1\n");
    assert!(m.contains("`a` is a data address, which is only known when the program is linked"), "{m}");
    assert_eq!((l, c), (2, 10));
    let (m, ..) = one("a db 1\nconst E = a + 1\nmemory E\n");
    assert!(m.contains("`E` depends on `a` (a data address)"), "{m}");
    let (m, l, c, _) = one("a db 1\npub const E = a + 1\n");
    assert!(m.contains("`pub const E` must be known when the file is assembled"), "{m}");
    assert_eq!((l, c), (2, 11));
}

#[test]
fn api_constants_in_expressions() {
    let mut r: HostRegistry<()> = HostRegistry::new();
    let mut t = StructDesc::new("Transform", 20, 4)
        .field(FieldDesc::new("pos", 0, Prim::F32, 2))
        .field(FieldDesc::new("rot", 8, Prim::F32, 1))
        .field(FieldDesc::new("scale", 12, Prim::F32, 2));
    t.owner = "engine".into();
    t.component = Some(77);
    r.register_type(t).unwrap();
    let ix: ApiIndex = r.index();
    let o = AsmOptions { api: Some(&ix), ..Default::default() };
    let src = func("    const.i64 d, Transform.size * 2 + Transform.rot\n    f32.load d, p, Transform.scale - Transform.rot");
    let m = assemble_with(&src, "t.efa", &o).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(first_op(&m, Op::ConstI64).b, 48);
    assert_eq!(first_op(&m, Op::F32Load).c, 4);
    let m = assemble_with("const N = Transform.id + 1\nglobal g: i32 = N\n", "t.efa", &o).unwrap();
    assert_eq!(m.data().globals[0].init, 78);
}

// ---- errors ------------------------------------------------------------------------------

#[test]
fn cycles_are_reported_once() {
    let (m, l, c, _) = one("const A = B + 1\nconst B = A * 2\n");
    assert!(m.contains("is defined in terms of itself"), "{m}");
    // the use that closes the cycle: `A` in the second line
    assert_eq!((l, c), (2, 11));
    let v = errs("const A = A + 1\n");
    assert_eq!(v.len(), 1, "{v:?}");
    assert_eq!((v[0].line, v[0].column, v[0].len), (1, 11, 1));
    assert_eq!(v[0].note.as_deref(), Some("cycle: A -> A"));
    let v = errs("const A = S.size\nstruct S {\n    a: u8[A]\n}\n");
    assert_eq!(v.len(), 1, "{v:?}");
    assert!(v[0].message.contains("defined in terms of itself"));
    let v = errs("const A = x.len\nx resb A\n");
    assert_eq!(v.len(), 1, "{v:?}");
}

#[test]
fn arithmetic_errors_point_at_the_operation() {
    let (m, l, c, n) = one("const A = 1 / 0\n");
    assert!(m.contains("division by zero"), "{m}");
    assert_eq!((l, c, n), (1, 11, 5));
    let (m, l, c, n) = one("const A = 7 % (2 - 2)\n");
    assert!(m.contains("division by zero"), "{m}");
    assert_eq!((l, c, n), (1, 11, 11));
    let (m, l, c, n) = one("const A = 1 << 200\n");
    assert!(m.contains("overflow"), "{m}");
    assert_eq!((l, c, n), (1, 11, 8));
    let (m, l, c, n) = one("const A = 170141183460469231731687303715884105727 + 1\n");
    assert!(m.contains("overflow"), "{m}");
    assert_eq!((l, c), (1, 11));
    assert!(n > 10);
    let (m, _, c, n) = one("const A = 1.5 << 2\n");
    assert!(m.contains("constant expression"), "{m}");
    assert_eq!((c, n), (11, 8));
}

#[test]
fn out_of_range_for_the_slot() {
    let (m, l, c, n) = one(&func("    const.i32 d, 1 << 40"));
    assert!(m.contains("does not fit in i32"), "{m}");
    assert_eq!((l, c, n), (3, 18, 7));
    let (m, ..) = one("global g: i32 = 1 << 32 << 1\n");
    assert!(m.contains("does not fit in i32"), "{m}");
    let (m, ..) = one("t db 100 + 200 - 50 + 100\n");
    assert!(m.contains("does not fit in a byte"), "{m}");
    let (m, l, c, n) = one("memory 1 - 2\n");
    assert!(m.contains("memory size -1"), "{m}");
    assert_eq!((l, c, n), (1, 8, 5));
    let (m, ..) = one("x resb 0 - 1\n");
    assert!(m.contains("`resb` size -1 is out of range"), "{m}");
    let (m, l, c, n) = one(&func("    i32.load d, p, 0 - 4"));
    assert!(m.contains("offset -4 does not fit in u32"), "{m}");
    assert_eq!((l, c, n), (3, 20, 5));
}

#[test]
fn a_float_in_an_integer_slot_is_an_error() {
    let (m, l, c, n) = one(&func("    const.i32 d, 1.5 * 2.0"));
    assert!(m.contains("float literal in an integer position"), "{m}");
    assert_eq!((l, c, n), (3, 18, 9));
    let (m, ..) = one("global g: i64 = 1 + 0.5\n");
    assert!(m.contains("float literal in an integer position"), "{m}");
    let (m, l, c, _) = one("buf resb 1.0 * 4\n");
    assert!(m.contains("expected an integer, found a float"), "{m}");
    assert_eq!((l, c), (1, 10));
    let (m, ..) = one("t db 1.0 + 1.0\n");
    assert!(m.contains("float literal in an integer position"), "{m}");
    let (m, ..) = one("struct S {\n    a: u8[2.0 * 2]\n}\n");
    assert!(m.contains("expected an integer, found a float"), "{m}");
}

#[test]
fn unknown_names_point_at_the_name() {
    let (m, l, c, n) = one("const A = 1 + Nope * 2\n");
    assert!(m.contains("unknown constant `Nope`"), "{m}");
    assert_eq!((l, c, n), (1, 15, 4));
    let (m, l, c, n) = one(&func("    const.i64 d, 4 * Nope"));
    assert!(m.contains("unknown constant `Nope`"), "{m}");
    assert_eq!((l, c, n), (3, 22, 4));
    // a register is not a constant
    let (m, ..) = one(&func("    const.i64 d, p + 1"));
    assert!(m.contains("unknown"), "{m}");
}

#[test]
fn a_failed_const_does_not_cascade() {
    let v = errs("const A = 1 / 0\nconst B = A + 1\nmemory B\n");
    assert_eq!(v.len(), 1, "{v:?}");
}

#[test]
fn syntax_errors() {
    let (m, l, c, _) = one("const A = 1 +\n");
    assert!(m.contains("incomplete"), "{m}");
    assert_eq!((l, c), (1, 13));
    let (m, l, c, _) = one("const A = (1 + 2\n");
    assert!(m.contains("unclosed `(`"), "{m}");
    assert_eq!((l, c), (1, 11));
    let (m, ..) = one("const A = 1 + 2)\n");
    assert!(m.contains("expected end of line"), "{m}");
    let (m, ..) = one("const A = 1 * * 2\n");
    assert!(m.contains("invalid constant expression"), "{m}");
    let (m, ..) = one("const A = nan:0x7fc00001 + 1\n");
    assert!(m.contains("nan:0x"), "{m}");
    let (m, ..) = one("const A = 2 3\n");
    assert!(m.contains("expected end of line"), "{m}");
}

#[test]
fn expressions_stay_out_of_instruction_sources() {
    // `(` there still starts a tuple, and a source is a register or one literal
    let m = errs(&func("    i64.add d, p, 1 + 2"));
    assert!(!m.is_empty());
    ok("fn g(a: i64, b: i64) -> (i64, i64) {\n    ret (a, b)\n}\n");
}

// ---- operand shapes ----------------------------------------------------------------------

fn const_value(src: &str) -> Operand {
    let m = parse(src, "t.efa").unwrap_or_else(|e| panic!("{e}"));
    match &m.items[0] {
        efa::asm::ast::Item::Const(c) => c.value.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn single_literals_keep_their_operand_shape() {
    assert!(matches!(const_value("const K = 5\n"), Operand::Int(5, _)));
    assert!(matches!(const_value("const K = -5\n"), Operand::Int(-5, _)));
    assert!(matches!(const_value("const K = 'a'\n"), Operand::Char('a', _)));
    assert!(matches!(const_value("const K = other\n"), Operand::Name(_)));
    assert!(matches!(const_value("const K = a.b\n"), Operand::Name(_)));
    match const_value("const K = nan:0x7fc00001\n") {
        Operand::Float(t, _) => assert_eq!(t, "nan:0x7fc00001"),
        o => panic!("{o:?}"),
    }
    match const_value("const K = -1.50\n") {
        Operand::Float(t, _) => assert_eq!(t, "-1.50"),
        o => panic!("{o:?}"),
    }
    assert!(matches!(const_value("const K = 5 - 1\n"), Operand::Expr(_)));
    assert!(matches!(const_value("const K = (5)\n"), Operand::Expr(_)));
    assert!(matches!(const_value("const K = -x\n"), Operand::Expr(_)));
    match const_value("const K = a + 1\n") {
        Operand::Expr(e) => {
            assert_eq!((e.span.line, e.span.column, e.span.len), (1, 11, 5));
            assert_eq!(e.spans.len(), 3);
        }
        o => panic!("{o:?}"),
    }
}

// ---- other files -------------------------------------------------------------------------

fn program(files: &[(&str, &str)]) -> Result<Module, Vec<Diagnostic>> {
    let src = files.iter().fold(MemSource::new(), |s, (p, t)| s.with(p, t));
    assemble_program(files[0].0, &src, &AsmOptions::default()).map_err(|d| d.0)
}

#[test]
fn pub_consts_of_other_files_in_expressions() {
    let a = "use \"b.efa\"\nconst X = b.A * 2 + b.B.size\nmemory b.A\nglobal g: i64 = X\nexport fn f() {\n    reg d\n    const.i64 d, X + b.B.size\n    ret\n}\n";
    let b = "pub const A = 10 + 1\nstruct Hidden {\n    a: u8\n}\npub struct B {\n    a: u32\n    b: u32\n}\n";
    let m = program(&[("a.efa", a), ("b.efa", b)]).unwrap_or_else(|e| panic!("{}", efa::asm::Diagnostics(e)));
    assert_eq!(m.data().globals[0].init, 30);
    assert_eq!(m.data().memory.min_pages, 11);
    assert_eq!(first_op(&m, Op::ConstI64).b, 38);
}

#[test]
fn a_chain_of_files_declares_dependencies_first() {
    // the root uses b, b uses c; each file's pub const reads the next one's
    let a = "use \"b.efa\"\nglobal g: i64 = b.TWO * 100\n";
    let b = "use \"c.efa\"\npub const TWO = c.ONE + c.ONE\n";
    let c = "pub const ONE = 1\n";
    let m = program(&[("a.efa", a), ("b.efa", b), ("c.efa", c)]).unwrap();
    assert_eq!(m.data().globals[0].init, 200);
}

#[test]
fn another_files_data_address_is_late() {
    let a = "use \"b.efa\"\nglobal g: i64 = b.buf + 8\nconst L = b.buf.len * 2\nglobal h: i64 = L\n";
    let b = "pub buf resb 16\n";
    let m = program(&[("a.efa", a), ("b.efa", b)]).unwrap_or_else(|e| panic!("{}", efa::asm::Diagnostics(e)));
    assert_eq!(m.data().globals[1].init, 32);
    assert!(m.data().globals[0].init >= 8);
    let a = "use \"b.efa\"\nmemory b.buf\n";
    let e = program(&[("a.efa", a), ("b.efa", b)]).unwrap_err();
    assert!(e[0].message.contains("data address"), "{e:?}");
}

#[test]
fn private_constants_of_other_files_still_error() {
    let a = "use \"b.efa\"\nconst X = b.A + 1\n";
    let b = "const A = 1\n";
    let e = program(&[("a.efa", a), ("b.efa", b)]).unwrap_err();
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].message.contains("private"), "{}", e[0].message);
    assert_eq!((e[0].line, e[0].column), (2, 11));
}

#[test]
fn hostile_expressions_do_not_overflow_the_stack() {
    let long = vec!["1"; 5000].join(" + ");
    let (m, ..) = one(&format!("const A = {long}\n"));
    assert!(m.contains("too long"), "{m}");
    let deep = format!("const A = {}1{}\n", "(".repeat(100_000), ")".repeat(100_000));
    assert!(!errs(&deep).is_empty());
    let neg = format!("const A = {}1\n", "-".repeat(100_000));
    assert!(!errs(&neg).is_empty());
    let ok_long = vec!["1"; 450].join(" + ");
    ok(&format!("const A = {ok_long}\n"));
}
