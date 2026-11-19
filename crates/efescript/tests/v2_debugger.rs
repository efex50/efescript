//! Integration tests for the v2 line-oriented debugger (crates/efescript/src/debugger).

use std::io::Cursor;

use efescript::asm::assemble;
use efescript::debugger::Debugger;
use efevm::api::{ApiIndex, ApiType, HostFn, HostRegistry};
use efevm::HostOk;

#[derive(Default)]
struct TestHost {
    output: Vec<u8>,
}

fn setup_host() -> (efevm::Imports<TestHost>, ApiIndex) {
    let mut reg: HostRegistry<TestHost> = HostRegistry::new();
    reg.define_namespace("std", 1, "std", "Console output").unwrap();
    reg.register(
        HostFn::new("std", "println")
            .doc("Prints string with newline.")
            .param("text", ApiType::Str),
        |caller, args, _res| {
            let slice = args.str(0)?;
            let text = caller.read_str(slice)?;
            caller.host().output.extend_from_slice(text.as_bytes());
            caller.host().output.push(b'\n');
            Ok(HostOk::Continue)
        },
    )
    .unwrap();
    let index = reg.index();
    let imports = reg.imports();
    (imports, index)
}

const TEST_PROG: &str = r#"import std.println(i64, i64)

msg db "hello debug!"

fn callee(a: i64) -> i64 {
    reg x
    const.i64 x, 42
    ret x
}

export fn main() {
    reg r
    call callee(10) -> r
    call std.println(msg, msg.len)
    ret
}
"#;

#[test]
fn test_debugger_scripted_session() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly should succeed");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");
    dbg.add_source("test.efa", TEST_PROG);

    // Drive the debugger with scripted input
    let script = "\
step
break 7
continue
bt
step
regs
mem 1024 12
break 14
continue
step
step
asdfghjkl_garbage
mem 0x99999999 100
break 99999
delete 99999
continue
";

    let mut input = Cursor::new(script.as_bytes());
    let mut output = Vec::new();
    let mut host = TestHost::default();

    dbg.run_script(&mut host, &mut input, &mut output).expect("run_script");
    let out = String::from_utf8_lossy(&output).into_owned();

    // 1. Prompts were printed
    assert!(out.contains("(efdb) "), "should print prompts");

    // 2. Step advances one instruction and location changes
    assert!(
        out.contains("main at test.efa:"),
        "step should print location in main"
    );

    // 3. Line breakpoint stops at that line on continue
    assert!(
        out.contains("callee at test.efa:7"),
        "continue should stop at line 7 in callee"
    );

    // 4. bt inside the callee shows two frames
    assert!(
        out.contains("#0  callee at test.efa:7") && out.contains("#1  main at test.efa:13"),
        "bt should show 2 frames inside callee:\n{out}"
    );

    // 5. regs shows a known value (42)
    assert!(
        out.contains("r1 (x) = 42 (0x2a)"),
        "regs should show r1 (x) = 42 (0x2a):\n{out}"
    );

    // 6. mem shows data bytes of a string ("hello debug!")
    assert!(
        out.contains("hello debug!"),
        "mem should show ascii text of string:\n{out}"
    );
    assert!(
        out.contains("68 65 6c 6c 6f 20 64 65 62 75 67 21"),
        "mem should show hex bytes of string:\n{out}"
    );

    // 7. Import call is shown by name with parameter names when ApiIndex is given
    assert!(
        out.contains("call std.println(text = r"),
        "import call should expand parameter names from ApiIndex:\n{out}"
    );

    // 8. Garbage commands do not panic
    assert!(
        out.contains("unknown command `asdfghjkl_garbage`"),
        "garbage command should report error:\n{out}"
    );
    assert!(
        out.contains("error: out of bounds memory access"),
        "out of bounds mem should report error:\n{out}"
    );

    // 9. Continue to the end prints program finished
    assert!(
        out.contains("program finished"),
        "continuing to the end should print 'program finished':\n{out}"
    );

    // 10. The host function actually ran
    assert_eq!(
        String::from_utf8_lossy(&host.output),
        "hello debug!\n",
        "program output should match"
    );
}

#[test]
fn test_step_and_next() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");
    dbg.add_source("test.efa", TEST_PROG);

    let mut host = TestHost::default();
    let mut out = Vec::new();

    // Step advances instruction
    let cont = dbg.command("step", &mut host, &mut out).unwrap();
    assert!(cont);
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("main at test.efa:13"));

    // Next steps over the call to callee
    out.clear();
    let cont = dbg.command("next", &mut host, &mut out).unwrap();
    assert!(cont);
    let s = String::from_utf8_lossy(&out);
    // After next, we should have stepped over callee and still be in main
    assert!(s.contains("main at test.efa:13") || s.contains("main at test.efa:14"));
}

#[test]
fn test_delete_breakpoint() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    // Set breakpoint
    dbg.command("break 7", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("breakpoint 1 at callee (test.efa:7)"));

    // Delete breakpoint
    out.clear();
    dbg.command("delete 1", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("deleted breakpoint 1"));

    // Deleting again reports error
    out.clear();
    dbg.command("delete 1", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("no breakpoint 1"));

    // Continue should now run to the end without stopping at line 7
    out.clear();
    let cont = dbg.command("continue", &mut host, &mut out).unwrap();
    assert!(!cont, "program should have finished");
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("program finished"));
}

#[test]
fn test_list_command() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    // Without source: list shows disassembled instruction
    dbg.command("list", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("const.i64") || s.contains("call"));

    // With source: list shows 5 lines around current line
    dbg.add_source("test.efa", TEST_PROG);
    out.clear();
    dbg.command("list", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains(">"));
    assert!(s.contains("call callee(10) -> r") || s.contains("reg r"));
}

#[test]
fn test_where_command() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    dbg.command("where", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("main at test.efa:13:"));
}

#[test]
fn test_quit_command() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    let cont = dbg.command("quit", &mut host, &mut out).unwrap();
    assert!(!cont, "quit should return false");
}

#[test]
fn test_func_breakpoint() {
    let (imports, index) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    dbg.command("break callee", &mut host, &mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("breakpoint 1 at callee (inst 0)"));

    out.clear();
    let cont = dbg.command("continue", &mut host, &mut out).unwrap();
    assert!(cont, "should stop at callee");
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("callee at test.efa:7"));
}

#[test]
fn test_without_api_index() {
    let (imports, _) = setup_host();
    let module = assemble(TEST_PROG, "test.efa").expect("assembly");
    // Without ApiIndex: index = None
    let mut dbg = Debugger::new(module, &imports, "main", None).expect("debugger init");

    let mut host = TestHost::default();
    let mut out = Vec::new();

    dbg.command("break 14", &mut host, &mut out).unwrap();
    dbg.command("continue", &mut host, &mut out).unwrap();
    // `call std.println(msg, msg.len)` lowers to argument moves, then the call.
    for _ in 0..2 {
        dbg.command("step", &mut host, &mut out).unwrap();
    }
    let s = String::from_utf8_lossy(&out);
    // Should show call std.println without parameter expansion
    assert!(s.contains("call std.println"));
    assert!(!s.contains("text ="));
}

#[test]
fn cli_debug_steps_hello_and_names_the_host_call() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_efescript"))
        .args(["debug", "examples/hello.efa"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("efescript runs");
    child.stdin.take().unwrap().write_all(b"step\nstep\nregs\ncontinue\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("call std.println(text = r0..r1)"), "{s}");
    assert!(s.contains("r0 = 1024 (0x400)"), "{s}");
    assert!(s.contains("hello world!") && s.contains("program finished"), "{s}");
    assert_eq!(out.status.code(), Some(0));
}
