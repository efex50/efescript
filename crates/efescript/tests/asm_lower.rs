use std::cell::RefCell;
use std::rc::Rc;

use efevm::{CallError, ExportKind, Inst, Instance, Limits, Module, Op, ValType};

use efescript::asm::{assemble, Diagnostic};
use efescript::host_std::{std_registry, StdEnv, StdOptions};
use efescript::prelude::CaptureHost;

fn asm(src: &str) -> Module {
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

/// The only diagnostic: (message, line, column, note).
fn one(src: &str) -> (String, u32, u32, Option<String>) {
    let mut v = errs(src);
    assert_eq!(v.len(), 1, "expected one diagnostic, got:\n{}", efescript::asm::Diagnostics(v));
    let d = v.remove(0);
    (d.message, d.line, d.column, d.note)
}

fn run(m: &Module, export: &str, args: &[u64]) -> (Result<Vec<u64>, CallError>, String) {
    let cap = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    let imports = std_registry::<Rc<RefCell<CaptureHost>>>(&StdOptions::default()).imports();
    let mut inst = match Instance::new(m, &imports, Limits::default()) {
        Ok(i) => i,
        Err(e) => panic!("link failed: {e}"),
    };
    let r = inst.call(export, args, &mut env, 1_000_000);
    let out = cap.borrow().out_string();
    (r, out)
}

fn code(m: &Module, func: usize) -> Vec<Inst> {
    m.data().functions[func].code.clone()
}

fn i(op: Op, a: u32, b: u32, c: u32) -> Inst {
    Inst::new(op, a, b, c)
}

#[test]
fn hello_world_runs() {
    let m = asm("; hello.efa
import std.println(i64, i64)          ; pointer, length

msg db \"hello world!\"

export fn main() {
    call std.println(msg, msg.len)
    ret
}
");
    let (r, out) = run(&m, "main", &[]);
    assert_eq!(r.ok(), Some(vec![]));
    assert_eq!(out, "hello world!\n");
    // import 0, function 1: window at r0 (no registers named), two const.i64 then the call.
    assert_eq!(
        code(&m, 0),
        vec![
            Inst::const_i64(0, 1024),
            Inst::const_i64(1, 12),
            i(Op::Call, 0, 0, 0),
            i(Op::Ret, 0, 0, 0),
        ]
    );
    assert_eq!(m.data().functions[0].nregs, 2);
}

#[test]
fn sum_loop_returns_15() {
    let m = asm("export fn sum(n: i64) -> i64 {
    reg acc, t
    const.i64 acc, 0
loop:
    i64.eqz t, n
    br_if t, done
    i64.add acc, acc, n
    i64.sub n, n, 1
    jmp loop
done:
    ret acc
}
");
    let (r, _) = run(&m, "sum", &[5]);
    assert_eq!(r.ok(), Some(vec![15]));
    let c = code(&m, 0);
    // i32 condition: br_if tests the low 32 bits of t (a register, no temp).
    assert_eq!(c[2], i(Op::BrIf, 2, 7, 0));
    assert_eq!(c[4], Inst::const_i64(3, 1));
    assert_eq!(c[6], i(Op::Jmp, 1, 0, 0));
    assert_eq!(c[7], i(Op::Ret, 1, 0, 0));
    assert_eq!(m.data().functions[0].nregs, 4);
}

#[test]
fn hidden_temporaries_and_immediate_typing() {
    let m = asm("fn f(a: i64) -> i64 {
    reg t
    i64.add t, a, 5
    i64.sub t, 7, t
    ret t
}
fn g(x: f64) -> f64 {
    f64.div x, x, 2
    ret x
}
fn h(x: i32) -> i32 {
    i32.add x, x, -1
    i32.add x, x, 4294967295
    ret x
}
");
    assert_eq!(
        code(&m, 0),
        vec![
            Inst::const_i64(2, 5),
            i(Op::I64Add, 1, 0, 2),
            Inst::const_i64(2, 7),
            i(Op::I64Sub, 1, 2, 1),
            i(Op::Ret, 1, 0, 0),
        ]
    );
    assert_eq!(m.data().functions[0].nregs, 3);
    assert_eq!(code(&m, 1)[0], Inst::const_f64(1, 2.0));
    assert_eq!(code(&m, 2)[0], Inst::const_i32(1, -1));
    assert_eq!(code(&m, 2)[2], Inst::const_i32(1, -1));
}

#[test]
fn call_sugar_windows() {
    let m = asm("fn g(x: i64, y: f64) -> (i64, i64) {
    ret (x, x)
}
export fn main() -> i64 {
    reg a, b
    call g(7, 2) -> (a, b)
    call g(a, 1.5)
    ret a
}
");
    assert_eq!(
        code(&m, 0),
        vec![
            i(Op::Mov, 2, 0, 0),
            i(Op::Mov, 3, 0, 0),
            i(Op::Ret, 2, 0, 0),
        ]
    );
    assert_eq!(m.data().functions[0].nregs, 4);
    assert_eq!(
        code(&m, 1),
        vec![
            Inst::const_i64(2, 7),
            Inst::const_f64(3, 2.0),
            i(Op::Call, 0, 2, 0),
            i(Op::Mov, 0, 2, 0),
            i(Op::Mov, 1, 3, 0),
            i(Op::Mov, 2, 0, 0),
            Inst::const_f64(3, 1.5),
            i(Op::Call, 0, 2, 0),
            i(Op::Ret, 0, 0, 0),
        ]
    );
    assert_eq!(m.data().functions[1].nregs, 4);
}

#[test]
fn call_with_results_runs() {
    let m = asm("fn add3(a: i64, b: i64, c: i64) -> i64 {
    i64.add a, a, b
    i64.add a, a, c
    ret a
}
export fn main() -> i64 {
    reg x
    call add3(1, 20, 300) -> x
    call add3(x, x, 1) -> x
    ret x
}
");
    assert_eq!(run(&m, "main", &[]).0.ok(), Some(vec![643]));
}

#[test]
fn ret_forms() {
    let m = asm("fn none() {
    ret
}
fn reg1(a: i64) -> i64 {
    ret a
}
fn imm() -> i64 {
    ret 5
}
fn imm_f() -> f32 {
    ret 2
}
fn tup(a: i64, b: i64) -> (i64, i64) {
    ret (b, 9)
}
fn raw(a: i64, b: i64) -> (i64, i64) {
    ret a
}
fn dflt(a: i64) -> i64 {
    ret
}
");
    assert_eq!(code(&m, 0), vec![i(Op::Ret, 0, 0, 0)]);
    assert_eq!(m.data().functions[0].nregs, 0);
    assert_eq!(code(&m, 1), vec![i(Op::Ret, 0, 0, 0)]);
    assert_eq!(code(&m, 2), vec![Inst::const_i64(0, 5), i(Op::Ret, 0, 0, 0)]);
    assert_eq!(m.data().functions[2].nregs, 1);
    assert_eq!(code(&m, 3), vec![Inst::const_f32(0, 2.0), i(Op::Ret, 0, 0, 0)]);
    assert_eq!(
        code(&m, 4),
        vec![i(Op::Mov, 2, 1, 0), Inst::const_i64(3, 9), i(Op::Ret, 2, 0, 0)]
    );
    assert_eq!(m.data().functions[4].nregs, 4);
    assert_eq!(code(&m, 5), vec![i(Op::Ret, 0, 0, 0)]);
    assert_eq!(m.data().functions[5].nregs, 2); // ret a returns a, a+1
    assert_eq!(m.data().functions[6].nregs, 1);
    let (r, _) = run(&m, "tup", &[]);
    assert!(r.is_err()); // not exported
}

#[test]
fn tuple_results_run() {
    let m = asm("export fn swap(a: i64, b: i64) -> (i64, i64) {
    ret (b, a)
}
");
    assert_eq!(run(&m, "swap", &[1, 2]).0.ok(), Some(vec![2, 1]));
}

/// One statement per instruction shape; the sequence must be `Op::ALL` exactly.
#[test]
fn every_op_assembles_one_to_one() {
    use efevm::Shape;
    let mut body = String::new();
    for op in Op::ALL {
        let n = op.name();
        let line = match op.shape() {
            Shape::None => n.to_string(),
            Shape::D => format!("{n} r0"),
            Shape::DA => format!("{n} r0, r1"),
            Shape::DAB => format!("{n} r0, r1, r2"),
            Shape::ConstI32 | Shape::ConstI64 => format!("{n} r0, 7"),
            Shape::ConstF32 | Shape::ConstF64 => format!("{n} r0, 1.5"),
            Shape::Select => format!("{n} r0, r1, r2, r3"),
            Shape::Load => format!("{n} r0, r1, 4"),
            Shape::Store => format!("{n} r0, r1"),
            Shape::ABC => format!("{n} r0, r1, r2"),
            Shape::GlobalGet => format!("{n} r0, g"),
            Shape::GlobalSet => format!("{n} g, r0"),
            Shape::Jmp => format!("{n} top"),
            Shape::Br => format!("{n} r0, top"),
            Shape::BrTable => format!("{n} r0, [top], top"),
            Shape::Call => format!("{n} f, r0"),
            Shape::CallIndirect => format!("{n} () -> (), r0, r0"),
            Shape::Ret => format!("{n} r0"),
        };
        body.push_str("    ");
        body.push_str(&line);
        body.push('\n');
    }
    let src = format!(
        "global mut g: i64 = 0\nfn f() {{\n    ret\n}}\nfn all() {{\ntop:\n{body}    unreachable\n}}\n"
    );
    let m = asm(&src);
    let ops: Vec<Op> = code(&m, 1).iter().map(|x| x.op).collect();
    let mut want: Vec<Op> = Op::ALL.to_vec();
    want.push(Op::Unreachable);
    assert_eq!(ops, want);
    // Operand details for a few shapes.
    let c = code(&m, 1);
    let find = |op: Op| c.iter().find(|x| x.op == op).copied().unwrap();
    assert_eq!(find(Op::I32Load), i(Op::I32Load, 0, 1, 4));
    assert_eq!(find(Op::I32Store), i(Op::I32Store, 0, 1, 0));
    assert_eq!(find(Op::Select), Inst::select(0, 1, 2, 3));
    assert_eq!(find(Op::GlobalSet), i(Op::GlobalSet, 0, 0, 0));
    assert_eq!(find(Op::Call), i(Op::Call, 0, 0, 0));
}

#[test]
fn immediates_in_other_shapes() {
    let m = asm("const OFF = 16
buf resb 32
fn f(p: i64) {
    reg x
    i64.load x, buf
    i32.store8 'A', buf, OFF
    i64.store 42, p, 8
    f32.store 1, p
    mov x, 3
    mov x, 2.5
    select x, 1, 2, 3.5
    memory.grow x, 1
    memory.fill buf, 0, 16
    ret
}
");
    let c = code(&m, 0);
    let want = vec![
        Inst::const_i64(2, 1024),
        i(Op::I64Load, 1, 2, 0),
        Inst::const_i32(2, 65),
        Inst::const_i64(3, 1024),
        i(Op::I32Store8, 2, 3, 16),
        Inst::const_i64(2, 42),
        i(Op::I64Store, 2, 0, 8),
        Inst::const_f32(2, 1.0),
        i(Op::F32Store, 2, 0, 0),
        Inst::const_i64(2, 3),
        i(Op::Mov, 1, 2, 0),
        Inst::const_f64(2, 2.5),
        i(Op::Mov, 1, 2, 0),
        Inst::const_i32(2, 1),
        Inst::const_i64(3, 2),
        Inst::const_f64(4, 3.5),
        Inst::select(1, 2, 3, 4),
        Inst::const_i64(2, 1),
        i(Op::MemoryGrow, 1, 2, 0),
        Inst::const_i64(2, 1024),
        Inst::const_i64(3, 0),
        Inst::const_i64(4, 16),
        i(Op::MemoryFill, 2, 3, 4),
        i(Op::Ret, 0, 0, 0),
    ];
    assert_eq!(c, want);
    assert_eq!(m.data().functions[0].nregs, 5);
}

#[test]
fn const_forms_and_special_floats() {
    let m = asm("fn f() {
    reg a
    const.f32 a, nan
    const.f32 a, -nan
    const.f32 a, nan:0x7fc00001
    const.f64 a, -inf
    const.f64 a, nan:0x7ff0000000000001
    const.f64 a, 7
    const.f32 a, 1e3
    const.i64 a, 0xffffffffffffffff
    const.i32 a, 'x'
    ret
}
");
    let c = code(&m, 0);
    assert_eq!(c[0].b, 0x7fc0_0000);
    assert_eq!(c[1].b, 0xffc0_0000);
    assert_eq!(c[2].b, 0x7fc0_0001);
    assert_eq!(c[3].const_i64_value(), f64::NEG_INFINITY.to_bits());
    assert_eq!(c[4].const_i64_value(), 0x7ff0_0000_0000_0001);
    assert_eq!(c[5].const_i64_value(), 7.0f64.to_bits());
    assert_eq!(c[6].b, 1000.0f32.to_bits());
    assert_eq!(c[7], Inst::const_i64(0, -1));
    assert_eq!(c[8], Inst::const_i32(0, 120));
}

#[test]
fn data_layout_and_symbols() {
    let m = asm("msg db \"hi\", 10
words dw 1, 2
buf resb 64
fl dd 1.5, -1
q dq -1, 2.5
fn f() -> i64 {
    reg x
    const.i64 x, buf
    const.i64 x, buf.len
    const.i64 x, msg.len
    const.i64 x, fl
    ret x
}
");
    let d = m.data();
    assert_eq!(d.data.len(), 4);
    assert_eq!((d.data[0].offset, d.data[0].bytes.clone()), (1024, vec![b'h', b'i', 10]));
    assert_eq!((d.data[1].offset, d.data[1].bytes.clone()), (1032, vec![1, 0, 2, 0]));
    // buf: 1040..1104; fl at 1104; q at 1104 + 8 = 1112
    let mut fl = 1.5f32.to_bits().to_le_bytes().to_vec();
    fl.extend_from_slice(&(-1i32).to_le_bytes());
    assert_eq!((d.data[2].offset, d.data[2].bytes.clone()), (1104, fl));
    let mut q = (-1i64).to_le_bytes().to_vec();
    q.extend_from_slice(&2.5f64.to_bits().to_le_bytes());
    assert_eq!((d.data[3].offset, d.data[3].bytes.clone()), (1112, q));
    let c = code(&m, 0);
    assert_eq!(c[0], Inst::const_i64(0, 1040));
    assert_eq!(c[1], Inst::const_i64(0, 64));
    assert_eq!(c[2], Inst::const_i64(0, 3));
    assert_eq!(c[3], Inst::const_i64(0, 1104));
    assert_eq!((d.memory.min_pages, d.memory.max_pages), (1, None));
}

#[test]
fn memory_pages() {
    let m = asm("big resb 70000\nfn f() {\n    ret\n}\n");
    assert_eq!(m.data().memory.min_pages, 2);
    assert!(m.data().data.is_empty());
    let m = asm("fn f() {\n    ret\n}\n");
    assert_eq!(m.data().memory.min_pages, 1);
    let m = asm("memory 0\nfn f() {\n    ret\n}\n");
    assert_eq!(m.data().memory.min_pages, 0);
    let m = asm("const N = 3\nmemory N 8\nx db 1\nfn f() {\n    ret\n}\n");
    assert_eq!((m.data().memory.min_pages, m.data().memory.max_pages), (3, Some(8)));
    // Forward references to data symbols work.
    let m = asm("fn f() -> i64 {\n    ret later\n}\nlater db 1\n");
    assert_eq!(code(&m, 0)[0], Inst::const_i64(0, 1024));
}

#[test]
fn globals_get_set() {
    let m = asm("global g: i64 = 5
global mut counter: i64 = 10
global mut h: f64 = 1.5
export global mut ticks: i32 = -1
const K = 7
global kk: i64 = K
export fn main() -> i64 {
    reg x
    global.get x, g
    global.get x, counter
    i64.add x, x, 1
    global.set counter, x
    global.set h, 2
    global.set ticks, -5
    global.get x, counter
    ret x
}
");
    let d = m.data();
    assert_eq!(d.globals.len(), 5);
    assert_eq!((d.globals[0].ty, d.globals[0].mutable, d.globals[0].init), (ValType::I64, false, 5));
    assert_eq!(d.globals[2].init, 1.5f64.to_bits());
    assert_eq!((d.globals[3].ty, d.globals[3].init), (ValType::I32, 0xFFFF_FFFF));
    assert_eq!(d.globals[4].init, 7);
    assert_eq!(m.export_global("ticks"), Some(3));
    let c = code(&m, 0);
    assert_eq!(c[0], i(Op::GlobalGet, 0, 0, 0));
    assert_eq!(c[4], i(Op::GlobalSet, 1, 0, 0));
    assert_eq!(c[5], Inst::const_f64(1, 2.0));
    assert_eq!(c[6], i(Op::GlobalSet, 2, 1, 0));
    assert_eq!(c[7], Inst::const_i32(1, -5));
    assert_eq!(c[8], i(Op::GlobalSet, 3, 1, 0));
    assert_eq!(run(&m, "main", &[]).0.ok(), Some(vec![11]));
}

#[test]
fn table_and_call_indirect() {
    let m = asm("import std.print_i64(i64)
table [inc, null, twice, std.print_i64]
fn inc(x: i64) -> i64 {
    i64.add x, x, 1
    ret x
}
fn twice(x: i64) -> i64 {
    i64.add x, x, x
    ret x
}
export fn main() -> i64 {
    reg v, idx
    const.i64 v, 41
    call_indirect (i64) -> i64, 0(v) -> v
    const.i32 idx, 2
    call_indirect (i64) -> i64, idx(v) -> v
    ret v
}
export fn raw(n: i64) -> i64 {
    reg k
    const.i32 k, 2
    mov r3, n
    call_indirect (i64) -> i64, k, r3
    ret r3
}
");
    assert_eq!(m.data().table, vec![Some(1), None, Some(2), Some(0)]);
    assert_eq!(run(&m, "main", &[]).0.ok(), Some(vec![(41 + 1) * 2]));
    assert_eq!(run(&m, "raw", &[21]).0.ok(), Some(vec![42]));
    // Sugar with an immediate index: args, then const.i32 at W + max(P, R), then the call.
    let c = code(&m, 2);
    let ty = m.data().functions[0].type_idx;
    assert_eq!(c[1], i(Op::Mov, 2, 0, 0));
    assert_eq!(c[2], Inst::const_i32(3, 0));
    assert_eq!(c[3], i(Op::CallIndirect, ty, 3, 2));
    assert_eq!(c[4], i(Op::Mov, 0, 2, 0));
    assert_eq!(m.data().functions[2].nregs, 4);
    // Raw form: one to one, window base r3 + max(P, R) = 4.
    assert_eq!(code(&m, 3)[2], i(Op::CallIndirect, ty, 1, 3));
    assert_eq!(m.data().functions[3].nregs, 4);
}

#[test]
fn br_table_dispatch() {
    let m = asm("export fn pick(i: i32) -> i64 {
    br_table i, [a, b], other
a:
    ret 10
b:
    ret 20
other:
    ret 30
}
export fn empty(i: i32) -> i64 {
    br_table 1, [], d
d:
    ret 7
}
");
    assert_eq!(run(&m, "pick", &[0]).0.ok(), Some(vec![10]));
    assert_eq!(run(&m, "pick", &[1]).0.ok(), Some(vec![20]));
    assert_eq!(run(&m, "pick", &[5]).0.ok(), Some(vec![30]));
    assert_eq!(run(&m, "empty", &[0]).0.ok(), Some(vec![7]));
    let t = &m.data().functions[0].br_tables[0];
    assert_eq!((t.targets.clone(), t.default), (vec![1, 3], 5));
    // The label bound to a statement that starts with a hidden temporary points at the temporary.
    assert_eq!(code(&m, 1)[0], Inst::const_i32(1, 1));
}

#[test]
fn names_exports_and_lines() {
    let m = asm("import std.println(i64, i64)
export global mut ticks: i32 = 0
export fn main(a: i64, r1: i64, c: f64) -> i64 {
    reg x, y
    i64.add x, a, 1
    ret x
}
fn helper() {
    ret
}
");
    let d = m.data();
    assert_eq!(
        d.names.funcs,
        vec![(0, "std.println".to_string()), (1, "main".to_string()), (2, "helper".to_string())]
    );
    // Named parameters (a, c) skip the unnamed r1, then the reg names.
    assert_eq!(
        d.names.regs,
        vec![
            (1, 0, "a".to_string()),
            (1, 2, "c".to_string()),
            (1, 3, "x".to_string()),
            (1, 4, "y".to_string()),
        ]
    );
    // Exports: functions first, then globals.
    assert_eq!(d.exports.len(), 2);
    assert_eq!(d.exports[0].name, "main");
    assert_eq!(d.exports[0].kind, ExportKind::Func(1));
    assert_eq!(d.exports[1].kind, ExportKind::Global(0));
    // Lines: file once; every instruction (temporary included) at the mnemonic's position.
    assert_eq!(d.lines.files, vec!["t.efa".to_string()]);
    let main_lines: Vec<(u32, u32, u32, u32)> = d.lines.funcs[0]
        .entries
        .iter()
        .map(|e| (e.inst, e.file, e.line, e.column))
        .collect();
    assert_eq!(main_lines, vec![(0, 0, 5, 5), (1, 0, 5, 5), (2, 0, 6, 5)]);
    assert_eq!(d.lines.funcs[0].func, 1);
    assert_eq!(d.lines.funcs[1].entries[0].line, 9);
}

#[test]
fn register_forms() {
    // `rN` works next to names; a name shadows a const; the hidden base follows raw registers.
    let m = asm("const n = 5
fn f(n: i64, r1: i64) -> i64 {
    i64.add n, n, r1
    i64.add r1, n, 2
    mov r6, n
    ret n
}
");
    let c = code(&m, 0);
    assert_eq!(c[0], i(Op::I64Add, 0, 0, 1));
    // B = 7 (r6 mentioned): the temporary is r7.
    assert_eq!(c[1], Inst::const_i64(7, 2));
    assert_eq!(c[2], i(Op::I64Add, 1, 0, 7));
    assert_eq!(m.data().functions[0].nregs, 8);
}

#[test]
fn nregs_statement() {
    let m = asm("fn f(a: i64) -> i64 {
    nregs 12
    i64.add a, a, 1
    ret a
}
");
    assert_eq!(m.data().functions[0].nregs, 12);
}

#[test]
fn runs_float_and_memory() {
    let m = asm("buf resb 16
export fn main() -> i64 {
    reg x, f
    i64.store 42, buf, 8
    i64.load x, buf, 8
    f64.store 2.5, buf
    f64.load f, buf
    f64.add f, f, 1
    i64.trunc_f64_s f, f
    i64.add x, x, f
    ret x
}
");
    assert_eq!(run(&m, "main", &[]).0.ok(), Some(vec![45]));
}

// ---------------------------------------------------------------- diagnostics

#[test]
fn diag_unknown_register_with_suggestion() {
    let (m, l, c, n) = one("fn f(acc: i64, n: i64) {
    i64.add acc, acx, n
    ret
}
");
    assert_eq!((m.as_str(), l, c), ("unknown register `acx`", 2, 18));
    assert_eq!(n.as_deref(), Some("did you mean `acc`?"));
    let rendered = errs("fn f(acc: i64, n: i64) {\n    i64.add acc, acx, n\n    ret\n}\n");
    assert!(rendered[0].to_string().contains("^^^ did you mean `acc`?"));
}

#[test]
fn diag_unknown_label_function_global_mnemonic() {
    let (m, l, c, n) = one("fn f() {\nloop:\n    jmp lop\n}\n");
    assert_eq!((m.as_str(), l, c), ("unknown label `lop`", 3, 9));
    assert_eq!(n.as_deref(), Some("did you mean `loop`?"));

    let (m, l, c, n) = one("fn helper() {\n    ret\n}\nfn f() {\n    call helpr()\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("unknown function `helpr`", 5, 10));
    assert_eq!(n.as_deref(), Some("did you mean `helper`?"));

    let (m, l, c, n) = one("global counter: i64 = 0\nfn f() {\n    reg x\n    global.get x, countr\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("unknown global `countr`", 4, 19));
    assert_eq!(n.as_deref(), Some("did you mean `counter`?"));

    let (m, l, c, n) = one("fn f() {\n    reg x\n    i64.ad x, x, x\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("unknown instruction `i64.ad`", 3, 5));
    assert_eq!(n.as_deref(), Some("did you mean `i64.add`?"));
}

#[test]
fn diag_operand_count_and_kind() {
    let (m, l, c, _) = one("fn f() {\n    reg x\n    i64.add x, x\n    ret\n}\n");
    assert_eq!(
        (m.as_str(), l, c),
        ("wrong number of operands for `i64.add`: expected 3, found 2", 3, 5)
    );
    let (m, l, c, _) = one("fn f() {\n    reg x\n    i64.add 5, x, x\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("expected a register, found 5", 3, 13));
    let (m, ..) = one("fn f() {\n    reg x\n    i64.load x, x, x\n    ret\n}\n");
    assert_eq!(m, "expected an immediate, found register `x`");
    let (m, ..) = one("fn f() {\n    reg x\n    i64.add x, x, \"s\"\n    ret\n}\n");
    assert!(m.starts_with("expected a register or an immediate, found a string"), "{m}");
}

#[test]
fn diag_immediate_range_and_kind() {
    let (m, l, c, _) = one("fn f() {\n    reg x\n    const.i32 x, 4294967296\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("integer 4294967296 does not fit in i32", 3, 18));
    let (m, ..) = one("fn f() {\n    reg x\n    i64.add x, x, 18446744073709551616\n    ret\n}\n");
    assert_eq!(m, "integer 18446744073709551616 does not fit in i64");
    let (m, l, c, _) = one("fn f() {\n    reg x\n    i64.add x, x, 1.5\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("float literal in an integer position", 3, 19));
    let (m, ..) = one("fn f() {\n    reg x\n    i32.store x, x, -1\n    ret\n}\n");
    assert_eq!(m, "offset -1 does not fit in u32");
}

#[test]
fn diag_missing_terminator() {
    let (m, l, c, n) = one("fn f() {\n    nop\n}\n");
    assert_eq!(
        (m.as_str(), l, c),
        ("function `f` does not end in a terminator (`ret`, `jmp`, `br_table` or `unreachable`)", 3, 1)
    );
    assert!(n.is_some());
    let (m, ..) = one("fn f() {\n    reg x\n}\n");
    assert_eq!(m, "function `f` has no instructions");
}

#[test]
fn diag_labels() {
    let (m, l, c, n) = one("fn f() {\na:\n    nop\na:\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("duplicate label `a`", 4, 1));
    assert_eq!(n.as_deref(), Some("first defined at line 2"));
    let (m, l, c, _) = one("fn f() {\n    ret\nend:\n}\n");
    assert_eq!((m.as_str(), l, c), ("label `end` has no instruction", 3, 1));
}

#[test]
fn diag_duplicates() {
    let (m, l, c, n) = one("fn f() {\n    ret\n}\nfn f() {\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("duplicate definition of function `f`", 4, 4));
    assert_eq!(n.as_deref(), Some("first defined at line 1"));
    let (m, _, _, n) = one("const A = 1\nA db 1\n");
    assert_eq!(m, "duplicate definition of `A`");
    assert_eq!(n.as_deref(), Some("first defined at line 1"));
    let (m, ..) = one("global g: i64 = 0\nglobal g: i64 = 1\n");
    assert_eq!(m, "duplicate definition of global `g`");
    let (m, ..) = one("export fn f() {\n    ret\n}\nexport global g: i64 = 0\nexport fn g() {\n    ret\n}\n");
    assert_eq!(m, "duplicate export name `g`");
}

#[test]
fn diag_call_arity() {
    let (m, l, c, _) = one("fn g(a: i64) {\n    ret\n}\nfn f() {\n    call g(1, 2)\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("`g` takes 1 argument, found 2", 5, 10));
    let (m, ..) = one("fn g() -> (i64, i64) {\n    ret (1, 2)\n}\nfn f() {\n    reg x\n    call g() -> x\n    ret\n}\n");
    assert_eq!(m, "`g` returns 2 values, found 1 result registers");
    let (m, ..) = one("fn f() -> i64 {\n    ret (1, 2)\n}\n");
    assert!(m.contains("returns 1 value"), "{m}");
    let (m, ..) = one("fn f() -> (i64, i64) {\n    ret 1\n}\n");
    assert!(m.contains("exactly one result"), "{m}");
}

#[test]
fn diag_data_and_memory() {
    let (m, l, c, _) = one("memory 0\nmsg db \"x\"\n");
    assert_eq!((m.as_str(), l, c), ("the data needs 1025 bytes but `memory 0` provides only 0", 1, 1));
    let (m, ..) = one("memory 2 1\n");
    assert_eq!(m, "memory maximum 1 is below the minimum 2");
    let (m, ..) = one("a db 256\n");
    assert!(m.contains("does not fit"), "{m}");
    let (m, ..) = one("a dd \"x\"\n");
    assert!(m.contains("only valid in `db`"), "{m}");
    let (m, ..) = one("a db 1.5\n");
    assert_eq!(m, "float literal in an integer position");
    let (m, ..) = one("a resb 1, 2\n");
    assert!(m.contains("exactly one"), "{m}");
    let (m, ..) = one("memory 1\nmemory 2\n");
    assert_eq!(m, "duplicate `memory` declaration");
}

#[test]
fn diag_nregs_and_registers() {
    let (m, l, c, _) = one("fn f(a: i64) {\n    nregs 1\n    mov r3, a\n    ret\n}\n");
    assert_eq!((m.as_str(), l, c), ("`nregs 1` is too small, this function needs 4", 2, 11));
    let (m, ..) = one("fn f() {\n    nregs 70000\n    ret\n}\n");
    assert_eq!(m, "`nregs 70000` is out of range (the limit is 65535)");
    let (m, ..) = one("fn f(a: i64) {\n    mov r0, r65535\n    ret\n}\n");
    assert!(m.starts_with("too many registers"), "{m}");
    let (m, _, _, n) = one("fn f(a: i64) {\n    reg a\n    ret\n}\n");
    assert_eq!(m, "duplicate register `a`");
    assert!(n.is_some());
    let (m, ..) = one("fn f(a: i64, r5: i64) {\n    ret\n}\n");
    assert!(m.contains("r5"), "{m}");
}

#[test]
fn diag_import_and_table() {
    let (m, ..) = one("import std.pri(i64)\ntable [std.prnt]\n");
    assert_eq!(m, "unknown function `std.prnt`");
    let (m, ..) = one("fn f() {\n    ret\n}\ntable [f]\ntable [f]\n");
    assert_eq!(m, "duplicate `table` declaration");
}

#[test]
fn diag_collects_many() {
    let v = errs("fn a() {\n    jmp nowhere\n    bogus\n    ret\n}\nfn b() {\n    nop\n}\nmemory 0\nx db 1\n");
    let lines: Vec<u32> = v.iter().map(|d| d.line).collect();
    assert!(lines.contains(&2) && lines.contains(&3) && lines.contains(&8), "{lines:?}");
    assert!(v.len() >= 4);
}

#[test]
fn verify_error_maps_to_statement() {
    // global.set on an immutable global is only caught by the verifier.
    let src = "global g: i64 = 0\nfn f() {\n    reg x\n    nop\n    global.set g, 1\n    ret\n}\n";
    let (m, l, c, _) = one(src);
    assert!(m.contains("immutable"), "{m}");
    // The failing instruction is the global.set (the const temp before it has the same span).
    assert_eq!((l, c), (5, 5));
}

#[test]
fn duplicate_export_fn_reports_once() {
    let v = errs("export fn main() {\n    ret\n}\nexport fn main() {\n    ret\n}\n");
    assert_eq!(v.len(), 1, "{}", efescript::asm::Diagnostics(v.clone()));
    assert_eq!(v[0].message, "duplicate definition of function `main`");
}

#[test]
fn too_many_registers_reported_once_and_capped() {
    let names: Vec<String> = (0..70000).map(|k| format!("v{k}")).collect();
    let src = format!("export fn main() {{\n    reg {}\n    ret\n}}\n", names.join(", "));
    let v = errs(&src);
    let n = v.iter().filter(|d| d.message.starts_with("too many registers")).count();
    assert_eq!(n, 1);
    assert!(efescript::asm::Diagnostics(v).to_string().len() < 5000);
}

#[test]
fn diagnostics_are_capped_at_100() {
    let body = "    bogus\n".repeat(300);
    let v = errs(&format!("fn f() {{\n{body}    ret\n}}\n"));
    assert_eq!(v.len(), 100);
    assert_eq!(v[99].line, 0);
    assert!(v[99].message.starts_with("too many errors"));
}

// ---- assembling against an API index (e3-design section 7) ----

mod with_api {
    use super::*;
    use efescript::asm::{assemble_with, AsmOptions};
    use efevm::api::{ApiIndex, ApiType, FieldDesc, HostFn, HostRegistry, Prim, StructDesc};
    use efevm::ApiRef;

    /// `engine.world` v3, `engine.log` v1, and the component `Transform`.
    pub(super) fn index() -> ApiIndex {
        let mut r: HostRegistry<()> = HostRegistry::new();
        r.define_namespace("engine.world", 3, "engine", "The world.").unwrap();
        r.define_namespace("engine.log", 1, "engine", "Logging.").unwrap();
        let mut t = StructDesc::new("Transform", 20, 4)
            .field(FieldDesc::new("pos", 0, Prim::F32, 2))
            .field(FieldDesc::new("rot", 8, Prim::F32, 1))
            .field(FieldDesc::new("scale", 12, Prim::F32, 2))
            .doc("Position, rotation and scale.");
        t.owner = "engine".into();
        t.component = Some(77);
        r.register_type(t).unwrap();
        r.register(
            HostFn::new("engine.world", "spawn").doc("Create an entity.").result("e", ApiType::Entity),
            |_, _, _| Ok(efevm::HostOk::Continue),
        )
        .unwrap();
        r.register(
            HostFn::new("engine.world", "get")
                .doc("Copy a component out.")
                .param("e", ApiType::Entity)
                .param("component", ApiType::I32)
                .param("out", ApiType::Ptr)
                .result("status", ApiType::I32),
            |_, _, _| Ok(efevm::HostOk::Continue),
        )
        .unwrap();
        r.register(
            HostFn::new("engine.log", "info").doc("Log a line.").param("text", ApiType::Str),
            |_, _, _| Ok(efevm::HostOk::Continue),
        )
        .unwrap();
        r.index()
    }

    fn asm_api(src: &str) -> Result<Module, Vec<Diagnostic>> {
        let ix = index();
        assemble_with(src, "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).map_err(|e| e.0)
    }

    /// The only diagnostic: (message, line, column, note).
    fn one_api(src: &str) -> (String, u32, u32, Option<String>) {
        let mut v = match asm_api(src) {
            Ok(_) => panic!("expected errors for:\n{src}"),
            Err(e) => e,
        };
        assert_eq!(v.len(), 1, "expected one diagnostic, got:\n{}", efescript::asm::Diagnostics(v));
        let d = v.remove(0);
        (d.message, d.line, d.column, d.note)
    }

    #[test]
    fn unknown_import_suggests_a_name() {
        let (m, l, c, note) = one_api("memory 1\nimport engine.world.spwan() -> i64\n");
        assert_eq!(m, "unknown host function `engine.world.spwan`");
        assert_eq!((l, c), (2, 8));
        assert_eq!(note.as_deref(), Some("did you mean `engine.world.spawn`?"));
        // nothing close: no suggestion
        let (m, _, _, note) = one_api("import zzz.qqqqqq(i64)\n");
        assert_eq!(m, "unknown host function `zzz.qqqqqq`");
        assert_eq!(note, None);
    }

    #[test]
    fn signature_mismatch_shows_the_index_signature() {
        let (m, l, c, _) = one_api("\n\nimport engine.log.info(i64) -> i32\n");
        assert_eq!((l, c), (3, 8));
        assert!(m.contains("signature mismatch for `engine.log.info`"), "{m}");
        assert!(m.contains("declared (i64) -> i32"), "{m}");
        assert!(m.contains("engine.log.info(text: str)"), "{m}");
        assert!(m.contains("lowers to (i64, i64)"), "{m}");
    }

    #[test]
    fn matching_signature_is_accepted() {
        asm_api("import engine.log.info(i64, i64)\nimport engine.world.get(i64, i32, i64) -> i32\nexport fn main() {\n    ret\n}\n").unwrap();
    }

    #[test]
    fn import_without_signature_takes_it_from_the_index() {
        let m = asm_api(
            "import engine.world.spawn\nimport engine.log.info\nmsg db \"hi\"\n\
             export fn main() {\n    reg e\n    call engine.world.spawn() -> e\n    call engine.log.info(msg, msg.len)\n    ret\n}\n",
        )
        .unwrap();
        let d = m.data();
        assert_eq!(d.types[d.imports[0].type_idx as usize].to_string(), "() -> i64");
        assert_eq!(d.types[d.imports[1].type_idx as usize].to_string(), "(i64, i64)");
        // call arity is still checked against the index signature
        let (m, l, _, _) = one_api("import engine.log.info\nexport fn main() {\n    call engine.log.info(1)\n    ret\n}\n");
        assert_eq!(l, 3);
        assert!(m.contains("engine.log.info"), "{m}");
    }

    #[test]
    fn unknown_import_without_signature_is_one_diagnostic() {
        let (m, l, c, note) = one_api("import engine.log.infoo\n");
        assert_eq!(m, "unknown host function `engine.log.infoo`");
        assert_eq!((l, c), (1, 8));
        assert_eq!(note.as_deref(), Some("did you mean `engine.log.info`?"));
    }

    #[test]
    fn import_without_signature_needs_an_index() {
        let (m, l, c, _) = one("\nimport std.println\n");
        assert_eq!((l, c), (2, 8));
        assert!(m.contains("`std.println` has no signature"), "{m}");
        assert!(m.contains("--api"), "{m}");
    }

    #[test]
    fn struct_constants() {
        let m = asm_api(
            "export fn main() -> i64 {\n    reg a, b, c, d, e\n    const.i64 a, Transform.size\n    const.i64 b, Transform.align\n    \
             const.i64 c, Transform.scale\n    const.i64 d, Transform.id\n    const.i64 e, Transform.rot\n    ret a\n}\n",
        )
        .unwrap();
        let code = code(&m, 0);
        let consts: Vec<Inst> = (0..5).map(|k| code[k]).collect();
        assert_eq!(
            consts,
            vec![
                Inst::const_i64(0, 20),
                Inst::const_i64(1, 4),
                Inst::const_i64(2, 12),
                Inst::const_i64(3, 77),
                Inst::const_i64(4, 8),
            ]
        );
        // usable where a `const` integer is (memory size, resb)
        asm_api("buf resb Transform.size\nexport fn main() {\n    ret\n}\n").unwrap();
        // without the index they are unknown
        let (m, _, _, _) = one("export fn main() -> i64 {\n    reg a\n    const.i64 a, Transform.size\n    ret a\n}\n");
        assert!(m.contains("Transform.size"), "{m}");
    }

    #[test]
    fn unknown_struct_constant_suggests() {
        let (m, l, c, note) =
            one_api("export fn main() {\n    reg a\n    const.i64 a, Transform.scal\n    ret\n}\n");
        assert!(m.contains("Transform.scal"), "{m}");
        assert_eq!((l, c), (3, 18));
        assert_eq!(note.as_deref(), Some("did you mean `Transform.scale`?"));
    }

    #[test]
    fn data_symbol_collides_with_a_field_named_len() {
        // `Blob.len` would be both the data length and a field offset.
        let mut ix = index();
        let mut blob = StructDesc::new("Blob", 8, 4)
            .field(FieldDesc::new("len", 4, Prim::U32, 1))
            .field(FieldDesc::new("tag", 0, Prim::U32, 1));
        blob.owner = "engine".into();
        ix.types.push(blob);
        ix.sort();
        let src = "\n  Blob db 1, 2\nexport fn main() {\n    ret\n}\n";
        let d = assemble_with(src, "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).unwrap_err().0;
        assert_eq!(d.len(), 1, "{}", efescript::asm::Diagnostics(d.clone()));
        assert!(d[0].message.contains("`Blob` collides with the API index"), "{}", d[0].message);
        assert!(d[0].message.contains("`Blob.len`"), "{}", d[0].message);
        assert_eq!((d[0].line, d[0].column), (2, 3));
        // other data symbols are fine
        assemble_with("Other db 1\nexport fn main() {\n    ret\n}\n", "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).unwrap();
    }

    #[test]
    fn section_12_records_hash_and_used_namespaces() {
        let ix = index();
        let m = asm_api("import engine.log.info\nimport engine.world.spawn\nexport fn main() {\n    ret\n}\n").unwrap();
        assert_eq!(
            m.data().api,
            Some(ApiRef {
                hash: ix.hash(),
                namespaces: vec![("engine.log".to_string(), 1), ("engine.world".to_string(), 3)],
            })
        );
        // an index with no imports still records the hash
        let m = asm_api("export fn main() {\n    ret\n}\n").unwrap();
        assert_eq!(m.data().api, Some(ApiRef { hash: ix.hash(), namespaces: vec![] }));
        // survives encode/decode
        let back = Module::decode(&m.encode()).unwrap();
        assert_eq!(back.data().api, m.data().api);
    }

    #[test]
    fn no_index_means_no_section_12() {
        let m = asm("import std.println(i64, i64)\nexport fn main() {\n    ret\n}\n");
        assert_eq!(m.data().api, None);
    }
}
