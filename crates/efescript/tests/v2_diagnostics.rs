//! Diagnostics of the v2 assembler: message, position and note (design section 6).

use efescript::asm::{assemble, Diagnostic};

fn errs(src: &str) -> Vec<Diagnostic> {
    match assemble(src, "t.efa") {
        Ok(_) => panic!("expected an error, but it assembled:\n{src}"),
        Err(d) => d.0,
    }
}

/// The only (or first) diagnostic.
fn first(src: &str) -> Diagnostic {
    let mut v = errs(src);
    assert!(!v.is_empty());
    v.remove(0)
}

fn check(d: &Diagnostic, msg: &str, line: u32, col: u32, note: Option<&str>) {
    assert!(d.message.contains(msg), "message {:?} lacks {:?}", d.message, msg);
    assert_eq!((d.line, d.column), (line, col), "position of {d}");
    match note {
        Some(n) => {
            let got = d.note.as_deref().unwrap_or("");
            assert!(got.contains(n), "note {:?} lacks {:?}", d.note, n);
        }
        None => {}
    }
}

#[test]
fn design_example_unknown_register() {
    let src = "export fn sum(n: i64) -> i64 {\n    reg acc, t\n    const.i64 acc, 0\n    i64.add acc, acx, n\n    ret acc\n}\n";
    let v = errs(src);
    assert_eq!(v.len(), 1);
    check(&v[0], "unknown register `acx`", 4, 18, Some("did you mean `acc`?"));
    assert_eq!(v[0].len, 3);
    assert_eq!(v[0].source_line, "    i64.add acc, acx, n");
}

#[test]
fn unknown_label_with_suggestion() {
    let d = first("export fn f() {\n    jmp lop\nloop:\n    ret\n}\n");
    check(&d, "unknown label `lop`", 2, 9, Some("did you mean `loop`?"));
}

#[test]
fn unknown_function_with_suggestion() {
    let d = first("fn helper() {\n    ret\n}\nexport fn f() {\n    call helpr()\n    ret\n}\n");
    check(&d, "unknown function `helpr`", 5, 10, Some("did you mean `helper`?"));
}

#[test]
fn unknown_function_without_suggestion() {
    let d = first("export fn f() {\n    call nothere(1)\n    ret\n}\n");
    check(&d, "unknown function `nothere`", 2, 10, None);
    assert!(d.note.is_none());
}

#[test]
fn unknown_global_with_suggestion() {
    let d = first("global counter: i64 = 0\nexport fn f() {\n    reg a\n    global.get a, counte\n    ret\n}\n");
    check(&d, "unknown global `counte`", 4, 19, Some("did you mean `counter`?"));
}

#[test]
fn unknown_mnemonic_with_suggestion() {
    let d = first("export fn f() {\n    reg a\n    i64.ad a, a, a\n    ret\n}\n");
    check(&d, "unknown instruction `i64.ad`", 3, 5, Some("did you mean `i64.add`?"));
}

#[test]
fn missing_terminator() {
    let d = first("export fn f() {\n    reg a\n    i64.add a, a, a\n}\n");
    check(&d, "terminator", 4, 1, Some("fall off its end"));
}

#[test]
fn duplicate_label_mentions_first() {
    let d = first("export fn f() {\nx:\n    nop\nx:\n    ret\n}\n");
    check(&d, "duplicate label `x`", 4, 1, Some("first defined at line 2"));
}

#[test]
fn wrong_operand_count() {
    let d = first("export fn f() {\n    reg a\n    i64.add a, a\n    ret\n}\n");
    check(&d, "wrong number of operands for `i64.add`: expected 3, found 2", 3, 5, None);
}

#[test]
fn immediate_in_destination() {
    let d = first("export fn f() {\n    i64.add 5, 1, 2\n    ret\n}\n");
    check(&d, "expected a register, found 5", 2, 13, None);
}

#[test]
fn i32_immediate_out_of_range() {
    let d = first("export fn f() {\n    reg a\n    i32.add a, a, 4294967296\n    ret\n}\n");
    check(&d, "integer 4294967296 does not fit in i32", 3, 19, None);
    assert_eq!(d.len, 10);
}

#[test]
fn float_in_integer_position() {
    let d = first("export fn f() {\n    reg a\n    i64.add a, a, 1.5\n    ret\n}\n");
    check(&d, "float literal in an integer position", 3, 19, None);
}

#[test]
fn call_arity() {
    let d = first("fn g(a: i64, b: i64) -> i64 {\n    ret a\n}\nexport fn f() {\n    reg a\n    call g(1) -> a\n    ret\n}\n");
    check(&d, "`g` takes 2 arguments, found 1", 6, 10, None);
}

#[test]
fn unterminated_string() {
    let d = first("msg db \"abc\nexport fn f() {\n    ret\n}\n");
    check(&d, "unterminated string", 1, 8, None);
}

#[test]
fn bad_escape() {
    let d = first("msg db \"a\\qb\"\nexport fn f() {\n    ret\n}\n");
    check(&d, "unknown escape `\\q`", 1, 10, None);
    assert_eq!(d.len, 2);
}

#[test]
fn missing_closing_brace() {
    let d = first("export fn f() {\n    ret\n");
    check(&d, "expected `}`", 3, 1, Some("starts at line 1"));
}

#[test]
fn import_without_namespace() {
    let d = first("import foo(i64)\nexport fn f() {\n    ret\n}\n");
    check(&d, "must be qualified as `namespace.name`", 1, 8, None);
}

#[test]
fn data_too_large_for_explicit_memory() {
    let d = first("memory 1 1\nbig resb 70000\nexport fn f() {\n    ret\n}\n");
    check(&d, "the data needs 71024 bytes but `memory 1` provides only 65536", 1, 1, None);
}

#[test]
fn three_independent_errors_in_line_order() {
    let src = "\
export fn a() {
    reg x
    i64.add x, x, y
    ret
}

export fn b() {
    jmp nowhere
}

export fn c() {
    reg z
    i64.add z, z
    ret
}
";
    let v = errs(src);
    let lines: Vec<u32> = v.iter().map(|d| d.line).collect();
    assert!(lines.windows(2).all(|w| w[0] <= w[1]), "not in line order: {lines:?}");
    let find = |line: u32| v.iter().find(|d| d.line == line).unwrap_or_else(|| panic!("no error on line {line}: {lines:?}"));
    check(find(3), "unknown register `y`", 3, 19, None);
    check(find(8), "unknown label `nowhere`", 8, 9, None);
    check(find(13), "wrong number of operands", 13, 5, None);
    assert_eq!(v.len(), 3, "{v:?}");
}

#[test]
fn display_is_byte_exact() {
    let src = "export fn sum(n: i64) -> i64 {\n    reg acc, t\n    const.i64 acc, 0\n    i64.add acc, acx, n\n    ret acc\n}\n";
    let d = first(src);
    let want = "\
error: unknown register `acx`
  --> t.efa:4:18
   |
 4 |     i64.add acc, acx, n
   |                  ^^^ did you mean `acc`?";
    assert_eq!(d.to_string(), want);
}
