//! CLI tests for the efescript v2 subcommands (`run`, `build`, `dis`) and the `.efa`
//! shorthand, run through the built binary. Temporary files live under `std::env::temp_dir()`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_efescript");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn efescript(args: &[&str]) -> Output {
    Command::new(BIN).args(args).current_dir(ROOT).output().expect("spawn efescript")
}

/// A per-process scratch directory under the system temp dir.
fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("efescript-v2-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `text` to a file named `name` in the scratch directory and returns its path.
fn write_tmp(name: &str, text: &str) -> PathBuf {
    let path = work_dir().join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn s(p: &Path) -> &str {
    p.to_str().expect("temp path is UTF-8")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn run_subcommand_runs_hello() {
    let o = efescript(&["run", "examples/hello.efa"]);
    assert_eq!(stdout(&o), "hello world!\n");
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn bare_efa_path_is_shorthand_for_run() {
    let o = efescript(&["examples/hello.efa"]);
    assert_eq!(stdout(&o), "hello world!\n");
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn run_missing_file_exits_2_with_empty_stdout() {
    let o = efescript(&["run", "examples/no_such_file.efa"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(!stderr(&o).is_empty());
}

#[test]
fn assembly_error_exits_2_and_names_the_line() {
    let src = write_tmp("v2_bad_add.efa", "export fn main() {\n    i64.add r0, r1\n}\n");
    let o = efescript(&["run", s(&src)]);
    let err = stderr(&o);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(err.contains("error:"), "stderr: {err}");
    assert!(err.contains("v2_bad_add.efa"), "stderr: {err}");
    assert!(err.contains(":2:"), "stderr: {err}");
}

#[test]
fn trap_exits_3_and_reports_unreachable() {
    let src = write_tmp("v2_trap.efa", "export fn main() {\n    unreachable\n}\n");
    let o = efescript(&["run", s(&src)]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("unreachable"), "stderr: {}", stderr(&o));
}

#[test]
fn exhausted_budget_exits_3_and_mentions_budget() {
    let src = write_tmp("v2_endless.efa", "export fn main() {\nloop:\n    jmp loop\n}\n");
    let o = efescript(&["run", s(&src), "--budget", "1000"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("budget"), "stderr: {}", stderr(&o));
}

#[test]
fn build_then_run_and_dis_round_trip() {
    let dir = work_dir();
    let efvm = dir.join("r.efvm");
    let _ = std::fs::remove_file(&efvm);

    // build writes a module and prints nothing
    let o = efescript(&["build", "examples/recursion.efa", "-o", s(&efvm)]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert!(o.stdout.is_empty());
    let bytes = std::fs::read(&efvm).expect("build must write the file");
    assert!(bytes.starts_with(b"EFVM"));

    // running the module matches running the source
    let from_source = efescript(&["run", "examples/recursion.efa"]);
    let from_module = efescript(&["run", s(&efvm)]);
    assert_eq!(from_source.status.code(), Some(0));
    assert_eq!(from_module.status.code(), Some(0));
    assert_eq!(from_module.stdout, from_source.stdout);
    assert_eq!(
        stdout(&from_module),
        "sum(100)=5050\nfactorial(10)=3628800\nfibonacci(20)=6765\n"
    );

    // dis prints a disassembly to stdout
    let o = efescript(&["dis", s(&efvm)]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert!(stdout(&o).contains("export fn main"), "stdout: {}", stdout(&o));

    // dis -o writes an .efa that runs the same way
    let efa = dir.join("r2.efa");
    let o = efescript(&["dis", s(&efvm), "-o", s(&efa)]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let o = efescript(&["run", s(&efa)]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(stdout(&o), stdout(&from_source));

    let _ = std::fs::remove_file(&efvm);
    let _ = std::fs::remove_file(&efa);
}

#[test]
fn build_without_output_exits_2() {
    let o = efescript(&["build", "examples/hello.efa"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("-o"), "stderr: {}", stderr(&o));
}

#[test]
fn no_efvm_is_left_in_the_crate_directory() {
    for dir in [PathBuf::from(ROOT), PathBuf::from(ROOT).join("examples")] {
        let entries = std::fs::read_dir(&dir).expect("crate directory is readable");
        for entry in entries {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(!name.ends_with(".efvm"), "stray build output in {}: {name}", dir.display());
        }
    }
}
