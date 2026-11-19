//! E4.4: the language server, driven in-process (`Server::handle`) and through the binary.

use efescript::lsp::Server;
use efevm::api::{ApiIndex, HostFn};
use lsp_server::{Message, Notification, Request, RequestId};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const URI: &str = "file:///proj/main.efa";

const GOOD: &str = "import std.println(i64, i64)
msg db \"hi\"
export fn main() {
    reg n
    const.i64 n, 3
loop:
    br_ifz n, done
    jmp loop
done:
    call std.println(msg, msg.len)
    ret
}
";

fn req(id: i32, method: &str, params: Value) -> Message {
    Message::Request(Request::new(RequestId::from(id), method.to_string(), params))
}

fn notif(method: &str, params: Value) -> Message {
    Message::Notification(Notification::new(method.to_string(), params))
}

fn std_index_json() -> String {
    efescript::host_std::std_registry::<efescript::prelude::CaptureHost>(&Default::default()).index().to_json()
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("efescript_lsp_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn response(out: &[Message], id: i32) -> lsp_server::Response {
    out.iter()
        .find_map(|m| match m {
            Message::Response(r) if r.id == RequestId::from(id) => Some(r.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no response {id} in {out:?}"))
}

fn result(out: &[Message], id: i32) -> Value {
    let r = response(out, id);
    r.response_result.unwrap_or_else(|e| panic!("error: {e:?}"))
}

fn notifications<'a>(out: &'a [Message], method: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    out.iter().filter_map(move |m| match m {
        Message::Notification(n) if n.method == method => Some(&n.params),
        _ => None,
    })
}

fn start(api: Option<PathBuf>, root: Option<&Path>) -> Server {
    let mut s = Server::new(api);
    let root_uri = root.map(|r| format!("file://{}", r.display()));
    let out = s.handle(req(1, "initialize", json!({"capabilities": {}, "rootUri": root_uri})));
    let caps = &result(&out, 1)["capabilities"];
    assert_eq!(caps["definitionProvider"], true);
    s.handle(notif("initialized", json!({})));
    s
}

fn open(s: &mut Server, text: &str) -> Vec<Message> {
    s.handle(notif(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": URI, "languageId": "efa", "version": 1, "text": text}}),
    ))
}

fn at(line: u32, character: u32) -> Value {
    json!({"textDocument": {"uri": URI}, "position": {"line": line, "character": character}})
}

fn write_index(path: &Path, ix: &ApiIndex) {
    std::fs::write(path, ix.to_json()).unwrap();
}

fn bump_mtime(path: &Path, secs: u64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(secs)).unwrap();
}

#[test]
fn diagnostics_have_the_right_range() {
    let mut s = start(None, None);
    assert!(notifications(&open(&mut s, GOOD), "textDocument/publishDiagnostics").next().unwrap()["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
    let bad = GOOD.replace("br_ifz n, done", "br_ifz zz, done");
    let out = open(&mut s, &bad);
    let p = notifications(&out, "textDocument/publishDiagnostics").next().unwrap();
    assert_eq!(p["uri"], URI);
    let d = &p["diagnostics"][0];
    assert!(d["message"].as_str().unwrap().contains("zz"), "{d}");
    assert_eq!(d["severity"], 1);
    let col = "    br_ifz zz, done".find("zz").unwrap();
    assert_eq!(d["range"]["start"], json!({"line": 6, "character": col}));
    assert_eq!(d["range"]["end"], json!({"line": 6, "character": col + 2}));
}

#[test]
fn didchange_updates_diagnostics() {
    let mut s = start(None, None);
    open(&mut s, GOOD);
    let bad = GOOD.replace("jmp loop", "jmp nowhere");
    let out = s.handle(notif(
        "textDocument/didChange",
        json!({"textDocument": {"uri": URI, "version": 2}, "contentChanges": [{"text": bad}]}),
    ));
    let p = notifications(&out, "textDocument/publishDiagnostics").next().unwrap();
    assert_eq!(p["version"], 2);
    assert_eq!(p["diagnostics"][0]["range"]["start"]["line"], 7);
}

#[test]
fn index_checked_import_error() {
    let dir = temp_dir("importerr");
    let api = dir.join("game.efapi");
    std::fs::write(&api, std_index_json()).unwrap();
    let mut s = start(Some(api), None);
    assert!(s.has_index());
    let src = GOOD.replace("import std.println(i64, i64)", "import std.nope(i64)");
    let out = open(&mut s, &src);
    let p = notifications(&out, "textDocument/publishDiagnostics").next().unwrap();
    let d = &p["diagnostics"].as_array().unwrap();
    assert!(d.iter().any(|d| d["message"].as_str().unwrap().contains("nope") && d["range"]["start"]["line"] == 0), "{d:?}");
    // a signature-less import is fine with the index
    let out = open(&mut s, &GOOD.replace("import std.println(i64, i64)", "import std.println"));
    assert!(notifications(&out, "textDocument/publishDiagnostics").next().unwrap()["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn definition_of_label_function_and_data() {
    let mut s = start(None, None);
    open(&mut s, GOOD);
    // `jmp loop`: line 7, on `loop`
    let out = s.handle(req(2, "textDocument/definition", at(7, 9)));
    let r = result(&out, 2);
    assert_eq!(r["uri"], URI);
    assert_eq!(r["range"]["start"], json!({"line": 5, "character": 0}));
    // `br_ifz n, done`: `done` on line 6
    let r = result(&s.handle(req(3, "textDocument/definition", at(6, 15))), 3);
    assert_eq!(r["range"]["start"]["line"], 8);
    // `msg.len` on line 9 resolves to the data symbol
    let r = result(&s.handle(req(4, "textDocument/definition", at(9, 28))), 4);
    assert_eq!(r["range"]["start"]["line"], 1);
    // the register `n`
    let r = result(&s.handle(req(5, "textDocument/definition", at(6, 11))), 5);
    assert_eq!(r["range"]["start"]["line"], 3);
    // nothing under whitespace
    assert_eq!(result(&s.handle(req(6, "textDocument/definition", at(10, 0))), 6), Value::Null);
}

#[test]
fn completion_has_host_functions_with_signature() {
    let dir = temp_dir("completion");
    let api = dir.join("game.efapi");
    std::fs::write(&api, std_index_json()).unwrap();
    let mut s = start(Some(api), None);
    open(&mut s, GOOD);
    let out = s.handle(req(2, "textDocument/completion", at(9, 12)));
    let items = result(&out, 2);
    let items = items.as_array().unwrap();
    let ix = ApiIndex::from_json(&std_index_json()).unwrap();
    let f = ix.function("std", "println").unwrap();
    let it = items.iter().find(|i| i["label"] == "std.println").expect("std.println");
    assert_eq!(it["detail"], f.signature_text());
    assert!(it["documentation"]["value"].as_str().unwrap().contains(f.doc.as_str()));
    assert!(items.iter().any(|i| i["label"] == "i64.add"), "mnemonics");
    assert!(items.iter().any(|i| i["label"] == "loop"), "labels");
    assert!(items.iter().any(|i| i["label"] == "msg.len"), "data length");
    assert!(items.iter().any(|i| i["label"] == "n"), "registers");
}

#[test]
fn completion_has_struct_constants() {
    let dir = temp_dir("structs");
    let api = dir.join("game.efapi");
    let mut ix = ApiIndex::from_json(&std_index_json()).unwrap();
    ix.types.push({
        let mut t = efevm::api::StructDesc::new("Pt", 8, 4);
        t = t.field(efevm::api::FieldDesc::new("x", 0, efevm::api::Prim::I32, 1).with_doc("the x"));
        t = t.field(efevm::api::FieldDesc::new("y", 4, efevm::api::Prim::I32, 1));
        t.owner = "std".into();
        t
    });
    write_index(&api, &ix);
    let mut s = start(Some(api), None);
    open(&mut s, GOOD);
    let items = result(&s.handle(req(2, "textDocument/completion", at(10, 4))), 2);
    let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    for l in ["Pt.size", "Pt.align", "Pt.x", "Pt.y"] {
        assert!(labels.contains(&l), "{l}");
    }
    let src = "const K = 1\nexport fn main() {\n    const.i64 r, Pt.y\n    ret\n}\n";
    open(&mut s, src);
    let h = result(&s.handle(req(3, "textDocument/hover", at(2, 18))), 3);
    let v = h["contents"]["value"].as_str().unwrap();
    assert!(v.contains("offset 4"), "{v}");
}

#[test]
fn hover_on_import_shows_doc() {
    let dir = temp_dir("hover");
    let api = dir.join("game.efapi");
    std::fs::write(&api, std_index_json()).unwrap();
    let mut s = start(Some(api), None);
    open(&mut s, GOOD);
    let ix = ApiIndex::from_json(&std_index_json()).unwrap();
    let f = ix.function("std", "println").unwrap();
    let h = result(&s.handle(req(2, "textDocument/hover", at(0, 10))), 2);
    let v = h["contents"]["value"].as_str().unwrap();
    assert!(v.contains(&f.doc) && v.contains(&f.signature_text()), "{v}");
    assert!(v.contains("namespace `std` version"), "{v}");
    assert_eq!(h["range"]["start"], json!({"line": 0, "character": 7}));
    // a mnemonic
    let h = result(&s.handle(req(3, "textDocument/hover", at(4, 6))), 3);
    assert!(h["contents"]["value"].as_str().unwrap().contains("const.i64"));
    // without a word
    assert_eq!(result(&s.handle(req(4, "textDocument/hover", at(11, 5))), 4), Value::Null);
}

#[test]
fn reloads_the_index_when_the_file_changes() {
    let dir = temp_dir("reload");
    let api = dir.join("game.efapi");
    std::fs::write(&api, std_index_json()).unwrap();
    bump_mtime(&api, 0);
    let mut s = start(Some(api.clone()), None);
    let src = "import std.shiny\nexport fn main() {\n    ret\n}\n";
    let out = open(&mut s, src);
    let p = notifications(&out, "textDocument/publishDiagnostics").next().unwrap();
    assert!(!p["diagnostics"].as_array().unwrap().is_empty());

    let mut ix = ApiIndex::from_json(&std_index_json()).unwrap();
    let mut f = HostFn::new("std", "shiny").doc("a new function");
    f.owner = "std".into();
    ix.functions.push(f);
    ix.sort();
    write_index(&api, &ix);
    bump_mtime(&api, 30);
    // any request notices the new mtime and republishes
    let out = s.handle(req(2, "textDocument/hover", at(0, 10)));
    let p = notifications(&out, "textDocument/publishDiagnostics").next().expect("republished");
    assert!(p["diagnostics"].as_array().unwrap().is_empty(), "{p}");
    let v = result(&out, 2)["contents"]["value"].as_str().unwrap().to_string();
    assert!(v.contains("a new function"), "{v}");

    // the watched-files notification forces a reload too
    ix.functions.last_mut().unwrap().doc = "second".into();
    write_index(&api, &ix);
    let out = s.handle(notif("workspace/didChangeWatchedFiles", json!({"changes": [{"uri": "file:///x.efapi", "type": 2}]})));
    assert_eq!(notifications(&out, "textDocument/publishDiagnostics").count(), 1);
}

#[test]
fn discovers_a_single_efapi_in_the_root() {
    let dir = temp_dir("discover");
    std::fs::write(dir.join("a.efapi"), std_index_json()).unwrap();
    let s = start(None, Some(&dir));
    assert!(s.has_index());
    std::fs::write(dir.join("b.efapi"), std_index_json()).unwrap();
    let s = start(None, Some(&dir));
    assert!(!s.has_index());
    let none = temp_dir("discover_none");
    assert!(!start(None, Some(&none)).has_index());
}

#[test]
fn bad_input_gives_errors_not_panics() {
    let mut s = Server::new(None);
    let out = s.handle(req(1, "textDocument/hover", at(0, 0)));
    assert_eq!(response(&out, 1).response_result.unwrap_err().code, -32002);
    let mut s = start(None, None);
    let out = s.handle(req(2, "textDocument/hover", at(0, 0)));
    assert_eq!(response(&out, 2).response_result.unwrap_err().code, -32602, "unknown document");
    let out = s.handle(req(3, "textDocument/hover", json!({"nonsense": 1})));
    assert_eq!(response(&out, 3).response_result.unwrap_err().code, -32602);
    let out = s.handle(req(4, "no/such", json!(null)));
    assert_eq!(response(&out, 4).response_result.unwrap_err().code, -32601);
    open(&mut s, "ret\nfn \u{1F600}(\n");
    for (i, (l, c)) in [(0, 0), (1, 1), (1, 3), (99, 99), (0, 4_000_000_000u32)].into_iter().enumerate() {
        for m in ["textDocument/hover", "textDocument/definition", "textDocument/completion"] {
            let out = s.handle(req(10 + i as i32, m, at(l, c)));
            assert!(response(&out, 10 + i as i32).response_result.is_ok(), "{m} at {l}:{c}");
        }
    }
    s.handle(notif("textDocument/didChange", json!({"textDocument": {"uri": URI, "version": 3}, "contentChanges": [{"range": {"start": {"line": 50, "character": 9}, "end": {"line": 0, "character": 1}}, "text": "\u{1F600}x"}]})));
    s.handle(notif("textDocument/didChange", json!({"bogus": true})));
    s.handle(notif("textDocument/didClose", json!({"textDocument": {"uri": URI}})));
    let out = s.handle(req(90, "shutdown", json!(null)));
    assert!(response(&out, 90).response_result.is_ok());
    let out = s.handle(req(91, "textDocument/hover", at(0, 0)));
    assert_eq!(response(&out, 91).response_result.unwrap_err().code, -32600);
    s.handle(notif("exit", json!(null)));
    assert!(s.exited());
    assert_eq!(s.exit_code(), 0);
}

fn frame(v: &Value) -> Vec<u8> {
    let body = v.to_string();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

fn parse_frames(mut bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let text = std::str::from_utf8(bytes).unwrap();
        let (head, rest) = text.split_once("\r\n\r\n").expect("header");
        let n: usize = head.strip_prefix("Content-Length: ").expect("length").trim().parse().unwrap();
        out.push(serde_json::from_str(&rest[..n]).unwrap());
        bytes = &rest.as_bytes()[n..];
    }
    out
}

#[test]
fn binary_speaks_lsp_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_efescript"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let msgs = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": URI, "languageId": "efa", "version": 1, "text": "bogus\n"}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
        json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    ];
    for m in &msgs {
        stdin.write_all(&frame(m)).unwrap();
    }
    stdin.flush().unwrap();
    let mut bytes = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut bytes).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let all = parse_frames(&bytes);
    let init = all.iter().find(|v| v["id"] == 1).expect("initialize response");
    assert_eq!(init["result"]["capabilities"]["hoverProvider"], true);
    let diag = all.iter().find(|v| v["method"] == "textDocument/publishDiagnostics").expect("diagnostics");
    assert!(!diag["params"]["diagnostics"].as_array().unwrap().is_empty());
    assert!(all.iter().any(|v| v["id"] == 2 && v["result"].is_null()));
}
