//! CLI tests for `efescript bundle` and the standalone executables it produces.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_efescript");
const RUNNER: &str = env!("CARGO_BIN_EXE_efescript-runner");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// A unique scratch directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("efescript-bundle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, text).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(exe: &str, args: &[&str]) -> Output {
    Command::new(exe).args(args).current_dir(ROOT).output().expect("spawn")
}

fn s(p: &Path) -> &str {
    p.to_str().expect("UTF-8 path")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn bundle_ok(args: &[&str]) -> Output {
    let o = run(BIN, &[&["bundle"], args].concat());
    assert_eq!(o.status.code(), Some(0), "stderr: {}", err(&o));
    assert!(err(&o).starts_with("bundled "), "stderr: {}", err(&o));
    o
}

fn check_hello(exe: &Path) {
    let o = run(s(exe), &[]);
    assert_eq!(out(&o), "hello world!\n", "stderr: {}", err(&o));
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn hello_with_default_runner_and_both_explicit_runners() {
    let d = Dir::new("hello");
    let a = d.path("a");
    let o = bundle_ok(&["examples/hello.efa", "-o", s(&a)]);
    assert!(err(&o).contains(&format!("bundled {}", s(&a))), "{}", err(&o));
    assert!(err(&o).contains("module"), "{}", err(&o));
    check_hello(&a);
    let b = d.path("b");
    bundle_ok(&["examples/hello.efa", "-o", s(&b), "--runner", RUNNER]);
    check_hello(&b);
    let c = d.path("c");
    bundle_ok(&["examples/hello.efa", "-o", s(&c), "--runner", BIN]);
    check_hello(&c);
    // the slim runner is smaller than the full CLI
    let slim = std::fs::metadata(&b).unwrap().len();
    let full = std::fs::metadata(&c).unwrap().len();
    assert!(slim <= full, "{slim} > {full}");
}

#[test]
fn recursion_matches_run() {
    let d = Dir::new("rec");
    let a = d.path("rec");
    bundle_ok(&["examples/recursion.efa", "-o", s(&a)]);
    let bundled = run(s(&a), &[]);
    let plain = run(BIN, &["run", "examples/recursion.efa"]);
    assert_eq!(bundled.status.code(), Some(0));
    assert_eq!(bundled.stdout, plain.stdout);
    assert!(!bundled.stdout.is_empty());
}

#[test]
fn exit_code_and_trap() {
    let d = Dir::new("codes");
    let seven = d.write("seven.efa", "export fn main() -> i32 {\n    reg x\n    const.i32 x, 7\n    ret x\n}\n");
    let a = d.path("seven");
    bundle_ok(&[s(&seven), "-o", s(&a)]);
    assert_eq!(run(s(&a), &[]).status.code(), Some(7));

    let trap = d.write("trap.efa", "export fn main() {\n    unreachable\n}\n");
    let t = d.path("trap");
    bundle_ok(&[s(&trap), "-o", s(&t)]);
    let o = run(s(&t), &[]);
    assert_eq!(o.status.code(), Some(3));
    assert!(err(&o).contains("unreachable"), "{}", err(&o));
}

#[test]
fn bundle_from_efvm() {
    let d = Dir::new("efvm");
    let m = d.path("hello.efvm");
    let o = run(BIN, &["build", "examples/hello.efa", "-o", s(&m)]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let a = d.path("hello");
    bundle_ok(&[s(&m), "-o", s(&a)]);
    check_hello(&a);
}

#[test]
fn rebundle_from_bundled_runner() {
    let d = Dir::new("rebundle");
    let first = d.path("first");
    bundle_ok(&["examples/hello.efa", "-o", s(&first), "--runner", RUNNER]);
    let seven = d.write("seven.efa", "export fn main() -> i32 {\n    reg x\n    const.i32 x, 7\n    ret x\n}\n");
    let m = d.path("seven.efvm");
    assert_eq!(run(BIN, &["build", s(&seven), "-o", s(&m)]).status.code(), Some(0));
    let second = d.path("second");
    bundle_ok(&[s(&m), "-o", s(&second), "--runner", s(&first)]);
    let o = run(s(&second), &[]);
    assert_eq!(o.status.code(), Some(7));
    assert!(o.stdout.is_empty());
    let want = std::fs::metadata(RUNNER).unwrap().len() + std::fs::metadata(&m).unwrap().len() + 16;
    assert_eq!(std::fs::metadata(&second).unwrap().len(), want);
}

#[test]
fn corrupt_trailer_exits_2() {
    let d = Dir::new("corrupt");
    let mut bytes = std::fs::read(RUNNER).unwrap();
    bytes.extend_from_slice(&u64::MAX.to_le_bytes());
    bytes.extend_from_slice(b"EFVMBNDL");
    let p = d.path("bad");
    std::fs::write(&p, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let o = run(s(&p), &[]);
    assert_eq!(o.status.code(), Some(2), "stderr: {}", err(&o));
    assert!(err(&o).contains("corrupt"), "{}", err(&o));
    assert!(!err(&o).contains("panicked"), "{}", err(&o));
}

#[test]
fn runner_without_module_exits_2() {
    let o = run(RUNNER, &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert_eq!(
        err(&o),
        "efescript-runner: no embedded module (create one with: efescript bundle FILE -o OUT)\n"
    );
}

#[test]
fn bundle_without_output_exits_2() {
    let o = run(BIN, &["bundle", "examples/hello.efa"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("-o"), "{}", err(&o));
    let o = run(BIN, &["bundle"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn assembly_error_exits_2() {
    let d = Dir::new("asmerr");
    let bad = d.write("bad.efa", "export fn main() {\n    i64.add r0, r1\n}\n");
    let o = run(BIN, &["bundle", s(&bad), "-o", s(&d.path("x"))]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains(":2:"), "{}", err(&o));
    assert!(!d.path("x").exists());
}

#[test]
fn refuses_to_overwrite_the_runner() {
    let d = Dir::new("same");
    let copy = d.path("runner-copy");
    std::fs::copy(RUNNER, &copy).unwrap();
    let o = run(BIN, &["bundle", "examples/hello.efa", "-o", s(&copy), "--runner", s(&copy)]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(RUNNER).unwrap());
}

#[cfg(unix)]
#[test]
fn output_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let d = Dir::new("mode");
    let a = d.path("a");
    bundle_ok(&["examples/hello.efa", "-o", s(&a)]);
    assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o755);
}
