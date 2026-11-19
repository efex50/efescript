//! Trap backtraces: names, `file:line:col` and the source line (design sections 7, 11).

use efescript::host_std::StdEnv;
use efescript::prelude::CaptureHost;
use efescript::run_v2::{load_path, load_source, run, Outcome, Program};

const DIV_SRC: &str = "\
fn g(a: i64, b: i64) -> i64 {
    i64.div_s a, a, b
    ret a
}

fn f(a: i64) -> i64 {
    reg r
    call g(a, 0) -> r
    ret r
}

export fn main() {
    reg r
    call f(5) -> r
    ret
}
";

const UNREACH_SRC: &str = "\
fn g() {
    unreachable
}

fn f() {
    call g()
    ret
}

export fn main() {
    call f()
    ret
}
";

fn go(prog: &Program) -> Outcome {
    let cap = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    run(prog, &mut env, None)
}

/// Asserts the frames appear in this order, each as `at NAME (FILE:LINE:COL)`, with the source line.
fn check_frames(err: &str, file: &str, frames: &[(&str, u32, u32, &str)]) {
    let mut pos = 0usize;
    for (name, line, col, text) in frames {
        let needle = format!("at {name} ({file}:{line}:{col})");
        let at = err[pos..].find(&needle).unwrap_or_else(|| panic!("missing or misordered {needle:?} in:\n{err}"));
        pos += at + needle.len();
        let src_line = format!("{line:>7} | {text}");
        let rest = &err[pos..];
        assert!(
            rest.starts_with(&format!("\n{src_line}")),
            "frame {name}: expected source line {src_line:?} right after, got:\n{err}"
        );
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("efescript_v2_backtrace_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&d).expect("create temp dir");
    d
}

#[test]
fn div_by_zero_backtrace() {
    let prog = load_source(DIV_SRC, "bt.efa").unwrap_or_else(|e| panic!("{e}"));
    let o = go(&prog);
    assert_eq!(o.exit_code, 3);
    let e = o.error.unwrap_or_default();
    assert!(e.contains("integer divide by zero"), "{e}");
    check_frames(
        &e,
        "bt.efa",
        &[
            ("g", 2, 5, "    i64.div_s a, a, b"),
            ("f", 8, 5, "    call g(a, 0) -> r"),
            ("main", 14, 5, "    call f(5) -> r"),
        ],
    );
}

#[test]
fn unreachable_backtrace() {
    let prog = load_source(UNREACH_SRC, "bt2.efa").unwrap_or_else(|e| panic!("{e}"));
    let o = go(&prog);
    assert_eq!(o.exit_code, 3);
    let e = o.error.unwrap_or_default();
    assert!(e.starts_with("error: trap: "), "{e}");
    assert!(e.contains("unreachable"), "{e}");
    check_frames(
        &e,
        "bt2.efa",
        &[
            ("g", 2, 5, "    unreachable"),
            ("f", 6, 5, "    call g()"),
            ("main", 11, 5, "    call f()"),
        ],
    );
}

#[test]
fn efvm_roundtrip_keeps_names_and_lines() {
    let dir = temp_dir("efvm");
    for (src, file, trap_text, frames) in [
        (
            DIV_SRC,
            "bt3.efa",
            "integer divide by zero",
            vec![
                ("g", 2u32, 5u32, "    i64.div_s a, a, b"),
                ("f", 8, 5, "    call g(a, 0) -> r"),
                ("main", 14, 5, "    call f(5) -> r"),
            ],
        ),
        (
            UNREACH_SRC,
            "bt4.efa",
            "unreachable",
            vec![("g", 2, 5, "    unreachable"), ("f", 6, 5, "    call g()"), ("main", 11, 5, "    call f()")],
        ),
    ] {
        // The source file must be findable by the name stored in the lines section, because a
        // loaded `.efvm` carries no source text.
        let src_path = dir.join(file);
        std::fs::write(&src_path, src).expect("write source");
        let src_name = src_path.to_string_lossy().to_string();
        let prog = load_source(src, &src_name).unwrap_or_else(|e| panic!("{e}"));
        let efvm = dir.join(format!("{file}.efvm"));
        std::fs::write(&efvm, prog.module.encode()).expect("write efvm");

        let loaded = load_path(&efvm.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        assert!(loaded.source.is_none());
        let o = go(&loaded);
        assert_eq!(o.exit_code, 3);
        let e = o.error.unwrap_or_default();
        assert!(e.contains(trap_text), "{e}");
        check_frames(&e, &src_name, &frames);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn efvm_without_source_file_still_shows_names_and_lines() {
    let dir = temp_dir("nosrc");
    let prog = load_source(DIV_SRC, "gone.efa").unwrap_or_else(|e| panic!("{e}"));
    let efvm = dir.join("p.efvm");
    std::fs::write(&efvm, prog.module.encode()).expect("write efvm");
    let loaded = load_path(&efvm.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
    let e = go(&loaded).error.unwrap_or_default();
    let (g, f, m) = (
        e.find("at g (gone.efa:2:5)"),
        e.find("at f (gone.efa:8:5)"),
        e.find("at main (gone.efa:14:5)"),
    );
    assert!(matches!((g, f, m), (Some(a), Some(b), Some(c)) if a < b && b < c), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
