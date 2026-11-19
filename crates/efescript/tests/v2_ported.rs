//! Examples ported from the legacy `.efe` set (`call.efe` -> `exit.efa`, `test.efe` ->
//! `echo.efa`) and the CLI answer for the removed legacy formats.

use std::process::Command;

use efescript::host_std::StdEnv;
use efescript::prelude::CaptureHost;
use efescript::run_v2;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const BIN: &str = env!("CARGO_BIN_EXE_efescript");

fn run_example(name: &str, input: &[u8]) -> (run_v2::Outcome, String) {
    let prog = run_v2::load_path(&format!("{ROOT}/examples/{name}")).expect("example loads");
    let cap = CaptureHost::shared();
    let mut env = StdEnv::with_input(cap.clone(), input.to_vec());
    let o = run_v2::run(&prog, &mut env, None);
    let out = cap.borrow().out_string();
    (o, out)
}

#[test]
fn exit_example_prints_and_exits_5() {
    let (o, out) = run_example("exit.efa", b"");
    assert_eq!(out, "in test\nback in main\n");
    assert_eq!(o.exit_code, 5);
    assert_eq!(o.error, None);
}

#[test]
fn echo_example_echoes_the_line() {
    let (o, out) = run_example("echo.efa", b"ali\n");
    assert_eq!(out, "name?\nali\n");
    assert_eq!(o.exit_code, 0);
    assert_eq!(o.error, None);
}

#[test]
fn legacy_formats_exit_2_with_a_message() {
    for name in ["old.efe", "old.efec"] {
        let o = Command::new(BIN).arg(name).current_dir(ROOT).output().expect("spawn efescript");
        assert_eq!(o.status.code(), Some(2), "{name}");
        assert!(o.stdout.is_empty());
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            err.contains("the legacy .efe/.efec format was removed; use .efa (see docs/efa-syntax.md)"),
            "stderr: {err}"
        );
    }
}

// ---- write_file.efa (create_e): std.fs with a temp root ----

fn temp_root(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("efescript-write-file-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn write_file_example_writes_below_the_root() {
    use efescript::host_std::StdOptions;
    let root = temp_root("lib");
    let prog = run_v2::load_path(&format!("{ROOT}/examples/write_file.efa")).expect("example loads");
    let cap = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    let o = run_v2::run_with(&prog, &mut env, None, &StdOptions { fs_root: Some(root.clone()) });
    assert_eq!(o.exit_code, 0, "{:?}", o.error);
    assert_eq!(cap.borrow().out_string(), "written\n");
    assert_eq!(std::fs::read(root.join("haha")).expect("file written"), b"haha\n");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn write_file_example_is_denied_without_a_root() {
    let (o, out) = run_example("write_file.efa", b"");
    assert_eq!(out, "denied\n");
    assert_eq!(o.exit_code, 1);
}

#[test]
fn write_file_example_through_the_cli() {
    let root = temp_root("cli");
    let o = Command::new(BIN)
        .args(["run", "examples/write_file.efa", "--fs-root"])
        .arg(&root)
        .current_dir(ROOT)
        .output()
        .expect("spawn efescript");
    assert_eq!(o.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "written\n");
    assert_eq!(std::fs::read(root.join("haha")).expect("file written"), b"haha\n");
    let _ = std::fs::remove_dir_all(&root);
}
