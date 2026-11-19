//! One rejecting test per load-time verification rule (SPEC.md section 8), plus the
//! link-time import rule and the builder `finish()` path. Black-box, public API only.

mod common;
use common::reg_host;
use efevm::*;

fn ty(p: usize, r: usize) -> FuncType {
    FuncType::new(vec![ValType::I32; p], vec![ValType::I32; r])
}

/// One function (index 0): type (i32)->(i32), nregs 4, `ret 0`. Valid.
fn base() -> ModuleData {
    ModuleData {
        types: vec![ty(1, 1)],
        functions: vec![Function {
            type_idx: 0,
            nregs: 4,
            code: vec![Inst::new(Op::Ret, 0, 0, 0)],
            br_tables: vec![],
        }],
        ..Default::default()
    }
}

/// `base()` with the single function's code replaced.
fn with_code(code: Vec<Inst>) -> ModuleData {
    let mut m = base();
    m.functions[0].code = code;
    m
}

fn ret() -> Inst {
    Inst::new(Op::Ret, 0, 0, 0)
}

/// Verification error of a module that must be rejected; panics if accepted.
fn reject(m: ModuleData) -> LoadError {
    match Module::new(m) {
        Ok(_) => panic!("module was accepted, expected a verification error"),
        Err(e) => e,
    }
}

#[test]
fn base_module_is_accepted() {
    assert!(Module::new(base()).is_ok());
    assert!(Module::new(with_code(vec![ret()])).is_ok());
}

#[test]
fn type_index_function() {
    let mut m = base();
    m.functions[0].type_idx = 5;
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::TypeIndex, .. }), "{e:?}");
}

#[test]
fn type_index_import() {
    let mut m = base();
    m.imports.push(Import { namespace: "env".into(), name: "f".into(), type_idx: 9 });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::TypeIndex, .. }), "{e:?}");
}

#[test]
fn type_index_call_indirect() {
    let m = with_code(vec![Inst::new(Op::CallIndirect, 7, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::TypeIndex, .. }), "{e:?}");
}

#[test]
fn function_index_call() {
    let m = with_code(vec![Inst::new(Op::Call, 1, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::FunctionIndex, .. }), "{e:?}");
}

#[test]
fn function_index_export() {
    let mut m = base();
    m.exports.push(Export { name: "x".into(), kind: ExportKind::Func(1) });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::FunctionIndex, .. }), "{e:?}");
}

#[test]
fn global_index_get() {
    let m = with_code(vec![Inst::new(Op::GlobalGet, 0, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::GlobalIndex, .. }), "{e:?}");
}

#[test]
fn global_index_set() {
    let m = with_code(vec![Inst::new(Op::GlobalSet, 3, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::GlobalIndex, .. }), "{e:?}");
}

#[test]
fn global_index_export() {
    let mut m = base();
    m.exports.push(Export { name: "x".into(), kind: ExportKind::Global(0) });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::GlobalIndex, .. }), "{e:?}");
}

#[test]
fn table_entry() {
    // Function count is 1, so entry 1 (function index 1) is out of range.
    let mut m = base();
    m.table = vec![Some(0), None, Some(1)];
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::TableEntry, .. }), "{e:?}");
}

#[test]
fn duplicate_export() {
    let mut m = base();
    m.globals.push(Global { ty: ValType::I32, mutable: false, init: 0 });
    m.exports.push(Export { name: "x".into(), kind: ExportKind::Func(0) });
    m.exports.push(Export { name: "x".into(), kind: ExportKind::Global(0) });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::DuplicateExport, .. }), "{e:?}");
}

#[test]
fn register_d() {
    // memory.size d=4 with nregs 4
    let m = with_code(vec![Inst::new(Op::MemorySize, 4, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_a() {
    // i32.add d=0, a=4, b=1
    let m = with_code(vec![Inst::new(Op::I32Add, 0, 4, 1), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_b() {
    // i32.add d=0, a=1, b=4
    let m = with_code(vec![Inst::new(Op::I32Add, 0, 1, 4), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_select_first_packed() {
    // select d=0, cond=1, ra=4, rb=2
    let m = with_code(vec![Inst::select(0, 1, 4, 2), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_select_second_packed() {
    // select d=0, cond=1, ra=2, rb=4
    let m = with_code(vec![Inst::select(0, 1, 2, 4), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_load_base() {
    // i32.load d=0, base=4
    let m = with_code(vec![Inst::new(Op::I32Load, 0, 4, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_store_src() {
    // i32.store src=4, base=0
    let m = with_code(vec![Inst::new(Op::I32Store, 4, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn register_br_if_a() {
    let m = with_code(vec![Inst::new(Op::BrIf, 9, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Register, .. }), "{e:?}");
}

#[test]
fn label_jmp() {
    let m = with_code(vec![Inst::new(Op::Jmp, 2, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Label, .. }), "{e:?}");
}

#[test]
fn label_br_if() {
    let m = with_code(vec![Inst::new(Op::BrIf, 0, 2, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Label, .. }), "{e:?}");
}

#[test]
fn label_br_table_target() {
    let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 0, 0), ret()]);
    m.functions[0].br_tables.push(BrTable { targets: vec![0, 9], default: 0 });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Label, .. }), "{e:?}");
}

#[test]
fn label_br_table_default() {
    let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 0, 0), ret()]);
    m.functions[0].br_tables.push(BrTable { targets: vec![0, 1], default: 7 });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::Label, .. }), "{e:?}");
}

#[test]
fn br_table_index() {
    // br_table refers to br_tables[1], but the function has only one (index 0 absent).
    let mut m = with_code(vec![Inst::new(Op::BrTable, 0, 1, 0), ret()]);
    m.functions[0].br_tables.push(BrTable { targets: vec![], default: 0 });
    let e = reject(m);
    assert!(
        matches!(e, LoadError::Verify { func: Some(0), inst: Some(0), error: VerifyError::BrTableIndex }),
        "{e:?}"
    );
}

#[test]
fn call_window_too_small() {
    // callee type (i32)->(i32) needs base + 1 <= nregs; base 4 gives 5 > 4.
    let m = with_code(vec![Inst::new(Op::Call, 0, 4, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::CallWindow, .. }), "{e:?}");
}

#[test]
fn call_window_huge_base() {
    let m = with_code(vec![Inst::new(Op::Call, 0, u32::MAX, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::CallWindow, .. }), "{e:?}");
}

#[test]
fn call_window_exact_fit_accepted() {
    // base 3 + max(1,1) == nregs 4: fits exactly. ret base 3 with one result also fits.
    let mut m = base();
    m.functions[0].code = vec![Inst::new(Op::Call, 0, 3, 0), Inst::new(Op::Ret, 3, 0, 0)];
    assert!(Module::new(m).is_ok());
}

#[test]
fn call_indirect_window_too_small() {
    let mut m = base();
    m.functions[0].code = vec![Inst::new(Op::CallIndirect, 0, 0, 4), ret()];
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::CallWindow, .. }), "{e:?}");
}

#[test]
fn call_indirect_window_exact_fit_accepted() {
    let mut m = base();
    m.functions[0].code = vec![Inst::new(Op::CallIndirect, 0, 0, 3), ret()];
    assert!(Module::new(m).is_ok());
}

#[test]
fn ret_window() {
    // ret base 4 with one result needs 4 + 1 <= nregs 4: fails.
    let m = with_code(vec![Inst::new(Op::Ret, 4, 0, 0)]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::RetWindow, .. }), "{e:?}");
}

#[test]
fn nregs_less_than_params() {
    let mut m = base();
    m.types[0] = ty(5, 0);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::NregsLessThanParams, .. }), "{e:?}");
}

#[test]
fn immutable_global_set() {
    let mut m = with_code(vec![Inst::new(Op::GlobalSet, 0, 0, 0), ret()]);
    m.globals.push(Global { ty: ValType::I32, mutable: false, init: 0 });
    let e = reject(m.clone());
    assert!(matches!(e, LoadError::Verify { error: VerifyError::ImmutableGlobalSet, .. }), "{e:?}");
    // Same module with the global made mutable is accepted.
    m.globals[0].mutable = true;
    assert!(Module::new(m).is_ok());
}

#[test]
fn global_init_i32_upper_bits() {
    let mut m = base();
    m.globals.push(Global { ty: ValType::I32, mutable: false, init: 1 << 32 });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::GlobalInit, .. }), "{e:?}");
}

#[test]
fn global_init_f32_upper_bits() {
    let mut m = base();
    m.globals.push(Global { ty: ValType::F32, mutable: false, init: 1 << 32 });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::GlobalInit, .. }), "{e:?}");
}

#[test]
fn global_init_i64_upper_bits_accepted() {
    let mut m = base();
    m.globals.push(Global { ty: ValType::I64, mutable: false, init: 1 << 32 });
    assert!(Module::new(m).is_ok());
}

#[test]
fn memory_limits() {
    let mut m = base();
    m.memory.min_pages = 3;
    m.memory.max_pages = Some(2);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::MemoryLimits, .. }), "{e:?}");
}

#[test]
fn data_out_of_range_past_min_memory() {
    // min 1 page = 65536 bytes; offset 65535 + 2 bytes runs past the end.
    let mut m = base();
    m.memory.min_pages = 1;
    m.data.push(DataSegment { offset: 65535, bytes: vec![1, 2] });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::DataOutOfRange, .. }), "{e:?}");
}

#[test]
fn data_out_of_range_offset_overflow() {
    let mut m = base();
    m.memory.min_pages = 1;
    m.data.push(DataSegment { offset: u64::MAX, bytes: vec![1] });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::DataOutOfRange, .. }), "{e:?}");
}

#[test]
fn data_exactly_fitting_accepted() {
    let mut m = base();
    m.memory.min_pages = 1;
    m.data.push(DataSegment { offset: 65534, bytes: vec![1, 2] });
    assert!(Module::new(m).is_ok());
}

#[test]
fn missing_terminator_empty_code() {
    let m = with_code(vec![]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::MissingTerminator, .. }), "{e:?}");
}

#[test]
fn missing_terminator_ends_in_nop() {
    let m = with_code(vec![ret(), Inst::new(Op::Nop, 0, 0, 0)]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::MissingTerminator, .. }), "{e:?}");
}

#[test]
fn missing_terminator_ends_in_br_if() {
    let m = with_code(vec![Inst::new(Op::BrIf, 0, 0, 0)]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::MissingTerminator, .. }), "{e:?}");
}

#[test]
fn unreachable_is_a_terminator() {
    assert!(Module::new(with_code(vec![Inst::new(Op::Unreachable, 0, 0, 0)])).is_ok());
}

#[test]
fn non_zero_unused_operand_nop() {
    let m = with_code(vec![Inst::new(Op::Nop, 1, 0, 0), ret()]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::NonZeroUnusedOperand, .. }), "{e:?}");
}

#[test]
fn non_zero_unused_operand_ret() {
    let m = with_code(vec![Inst::new(Op::Ret, 0, 1, 0)]);
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::NonZeroUnusedOperand, .. }), "{e:?}");
}

#[test]
fn name_index_func() {
    let mut m = base();
    m.names.funcs.push((3, "x".into()));
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::NameIndex, .. }), "{e:?}");
}

#[test]
fn name_index_reg() {
    // register 4 is out of range for nregs 4
    let mut m = base();
    m.names.regs.push((0, 4, "x".into()));
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::NameIndex, .. }), "{e:?}");
}

#[test]
fn line_index_func() {
    // line table names a function index that is not defined
    let mut m = base();
    m.lines.files.push("a.ef".into());
    m.lines.funcs.push(FuncLines {
        func: 5,
        entries: vec![LineEntry { inst: 0, file: 0, line: 1, column: 1 }],
    });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::LineIndex, .. }), "{e:?}");
}

#[test]
fn line_index_inst() {
    let mut m = base();
    m.lines.files.push("a.ef".into());
    m.lines.funcs.push(FuncLines {
        func: 0,
        entries: vec![LineEntry { inst: 1, file: 0, line: 1, column: 1 }],
    });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::LineIndex, .. }), "{e:?}");
}

#[test]
fn line_index_file() {
    let mut m = base();
    m.lines.files.push("a.ef".into());
    m.lines.funcs.push(FuncLines {
        func: 0,
        entries: vec![LineEntry { inst: 0, file: 1, line: 1, column: 1 }],
    });
    let e = reject(m);
    assert!(matches!(e, LoadError::Verify { error: VerifyError::LineIndex, .. }), "{e:?}");
}

#[test]
fn line_index_valid_accepted() {
    let mut m = base();
    m.lines.files.push("a.ef".into());
    m.lines.funcs.push(FuncLines {
        func: 0,
        entries: vec![LineEntry { inst: 0, file: 0, line: 1, column: 1 }],
    });
    assert!(Module::new(m).is_ok());
}

/// Module importing env.f with type (i32)->(i32), calling it and exporting "main".
fn import_module() -> Module {
    let mut b = ModuleBuilder::new();
    let imp = b.import("env", "f", ty(1, 1));
    let mut fb = FunctionBuilder::new(4);
    fb.call(imp, 0);
    fb.ret(0);
    let f = b.add_func(ty(1, 1), fb);
    b.export_func("main", f);
    b.finish().expect("import module should verify")
}

#[test]
fn link_missing_import() {
    let m = import_module();
    let imports: Imports<()> = Imports::new();
    let e = Instance::new(&m, &imports, Limits::default()).err().expect("expected link error");
    assert!(
        matches!(e, LoadError::Link(LinkError::MissingImport { ref namespace, ref name })
            if namespace == "env" && name == "f"),
        "{e:?}"
    );
}

#[test]
fn link_import_type_mismatch() {
    let m = import_module();
    let mut imports = HostRegistry::<()>::new();
    reg_host(&mut imports, "env", "f", FuncType::new([ValType::I64], [ValType::I32]), |_c, _a, _r| {
        Ok(HostOk::Continue)
    });
    let e = Instance::new(&m, &imports.imports(), Limits::default()).err().expect("expected link error");
    assert!(
        matches!(e, LoadError::Link(LinkError::ImportTypeMismatch { ref namespace, ref name, ref module, ref host })
            if namespace == "env" && name == "f" && module == "(i32) -> i32" && host == "(i64) -> i32"),
        "{e:?}"
    );
}

#[test]
fn link_import_exact_type_accepted() {
    let m = import_module();
    let mut imports = HostRegistry::<()>::new();
    reg_host(&mut imports, "env", "f", ty(1, 1), |_c, _a, _r| Ok(HostOk::Continue));
    assert!(Instance::new(&m, &imports.imports(), Limits::default()).is_ok());
}

#[test]
fn builder_finish_rejects_bad_call_target() {
    let mut b = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(4);
    fb.call(5, 0);
    fb.ret(0);
    b.add_func(ty(1, 1), fb);
    let e = b.finish().expect_err("finish should reject the module");
    assert!(matches!(e, LoadError::Verify { error: VerifyError::FunctionIndex, .. }), "{e:?}");
}
