//! Language server for `.efa` (E4.4): `efescript lsp [--api FILE.efapi]` over stdio.
//!
//! [`Server`] is the whole protocol state machine: [`Server::handle`] takes one incoming
//! message and returns the messages to send, so it is testable without stdio.
//! [`run_stdio`] only frames messages around it.

mod analysis;
mod project;
mod text;

use crate::asm::{self, ast::Module, AsmOptions};
use efevm::api::ApiIndex;
use efevm::Op;
use lsp_server::{Message, Notification, Request, RequestId, Response};
use lsp_types as lt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;
const NOT_INITIALIZED: i32 = -32002;

type Reply = Result<Value, (i32, String)>;

struct Doc {
    /// The normalised file path of the URI (`None` for a non-file URI).
    path: Option<PathBuf>,
    text: String,
    version: Option<i32>,
    /// The last AST that parsed (kept while the text has syntax errors).
    module: Option<Module>,
}

#[derive(Default)]
struct ApiState {
    /// The `--api` files, in the order given; their merge is `index`.
    paths: Vec<PathBuf>,
    /// The (modified time, length) of each path when the index was last loaded.
    stamps: Vec<Option<(SystemTime, u64)>>,
    index: Option<ApiIndex>,
}

pub struct Server {
    initialized: bool,
    shutdown: bool,
    exited: bool,
    explicit_api: bool,
    watch_dynamic: bool,
    root: Option<PathBuf>,
    docs: HashMap<String, Doc>,
    api: ApiState,
    /// The diagnostics last published per URI (other files of a program are published only
    /// when they change).
    last: HashMap<String, Vec<Value>>,
    /// Per program root: the URIs of files that are not open but got diagnostics.
    extras: HashMap<PathBuf, HashSet<String>>,
}

fn log(kind: u32, message: impl Into<String>) -> Message {
    Message::Notification(Notification::new(
        "window/logMessage".to_string(),
        json!({"type": kind, "message": message.into()}),
    ))
}

fn invalid(msg: impl Into<String>) -> (i32, String) {
    (INVALID_PARAMS, msg.into())
}

macro_rules! params {
    ($t:ty, $v:expr) => {
        serde_json::from_value::<$t>($v).map_err(|e| invalid(format!("malformed params: {e}")))?
    };
}

/// `file:///a/b%20c` to `/a/b c`.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && rest.is_char_boundary(i + 1) && rest.is_char_boundary(i + 3) {
            if let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

fn file_name_of(uri: &str) -> String {
    uri.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(uri).to_string()
}

impl Server {
    /// `api`: the `--api` file, when given on the command line.
    pub fn new(api: Option<PathBuf>) -> Server {
        Server::with_apis(api.into_iter().collect())
    }

    /// `apis`: the `--api` files, when given on the command line. Several are merged the same
    /// way as `efescript build --api` (`ApiIndex::merge`).
    pub fn with_apis(apis: Vec<PathBuf>) -> Server {
        Server {
            initialized: false,
            shutdown: false,
            exited: false,
            explicit_api: !apis.is_empty(),
            watch_dynamic: false,
            root: None,
            docs: HashMap::new(),
            api: ApiState { paths: apis, ..ApiState::default() },
            last: HashMap::new(),
            extras: HashMap::new(),
        }
    }

    /// True after the `exit` notification.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// Process exit code: 0 when `exit` followed `shutdown`, else 1.
    pub fn exit_code(&self) -> i32 {
        if self.shutdown {
            0
        } else {
            1
        }
    }

    /// Whether an API index is loaded.
    pub fn has_index(&self) -> bool {
        self.api.index.is_some()
    }

    /// Handles one incoming message; returns the messages to send (responses, notifications,
    /// requests to the client). Never panics: an internal panic becomes an error response.
    pub fn handle(&mut self, msg: Message) -> Vec<Message> {
        let id = match &msg {
            Message::Request(r) => Some(r.id.clone()),
            _ => None,
        };
        match catch_unwind(AssertUnwindSafe(|| self.handle_inner(msg))) {
            Ok(v) => v,
            Err(_) => match id {
                Some(id) => vec![Message::Response(Response::new_err(id, INTERNAL_ERROR, "internal error".into()))],
                None => vec![log(1, "internal error while handling a notification")],
            },
        }
    }

    fn handle_inner(&mut self, msg: Message) -> Vec<Message> {
        let mut out = Vec::new();
        if self.exited {
            return out;
        }
        match msg {
            Message::Request(req) => {
                if self.initialized {
                    self.refresh(&mut out);
                }
                self.request(req, &mut out);
            }
            Message::Notification(n) => {
                if self.initialized && n.method != "exit" && n.method != "workspace/didChangeWatchedFiles" {
                    self.refresh(&mut out);
                }
                self.notification(n, &mut out);
            }
            Message::Response(_) => {}
        }
        out
    }

    // ---- the API index ----

    /// Reloads the index when one of its files changed (or `force`). True when a new index was
    /// loaded. Several files are merged with `ApiIndex::merge`; a file that fails keeps the
    /// previous index.
    fn reload(&mut self, force: bool, out: &mut Vec<Message>) -> bool {
        if self.api.paths.is_empty() {
            return false;
        }
        let stamps: Vec<Option<(SystemTime, u64)>> = self
            .api
            .paths
            .iter()
            .map(|p| std::fs::metadata(p).ok().and_then(|m| Some((m.modified().ok()?, m.len()))))
            .collect();
        if !force && stamps == self.api.stamps {
            return false;
        }
        self.api.stamps = stamps;
        let names: Vec<String> = self.api.paths.iter().map(|p| p.display().to_string()).collect();
        let mut indices = Vec::with_capacity(self.api.paths.len());
        for path in self.api.paths.clone() {
            let loaded = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| ApiIndex::from_json(&t).map_err(|e| e.to_string()));
            match loaded {
                Ok(ix) => indices.push(ix),
                Err(e) => {
                    out.push(log(1, format!("efescript: cannot load API index {}: {e}", path.display())));
                    return false;
                }
            }
        }
        let refs: Vec<&ApiIndex> = indices.iter().collect();
        match ApiIndex::merge(&refs) {
            Ok(ix) => {
                out.push(log(3, format!("efescript: loaded API index {}", names.join(", "))));
                self.api.index = Some(ix);
                true
            }
            Err(e) => {
                out.push(log(1, format!("efescript: cannot merge API indices {}: {e}", names.join(", "))));
                false
            }
        }
    }

    fn refresh(&mut self, out: &mut Vec<Message>) {
        if self.reload(false, out) {
            let all = self.publish_all();
            out.extend(all);
        }
    }

    /// Looks for a single `*.efapi` in the workspace root (not recursive).
    fn discover(&mut self, out: &mut Vec<Message>) {
        let Some(root) = self.root.clone() else { return };
        let mut found: Vec<PathBuf> = match std::fs::read_dir(&root) {
            Ok(rd) => rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "efapi") && p.is_file())
                .collect(),
            Err(_) => Vec::new(),
        };
        found.sort();
        match found.len() {
            1 => self.api.paths = found,
            0 => out.push(log(3, "efescript: no .efapi file in the workspace root; running without an API index")),
            n => out.push(log(
                2,
                format!("efescript: {n} .efapi files in the workspace root; running without an API index (use --api)"),
            )),
        }
    }

    // ---- requests ----

    fn request(&mut self, req: Request, out: &mut Vec<Message>) {
        let Request { id, method, params } = req;
        let reply: Reply = if method == "initialize" {
            self.initialize(&params, out)
        } else if !self.initialized {
            Err((NOT_INITIALIZED, "server not initialized".into()))
        } else if self.shutdown && method != "shutdown" {
            Err((INVALID_REQUEST, "server is shutting down".into()))
        } else {
            match method.as_str() {
                "shutdown" => {
                    self.shutdown = true;
                    Ok(Value::Null)
                }
                "textDocument/hover" => self.hover(params),
                "textDocument/definition" => self.definition(params),
                "textDocument/completion" => self.completion(params),
                _ => Err((METHOD_NOT_FOUND, format!("unsupported request `{method}`"))),
            }
        };
        out.push(Message::Response(match reply {
            Ok(v) => Response::new_ok(id, v),
            Err((code, msg)) => Response::new_err(id, code, msg),
        }));
    }

    fn initialize(&mut self, p: &Value, out: &mut Vec<Message>) -> Reply {
        if self.initialized {
            return Err((INVALID_REQUEST, "already initialized".into()));
        }
        let root_uri = p
            .pointer("/workspaceFolders/0/uri")
            .or_else(|| p.get("rootUri"))
            .and_then(Value::as_str)
            .and_then(uri_to_path);
        self.root = root_uri.or_else(|| p.get("rootPath").and_then(Value::as_str).map(PathBuf::from));
        self.watch_dynamic =
            p.pointer("/capabilities/workspace/didChangeWatchedFiles/dynamicRegistration").and_then(Value::as_bool) == Some(true);
        self.initialized = true;
        if !self.explicit_api {
            self.discover(out);
        }
        self.reload(true, out);
        Ok(json!({
            "capabilities": {
                "textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": false}},
                "definitionProvider": true,
                "hoverProvider": true,
                "completionProvider": {"triggerCharacters": ["."]}
            },
            "serverInfo": {"name": "efescript-lsp", "version": env!("CARGO_PKG_VERSION")}
        }))
    }

    fn doc(&self, uri: &str) -> Result<&Doc, (i32, String)> {
        self.docs.get(uri).ok_or_else(|| invalid(format!("unknown document {uri}")))
    }

    fn hover(&self, v: Value) -> Reply {
        let p = params!(lt::HoverParams, v);
        let tp = p.text_document_position_params;
        let doc = self.doc(tp.text_document.uri.as_str())?;
        let lines = text::lines(&doc.text);
        let Some(line) = lines.get(tp.position.line as usize) else { return Ok(Value::Null) };
        let Some((s, e, word)) = text::word_at(line, text::char_col(line, tp.position.character)) else {
            return Ok(Value::Null);
        };
        let md = match self.alias_item(doc, &word) {
            Some(hit) => Some(hit.hover()),
            None => doc
                .module
                .as_ref()
                .and_then(|m| analysis::event_hover(self.api.index.as_ref(), m, tp.position.line + 1, &word))
                .or_else(|| analysis::hover_text(self.api.index.as_ref(), &word)),
        };
        let Some(md) = md else { return Ok(Value::Null) };
        let l = tp.position.line;
        Ok(json!({
            "contents": {"kind": "markdown", "value": md},
            "range": text::range_json(l, text::utf16_col(line, s), l, text::utf16_col(line, e))
        }))
    }

    fn definition(&self, v: Value) -> Reply {
        let p = params!(lt::GotoDefinitionParams, v);
        let tp = p.text_document_position_params;
        let uri = tp.text_document.uri.as_str();
        let doc = self.doc(uri)?;
        let lines = text::lines(&doc.text);
        let Some(line) = lines.get(tp.position.line as usize) else { return Ok(Value::Null) };
        let Some((_, _, word)) = text::word_at(line, text::char_col(line, tp.position.character)) else {
            return Ok(Value::Null);
        };
        let Some(m) = &doc.module else { return Ok(Value::Null) };
        if let Some(hit) = self.alias_item(doc, &word) {
            let tl = text::lines(&hit.text);
            return Ok(json!({"uri": hit.uri, "range": text::span_range(&tl, hit.item.name_span)}));
        }
        match analysis::definition(m, tp.position.line + 1, &word) {
            Some(span) => Ok(json!({"uri": uri, "range": text::span_range(&lines, span)})),
            None => Ok(Value::Null),
        }
    }

    fn completion(&self, v: Value) -> Reply {
        let p = params!(lt::CompletionParams, v);
        let tp = p.text_document_position;
        let doc = self.doc(tp.text_document.uri.as_str())?;
        let lines = text::lines(&doc.text);
        let l = tp.position.line;
        let line = lines.get(l as usize).copied().unwrap_or("");
        let cc = text::char_col(line, tp.position.character);
        let Some((start, _prefix)) = text::prefix_at(line, cc) else { return Ok(json!([])) };
        let range = text::range_json(l, text::utf16_col(line, start), l, text::utf16_col(line, cc));
        let mut items: Vec<Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut add = |label: String, kind: u32, detail: String, doc: String| {
            if !seen.insert(label.clone()) {
                return;
            }
            let mut it = json!({
                "label": label, "kind": kind, "detail": detail, "filterText": label,
                "textEdit": {"range": range.clone(), "newText": label}
            });
            if !doc.is_empty() {
                it["documentation"] = json!({"kind": "markdown", "value": doc});
            }
            items.push(it);
        };
        if line[..line.char_indices().nth(start).map_or(line.len(), |(b, _)| b)].trim() == "import event" {
            if let Some(ix) = &self.api.index {
                for (label, detail, d) in analysis::event_candidates(ix) {
                    add(label, 20, detail, d);
                }
            }
            return Ok(Value::Array(items));
        }
        if let Some((alias, _)) = _prefix.split_once('.') {
            if let Some(target) = self.alias_file(doc, alias) {
                for it in analysis::public_items(&target.module) {
                    add(format!("{alias}.{}", it.name), it.kind, it.detail, it.doc);
                }
                return Ok(Value::Array(items));
            }
        }
        if let Some(ix) = &self.api.index {
            for f in &ix.functions {
                add(f.qualified(), 3, f.signature_text(), f.doc.clone());
            }
            for (label, detail, d) in analysis::struct_constants(ix) {
                add(label, 21, detail, d);
            }
        }
        if let Some(m) = &doc.module {
            for (label, kind, detail) in analysis::local_symbols(m, l + 1) {
                add(label, kind, detail, String::new());
            }
        }
        for op in Op::ALL {
            add(op.name().to_string(), 14, op.shape().operands().to_string(), String::new());
        }
        Ok(Value::Array(items))
    }

    // ---- notifications ----

    fn notification(&mut self, n: Notification, out: &mut Vec<Message>) {
        let Notification { method, params: p } = n;
        match method.as_str() {
            "exit" => self.exited = true,
            "initialized" => {
                if self.watch_dynamic {
                    out.push(Message::Request(Request::new(
                        RequestId::from("efescript-watch".to_string()),
                        "client/registerCapability".to_string(),
                        json!({"registrations": [{
                            "id": "efapi-watch",
                            "method": "workspace/didChangeWatchedFiles",
                            "registerOptions": {"watchers": [{"globPattern": "**/*.efapi"}]}
                        }]}),
                    )));
                }
            }
            _ if !self.initialized => {}
            "textDocument/didOpen" => match serde_json::from_value::<lt::DidOpenTextDocumentParams>(p) {
                Ok(p) => {
                    let d = p.text_document;
                    let uri = d.uri.as_str().to_string();
                    self.docs.insert(uri.clone(), Doc { path: uri_to_path(&uri).map(|p| project::normalize(&p)), text: d.text, version: Some(d.version), module: None });
                    self.update_doc(&uri, out);
                }
                Err(e) => out.push(log(1, format!("malformed didOpen: {e}"))),
            },
            "textDocument/didChange" => match serde_json::from_value::<lt::DidChangeTextDocumentParams>(p) {
                Ok(p) => {
                    let uri = p.text_document.uri.as_str().to_string();
                    if let Some(doc) = self.docs.get_mut(&uri) {
                        doc.version = Some(p.text_document.version);
                        for c in p.content_changes {
                            match c.range {
                                None => doc.text = c.text,
                                Some(r) => text::apply_edit(
                                    &mut doc.text,
                                    (r.start.line, r.start.character),
                                    (r.end.line, r.end.character),
                                    &c.text,
                                ),
                            }
                        }
                        self.update_doc(&uri, out);
                    }
                }
                Err(e) => out.push(log(1, format!("malformed didChange: {e}"))),
            },
            "textDocument/didSave" => {
                if let Ok(p) = serde_json::from_value::<lt::DidSaveTextDocumentParams>(p) {
                    let uri = p.text_document.uri.as_str().to_string();
                    if let Some(t) = p.text {
                        if let Some(d) = self.docs.get_mut(&uri) {
                            d.text = t;
                        }
                    }
                    if self.docs.contains_key(&uri) {
                        self.update_doc(&uri, out);
                    }
                }
            }
            "textDocument/didClose" => {
                if let Ok(p) = serde_json::from_value::<lt::DidCloseTextDocumentParams>(p) {
                    let uri = p.text_document.uri.as_str().to_string();
                    if let Some(doc) = self.docs.remove(&uri) {
                        out.push(publish(&uri, None, Vec::new()));
                        self.last.remove(&uri);
                        // The buffer is gone: the other open files of its program see the disk.
                        if let Some(path) = doc.path {
                            self.publish_program(Some(project::program_root(&path)), None, false, out);
                        }
                    }
                }
            }
            "workspace/didChangeWatchedFiles" => {
                if !self.explicit_api && self.api.paths.is_empty() {
                    self.discover(out);
                }
                if self.reload(true, out) {
                    let all = self.publish_all();
                    out.extend(all);
                }
            }
            _ => {}
        }
    }

    // ---- other files of a program ----

    /// The file a `use` alias of `doc` names: an open buffer, else the disk.
    fn alias_file(&self, doc: &Doc, alias: &str) -> Option<AliasFile> {
        let path = analysis::use_path(doc.module.as_ref()?, alias)?;
        let full = project::normalize(&doc.path.as_ref()?.parent()?.join(path));
        if let Some((uri, d)) = self.docs.iter().find(|(_, d)| d.path.as_ref() == Some(&full)) {
            let module = d.module.clone().or_else(|| asm::parse(&d.text, "").ok())?;
            return Some(AliasFile { uri: uri.clone(), text: d.text.clone(), module });
        }
        let text = std::fs::read_to_string(&full).ok()?;
        let module = asm::parse(&text, "").ok()?;
        Some(AliasFile { uri: project::path_to_uri(&full), text, module })
    }

    /// `alias.item` (`m.square`, `m.msg.len`, `m.Point.size`) of `word`, found in the file
    /// the alias names.
    fn alias_item(&self, doc: &Doc, word: &str) -> Option<AliasHit> {
        let (alias, rest) = word.split_once('.')?;
        let target = self.alias_file(doc, alias)?;
        let item = analysis::find_item(&target.module, rest)?;
        Some(AliasHit { uri: target.uri, text: target.text, item })
    }

    // ---- diagnostics ----

    fn update_doc(&mut self, uri: &str, out: &mut Vec<Message>) {
        if let Some(doc) = self.docs.get_mut(uri) {
            if let Ok(m) = asm::parse(&doc.text, "") {
                doc.module = Some(m);
            }
        }
        let Some(doc) = self.docs.get(uri) else { return };
        match doc.path.as_ref() {
            Some(path) => {
                let root = project::program_root(path);
                self.publish_program(Some(root), Some(uri), false, out);
            }
            None => {
                let diags = self.diagnose_single(doc, &file_name_of(uri));
                let version = doc.version;
                out.push(publish(uri, version, diags));
            }
        }
    }

    /// A document that is not a file: a program of one in-memory file.
    fn diagnose_single(&self, doc: &Doc, name: &str) -> Vec<Value> {
        let opts = AsmOptions { api: self.api.index.as_ref(), ..Default::default() };
        let lines = text::lines(&doc.text);
        match asm::assemble_with(&doc.text, name, &opts) {
            Ok(_) => Vec::new(),
            Err(ds) => ds.0.iter().map(|d| lsp_diag(d, &lines)).collect(),
        }
    }

    /// Assembles the program with root file `root` (open buffers over the disk) and returns
    /// its diagnostics by file path.
    fn diagnose_program(&self, root: &Path) -> HashMap<PathBuf, Vec<asm::Diagnostic>> {
        let bufs: HashMap<PathBuf, &str> =
            self.docs.values().filter_map(|d| Some((d.path.clone()?, d.text.as_str()))).collect();
        let base = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let rel = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let src = project::BufSource { base, bufs: &bufs };
        let opts = AsmOptions { api: self.api.index.as_ref(), ..Default::default() };
        let mut by_file: HashMap<PathBuf, Vec<asm::Diagnostic>> = HashMap::new();
        if let Err(ds) = asm::assemble_program(&rel, &src, &opts) {
            for d in ds.0 {
                let key = if d.file.is_empty() { root.to_path_buf() } else { project::normalize(Path::new(&d.file)) };
                by_file.entry(key).or_default().push(d);
            }
        }
        by_file
    }

    /// Publishes the diagnostics of the program rooted at `root` (`None`: documents that are
    /// not files are skipped) to every open file of it and to the other files that have
    /// errors. `primary` is always published; the others only when they changed, or `force`.
    fn publish_program(&mut self, root: Option<PathBuf>, primary: Option<&str>, force: bool, out: &mut Vec<Message>) {
        let Some(root) = root else { return };
        let by_file = self.diagnose_program(&root);
        let mut group: Vec<String> = self
            .docs
            .iter()
            .filter(|(_, d)| d.path.as_ref().is_some_and(|p| project::program_root(p) == root))
            .map(|(u, _)| u.clone())
            .collect();
        group.sort();
        if let Some(p) = primary {
            if let Some(k) = group.iter().position(|u| u == p) {
                let u = group.remove(k);
                group.insert(0, u);
            }
        }
        // (uri, version, diagnostics) of every file to consider.
        let mut jobs: Vec<(String, Option<i32>, Vec<Value>)> = Vec::new();
        let mut open_paths: HashSet<PathBuf> = HashSet::new();
        for uri in &group {
            let Some(doc) = self.docs.get(uri) else { continue };
            let Some(path) = doc.path.clone() else { continue };
            let lines = text::lines(&doc.text);
            let mut diags: Vec<asm::Diagnostic> = by_file.get(&path).cloned().unwrap_or_default();
            // A file the program does not reach still shows its own syntax errors.
            if let Err(own) = asm::parse(&doc.text, &path.to_string_lossy()) {
                for d in own.0 {
                    if !diags.iter().any(|x| (x.line, x.column, &x.message) == (d.line, d.column, &d.message)) {
                        diags.push(d);
                    }
                }
                diags.sort_by_key(|d| (d.line, d.column));
            }
            jobs.push((uri.clone(), doc.version, diags.iter().map(|d| lsp_diag(d, &lines)).collect()));
            open_paths.insert(path);
        }
        let mut extra_paths: Vec<&PathBuf> = by_file.keys().filter(|p| !open_paths.contains(*p)).collect();
        extra_paths.sort();
        let mut new_extras: HashSet<String> = HashSet::new();
        for path in extra_paths {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let lines = text::lines(&text);
            let uri = project::path_to_uri(path);
            new_extras.insert(uri.clone());
            jobs.push((uri, None, by_file[path].iter().map(|d| lsp_diag(d, &lines)).collect()));
        }
        for (uri, version, diags) in jobs {
            let changed = self.last.get(&uri).map_or(!diags.is_empty(), |l| *l != diags);
            if force || changed || primary == Some(uri.as_str()) {
                out.push(publish(&uri, version, diags.clone()));
            }
            self.last.insert(uri, diags);
        }
        let old = self.extras.remove(&root).unwrap_or_default();
        for uri in old.difference(&new_extras) {
            if !self.docs.contains_key(uri) {
                out.push(publish(uri, None, Vec::new()));
                self.last.remove(uri);
            }
        }
        self.extras.insert(root, new_extras);
    }

    fn publish_all(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut uris: Vec<String> = self.docs.keys().cloned().collect();
        uris.sort();
        for uri in &uris {
            let Some(doc) = self.docs.get(uri) else { continue };
            match &doc.path {
                Some(p) => {
                    let r = project::program_root(p);
                    if !roots.contains(&r) {
                        roots.push(r.clone());
                        self.publish_program(Some(r), None, true, &mut out);
                    }
                }
                None => {
                    let diags = self.diagnose_single(doc, &file_name_of(uri));
                    out.push(publish(uri, doc.version, diags));
                }
            }
        }
        out
    }
}

/// A file of the program found through a `use` alias.
struct AliasFile {
    uri: String,
    text: String,
    module: Module,
}

/// An `alias.item` reference resolved to its declaration.
struct AliasHit {
    uri: String,
    text: String,
    item: analysis::PubItem,
}

impl AliasHit {
    fn hover(&self) -> String {
        let tl = text::lines(&self.text);
        let decl = tl
            .get((self.item.item_span.line as usize).saturating_sub(1))
            .map(|l| l.trim().trim_end_matches('{').trim_end().to_string())
            .unwrap_or_default();
        let mut md = format!("```\n{decl}\n```\n\n`{}`: {}", self.item.name, self.item.detail);
        if !self.item.doc.is_empty() {
            md.push_str(&format!("\n\n{}", self.item.doc));
        }
        md.push_str(&format!("\n\ndefined in `{}`", file_name_of(&self.uri)));
        md
    }
}

fn lsp_diag(d: &asm::Diagnostic, lines: &[&str]) -> Value {
    let mut msg = d.message.clone();
    if let Some(n) = &d.note {
        msg.push_str("\nnote: ");
        msg.push_str(n);
    }
    json!({
        "range": text::span_range(lines, d.span()),
        "severity": 1,
        "source": "efescript",
        "message": msg
    })
}

fn publish(uri: &str, version: Option<i32>, diags: Vec<Value>) -> Message {
    let mut p = json!({"uri": uri, "diagnostics": diags});
    if let Some(v) = version {
        p["version"] = json!(v);
    }
    Message::Notification(Notification::new("textDocument/publishDiagnostics".to_string(), p))
}

/// Serves LSP on stdin/stdout and returns the process exit code (0 after `shutdown` + `exit`).
pub fn run_stdio_code(apis: Vec<PathBuf>) -> i32 {
    use std::io::Write;
    let mut server = Server::with_apis(apis);
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    while !server.exited() {
        let msg = match Message::read(&mut input) {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                eprintln!("efescript lsp: {e}");
                continue;
            }
            Err(_) => break,
        };
        for reply in server.handle(msg) {
            let mut o = stdout.lock();
            if reply.write(&mut o).is_err() || o.flush().is_err() {
                return 1;
            }
        }
    }
    server.exit_code()
}

/// [`run_stdio_code`] as an [`std::process::ExitCode`].
pub fn run_stdio(apis: Vec<PathBuf>) -> std::process::ExitCode {
    std::process::ExitCode::from(run_stdio_code(apis) as u8)
}
