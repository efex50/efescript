//! Library-level tests for the efescript v2 examples: each example's output and exit code,
//! as written in its header comment, plus stdin, `std.exit` and `main` return values.

use std::cell::RefCell;
use std::rc::Rc;

use efescript::host_std::StdEnv;
use efescript::prelude::CaptureHost;
use efescript::run_v2::{self, Outcome};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// Runs an example file with a capture host. Returns the outcome, the captured output and
/// the sounds that were requested.
fn run_example(name: &str) -> (Outcome, String, Vec<String>) {
    let path = format!("{ROOT}/examples/{name}");
    let prog = run_v2::load_path(&path).unwrap_or_else(|e| panic!("{name} must load: {e}"));
    let cap = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    let outcome = run_v2::run(&prog, &mut env, None);
    let captured = cap.borrow();
    (outcome, captured.out_string(), captured.sounds.clone())
}

/// Runs inline source text with the given stdin bytes.
fn run_inline(src: &str, input: &[u8]) -> (Outcome, String, Vec<String>) {
    let prog = run_v2::load_source(src, "inline.efa")
        .unwrap_or_else(|e| panic!("inline program must load: {e}"));
    let cap = CaptureHost::shared();
    let mut env = StdEnv::with_input(cap.clone(), input.to_vec());
    let outcome = run_v2::run(&prog, &mut env, None);
    let captured = cap.borrow();
    (outcome, captured.out_string(), captured.sounds.clone())
}

#[test]
fn hello() {
    let (o, out, _) = run_example("hello.efa");
    assert_eq!(out, "hello world!\n");
    assert_eq!(o.exit_code, 0);
    assert_eq!(o.error, None);
}

#[test]
fn strings() {
    let (o, out, _) = run_example("strings.efa");
    let expected = r#"hello
h-e-l-l-o-
ell
say "hi" \ A
5
1000
-2
65535
100000
-3
1234567890123
-5
"#;
    assert_eq!(out, expected);
    assert_eq!(o.exit_code, 0);
}

#[test]
fn jumps() {
    let (o, out, _) = run_example("jumps.efa");
    let expected = r#"i64
eq taken
eq not taken
ne taken
ne not taken
lt_s taken
lt_s not taken
lt_u taken
lt_u not taken
gt_s taken
gt_s not taken
gt_u taken
gt_u not taken
le_s taken
le_s not taken
le_u taken
le_u not taken
ge_s taken
ge_s not taken
ge_u taken
ge_u not taken
eqz taken
eqz not taken
i32
eq taken
eq not taken
ne taken
ne not taken
lt_s taken
lt_s not taken
lt_u taken
lt_u not taken
gt_s taken
gt_s not taken
gt_u taken
gt_u not taken
le_s taken
le_s not taken
le_u taken
le_u not taken
ge_s taken
ge_s not taken
ge_u taken
ge_u not taken
eqz taken
eqz not taken
branches
br_if taken
br_if not taken
br_ifz taken
br_ifz not taken
jmp taken
select: 1
select: -1
br_table 1 in range
br_table 0 in range
br_table default
"#;
    assert_eq!(out, expected);
    assert_eq!(o.exit_code, 0);
}

#[test]
fn recursion() {
    let (o, out, _) = run_example("recursion.efa");
    assert_eq!(out, "sum(100)=5050\nfactorial(10)=3628800\nfibonacci(20)=6765\n");
    assert_eq!(o.exit_code, 0);
}

#[test]
fn floats() {
    let (o, out, _) = run_example("floats.efa");
    let expected = r#"f64
add 8.25
sub 4.25
mul 12.5
div 3.125
sqrt 2.5
min 2
max 6.25
abs 2.5
neg -6.25
floor -3
ceil -2
trunc -2
nearest 3
lt 0
ge 1
nan-eq 0
nan-ne 1
convert -7
i64.trunc_s 2
sat_nan 0
demote 6.25
promote 1.5
bits 4607182418800017408
back 1
bits-f32 1069547520
dq 3.25
dq 0.5
global 1.5
global 1.75
0.1+0.2 0.30000000000000004
nan-value NaN
pos-inf inf
neg-inf -inf
f32
add 8.25
sub 4.25
mul 12.5
div 3.125
sqrt 2.5
min 2
max 6.25
abs 2.5
neg -6.25
floor -3
ceil -2
trunc -2
nearest 3
lt 0
from-bits 1.5
"#;
    assert_eq!(out, expected);
    assert_eq!(o.exit_code, 0);
}

#[test]
fn sound_plays_through_capture_host() {
    let (o, out, sounds) = run_example("sound.efa");
    assert_eq!(out, "sound ok\n");
    assert_eq!(o.exit_code, 0);
    assert_eq!(sounds, ["examples/ominous_worse.ogg"]);
}

#[test]
fn read_line_echoes_stdin() {
    let src = r#"
import std.read_line(i64, i64) -> i64
import std.println(i64, i64)

buf db "                    "

export fn main() {
    reg n
    call std.read_line(buf, buf.len) -> n
    call std.println(buf, n)
    ret
}
"#;
    let (o, out, _) = run_inline(src, b"abc\n");
    assert_eq!(out, "abc\n");
    assert_eq!(o.exit_code, 0);
}

#[test]
fn std_exit_sets_the_exit_code() {
    let src = r#"
import std.println(i64, i64)
import std.exit(i32)

msg db "bye"

export fn main() {
    call std.println(msg, msg.len)
    call std.exit(7)
    ret
}
"#;
    let (o, out, _) = run_inline(src, b"");
    assert_eq!(out, "bye\n");
    assert_eq!(o.exit_code, 7);
    assert_eq!(o.error, None);
}

#[test]
fn main_returning_i32_is_the_exit_code() {
    let src = r#"
export fn main() -> i32 {
    ret 42
}
"#;
    let (o, out, _) = run_inline(src, b"");
    assert_eq!(out, "");
    assert_eq!(o.exit_code, 42);
    assert_eq!(o.error, None);
}

#[test]
fn shared_handle_is_readable_after_run() {
    // The capture host handle is shared, so the caller can read it after the run.
    let prog = run_v2::load_path(&format!("{ROOT}/examples/hello.efa")).unwrap();
    let cap: Rc<RefCell<CaptureHost>> = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    let _ = run_v2::run(&prog, &mut env, None);
    assert_eq!(cap.borrow().out, b"hello world!\n".to_vec());
}
