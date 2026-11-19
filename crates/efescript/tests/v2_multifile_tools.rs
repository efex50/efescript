//! E8 T14: the language server, the debugger and `run` on multi-file programs.

use efescript::asm::{assemble_program, AsmOptions, FsSource};
use efescript::debugger::Debugger;
use efescript::lsp::Server;
use efevm::api::{ApiIndex, ApiType, HostFn, HostRegistry};
use efevm::HostOk;
use lsp_server::{Message, Notification, Request, RequestId};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAIN: &str = "use \"util/math.efa\" as m
import std.println(i64, i64)
export fn main() -> i32 {
    reg x
    call m.square(6) -> x
    call std.println(m.msg, m.msg.len)
    ret x
}
";

const MATH: &str = "pub const LIMIT = 10
pub global mut counter: i64 = 0
pub msg db \"squared\"
pub struct Point {
    x: i32
    y: i32
}
fn hidden() {
    ret
}
pub fn square(n: i32) -> i32 {
    i32.mul n, n, n
    ret n
}
";

fn project(name: &str, main: &str, math: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("efescript_mf_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src/util")).unwrap();
    std::fs::write(d.join("mod.toml"), "[mod]\nid = \"demo\"\n\n[build]\nentry = \"src/main.efa\"\n").unwrap();
    std::fs::write(d.join("src/main.efa"), main).unwrap();
    std::fs::write(d.join("src/util/math.efa"), math).unwrap();
    d
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

fn req(id: i32, method: &str, params: Value) -> Message {
    Message::Request(Request::new(RequestId::from(id), method.to_string(), params))
}

fn notif(method: &str, params: Value) -> Message {
    Message::Notification(Notification::new(method.to_string(), params))
}

fn result(out: &[Message], id: i32) -> Value {
    out.iter()
        .find_map(|m| match m {
            Message::Response(r) if r.id == RequestId::from(id) => Some(r.response_result.clone().unwrap_or(Value::Null)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no response {id}"))
}

fn start(root: &Path) -> Server {
    let mut s = Server::new(None);
    s.handle(req(1, "initialize", json!({"capabilities": {}, "rootUri": uri(root)})));
    s.handle(notif("initialized", json!({})));
    s
}

fn open(s: &mut Server, u: &str, text: &str) -> Vec<Message> {
    s.handle(notif(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": u, "languageId": "efa", "version": 1, "text": text}}),
    ))
}

fn change(s: &mut Server, u: &str, version: i32, text: &str) -> Vec<Message> {
    s.handle(notif(
        "textDocument/didChange",
        json!({"textDocument": {"uri": u, "version": version}, "contentChanges": [{"text": text}]}),
    ))
}

/// The diagnostics published for `u` in `out` (the last publication), if any.
fn published(out: &[Message], u: &str) -> Option<Vec<Value>> {
    out.iter()
        .filter_map(|m| match m {
            Message::Notification(n) if n.method == "textDocument/publishDiagnostics" && n.params["uri"] == u => {
                Some(n.params["diagnostics"].as_array().unwrap().clone())
            }
            _ => None,
        })
        .last()
}

fn position(text: &str, needle: &str, nth_char: usize) -> Value {
    let (l, line) = text.lines().enumerate().find(|(_, l)| l.contains(needle)).unwrap();
    json!({"line": l, "character": line.find(needle).unwrap() + nth_char})
}

#[test]
fn definition_hover_and_completion_follow_the_alias() {
    let d = project("nav", MAIN, MATH);
    let main_uri = uri(&d.join("src/main.efa"));
    let math_uri = uri(&d.join("src/util/math.efa"));
    let mut s = start(&d);
    let out = open(&mut s, &main_uri, MAIN);
    assert_eq!(published(&out, &main_uri).unwrap().len(), 0, "{out:?}");

    // definition of `m.square` is the function in the other file (the math file is not open)
    let at = |p: Value| json!({"textDocument": {"uri": main_uri}, "position": p});
    let out = s.handle(req(2, "textDocument/definition", at(position(MAIN, "m.square", 3))));
    let r = result(&out, 2);
    assert_eq!(r["uri"], math_uri);
    let line = MATH.lines().position(|l| l.starts_with("pub fn square")).unwrap();
    assert_eq!(r["range"]["start"]["line"], line);
    // and a data item through `.len`
    let out = s.handle(req(3, "textDocument/definition", at(position(MAIN, "m.msg.len", 4))));
    let r = result(&out, 3);
    assert_eq!(r["uri"], math_uri);
    assert_eq!(MATH.lines().nth(r["range"]["start"]["line"].as_u64().unwrap() as usize).unwrap(), "pub msg db \"squared\"");

    // hover shows the declaration and the file
    let out = s.handle(req(4, "textDocument/hover", at(position(MAIN, "m.square", 3))));
    let md = result(&out, 4)["contents"]["value"].as_str().unwrap().to_string();
    assert!(md.contains("pub fn square(n: i32) -> i32"), "{md}");
    assert!(md.contains("math.efa"), "{md}");

    // completion after `alias.` lists the public items only
    let text = MAIN.replace("call m.square(6) -> x", "call m.");
    change(&mut s, &main_uri, 2, &text);
    let col = text.lines().nth(4).unwrap().len();
    let out = s.handle(req(5, "textDocument/completion", at(json!({"line": 4, "character": col}))));
    let items = result(&out, 5);
    let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    for want in ["m.square", "m.LIMIT", "m.counter", "m.msg", "m.msg.len", "m.Point", "m.Point.size", "m.Point.x"] {
        assert!(labels.contains(&want), "{want} missing in {labels:?}");
    }
    assert!(!labels.contains(&"m.hidden"), "{labels:?}");
    let sq = items.as_array().unwrap().iter().find(|i| i["label"] == "m.square").unwrap();
    assert_eq!(sq["detail"], "fn(n: i32) -> i32");
}

#[test]
fn errors_in_another_file_go_to_its_uri() {
    let bad_math = MATH.replace("i32.mul n, n, n", "i32.mul n, n, zz");
    let d = project("diag", MAIN, &bad_math);
    let main_uri = uri(&d.join("src/main.efa"));
    let math_uri = uri(&d.join("src/util/math.efa"));
    let mut s = start(&d);
    let out = open(&mut s, &main_uri, MAIN);
    let main_diags = published(&out, &main_uri).unwrap();
    assert!(main_diags.is_empty(), "{main_diags:?}");
    let math_diags = published(&out, &math_uri).expect("the other file gets its diagnostics");
    assert_eq!(math_diags.len(), 1, "{math_diags:?}");
    assert!(math_diags[0]["message"].as_str().unwrap().contains("zz"));
    let line = bad_math.lines().position(|l| l.contains("zz")).unwrap();
    assert_eq!(math_diags[0]["range"]["start"]["line"], line);

    // opening the file shows the same error on it, and fixing the buffer clears it
    let out = open(&mut s, &math_uri, &bad_math);
    assert_eq!(published(&out, &math_uri).unwrap().len(), 1);
    let out = change(&mut s, &math_uri, 2, MATH);
    assert!(published(&out, &math_uri).unwrap().is_empty());
    assert!(published(&out, &main_uri).is_none() || published(&out, &main_uri).unwrap().is_empty());
}

#[test]
fn open_buffers_win_over_the_disk() {
    let d = project("buf", MAIN, MATH);
    let main_uri = uri(&d.join("src/main.efa"));
    let math_uri = uri(&d.join("src/util/math.efa"));
    let mut s = start(&d);
    assert!(published(&open(&mut s, &main_uri, MAIN), &main_uri).unwrap().is_empty());
    // the buffer of math renames `square`; the disk still has it
    let renamed = MATH.replace("pub fn square", "pub fn sq");
    let out = open(&mut s, &math_uri, &renamed);
    assert!(published(&out, &math_uri).unwrap().is_empty());
    let main_diags = published(&out, &main_uri).expect("main is rechecked against the buffer");
    assert_eq!(main_diags.len(), 1, "{main_diags:?}");
    assert!(main_diags[0]["message"].as_str().unwrap().contains("square"), "{main_diags:?}");
    // definition also reads the buffer: `m.square` is gone, nothing to jump to
    let at = json!({"textDocument": {"uri": main_uri}, "position": position(MAIN, "m.square", 3)});
    assert_eq!(result(&s.handle(req(2, "textDocument/definition", at)), 2), Value::Null);
    // closing the buffer goes back to the disk
    let out = s.handle(notif("textDocument/didClose", json!({"textDocument": {"uri": math_uri}})));
    assert!(published(&out, &main_uri).unwrap().is_empty(), "{out:?}");
}

#[test]
fn single_file_outside_a_manifest_is_its_own_root() {
    let d = std::env::temp_dir().join(format!("efescript_mf_solo_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let u = uri(&d.join("a.efa"));
    let mut s = start(&d);
    let out = open(&mut s, &u, "export fn main() {\n    i32.add zz, zz, zz\n    ret\n}\n");
    let ds = published(&out, &u).unwrap();
    assert!(!ds.is_empty() && ds[0]["message"].as_str().unwrap().contains("zz"), "{ds:?}");
}

fn host() -> (efevm::Imports<Out>, ApiIndex) {
    let mut reg: HostRegistry<Out> = HostRegistry::new();
    reg.define_namespace("std", 1, "std", "Console output").unwrap();
    reg.register(HostFn::new("std", "println").doc("p").param("text", ApiType::Str), |caller, args, _| {
        let text = caller.read_str(args.str(0)?)?;
        caller.host().0.push_str(&text);
        Ok(HostOk::Continue)
    })
    .unwrap();
    (reg.imports(), reg.index())
}

#[derive(Default)]
struct Out(String);

#[test]
fn debugger_breaks_in_a_used_file_and_prints_qualified_names() {
    let d = project("dbg", MAIN, MATH);
    let root = format!("{}/src/main.efa", d.display());
    let (fs, rel) = FsSource::for_file(&root);
    let module = assemble_program(&rel, &fs, &AsmOptions::default()).unwrap();
    let (imports, index) = host();
    let mut dbg = Debugger::new(module, &imports, "main", Some(index)).unwrap();
    let line = MATH.lines().position(|l| l.contains("i32.mul")).unwrap() + 1;
    let script = format!("break util/math.efa:{line}\ncontinue\nwhere\nbt\nlist\ncontinue\n");
    let mut out = Vec::new();
    let mut h = Out::default();
    dbg.run_script(&mut h, &mut Cursor::new(script.into_bytes()), &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();
    let file = format!("{}/src/util/math.efa", d.display());
    assert!(out.contains(&format!("(util.math.square at {}:{line})", file)) || out.contains("breakpoint 1 at util.math.square"), "{out}");
    assert!(out.contains(&format!("util.math.square at {file}:{line}")), "{out}");
    assert!(out.contains(&format!("#0  util.math.square at {file}:{line}")), "{out}");
    assert!(out.contains("#1  main at "), "{out}");
    // `list` reads the used file from disk and marks the current line
    assert!(out.contains(&format!("> {line:>4} | pub fn square")) || out.contains(&format!("> {line:>4} |     i32.mul")), "{out}");
    assert_eq!(h.0, "squared");
}

#[test]
fn run_of_a_two_file_program() {
    let d = project("run", MAIN, MATH);
    let exe = env!("CARGO_BIN_EXE_efescript");
    let root = d.join("src/main.efa");
    let o = Command::new(exe).arg("run").arg(&root).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout), "squared\n");
    assert_eq!(o.status.code(), Some(36), "{}", String::from_utf8_lossy(&o.stderr));

    // `dis` prints one file with sanitised names
    let o = Command::new(exe).arg("dis").arg(&root).output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(text.contains("fn util_math_square"), "{text}");
    assert!(!text.contains("util.math.square"), "{text}");

    // an error in the used file is reported with that file's name
    std::fs::write(d.join("src/util/math.efa"), MATH.replace("i32.mul n, n, n", "i32.mul n, n, zz")).unwrap();
    let o = Command::new(exe).arg("run").arg(&root).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("math.efa"), "{}", String::from_utf8_lossy(&o.stderr));
}

fn event_index() -> ApiIndex {
    use efevm::api::{FieldDesc, Prim, StructDesc};
    let mut r: HostRegistry<()> = HostRegistry::new();
    r.set_game("demo");
    r.define_namespace("game", 3, "demo", "The game.").unwrap();
    r.register_type(StructDesc::new("Score", 8, 4).field(FieldDesc::new("points", 0, Prim::I32, 1)).doc("A score."))
        .unwrap();
    r.register_event(efevm::EventDesc::new("game", "score_changed").doc("The score changed.").payload("Score"))
        .unwrap();
    r.register_event(efevm::EventDesc::new("game", "reset").doc("Reset all.")).unwrap();
    r.index()
}

#[test]
fn event_completion_and_hover() {
    let d = std::env::temp_dir().join(format!("efescript_mf_ev_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let api = d.join("demo.efapi");
    std::fs::write(&api, event_index().to_json()).unwrap();
    let mut s = Server::new(Some(api));
    s.handle(req(1, "initialize", json!({"capabilities": {}, "rootUri": uri(&d)})));
    s.handle(notif("initialized", json!({})));
    let u = uri(&d.join("a.efa"));
    let text = "import event game.score_changed\nprovide event got(&Score), \"Got one.\"\nexport fn main() {\n    const.i32 x, game.score_changed.id\n    ret\n}\n";
    open(&mut s, &u, text);
    let at = |l: u32, c: u32| json!({"textDocument": {"uri": u}, "position": {"line": l, "character": c}});
    // hover on the import and on `.id`
    let mut md = |id: i32, p: Value| {
        let out = s.handle(req(id, "textDocument/hover", p));
        result(&out, id)["contents"]["value"].as_str().map(str::to_string).unwrap_or_default()
    };
    let h = md(2, at(0, 20));
    assert!(h.contains("event game.score_changed(&Score)") && h.contains("The score changed."), "{h}");
    let h = md(3, at(3, 20));
    assert!(h.contains("The score changed."), "{h}");
    // hover on a provided event
    let h = md(4, at(1, 15));
    assert!(h.contains("provide event got(&Score)") && h.contains("Got one."), "{h}");
    // completion after `import event `
    change(&mut s, &u, 2, "import event ga\n");
    let out = s.handle(req(5, "textDocument/completion", at(0, 15)));
    let items = result(&out, 5);
    let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["game.reset", "game.score_changed"]);
    assert!(items[0]["documentation"]["value"].as_str().unwrap().contains("Reset all."));
}
