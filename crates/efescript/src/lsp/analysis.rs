//! AST and index queries behind definition, completion and hover.

use crate::asm::ast::*;
use efevm::api::ApiIndex;
use efevm::Op;

pub fn enclosing_func(m: &Module, line: u32) -> Option<&FuncDecl> {
    m.items.iter().find_map(|i| match i {
        Item::Func(f) if f.span.line <= line && line <= f.end.line => Some(f),
        _ => None,
    })
}

fn line_mnemonic(f: &FuncDecl, line: u32) -> Option<&str> {
    f.body.iter().find_map(|s| match &s.kind {
        StmtKind::Inst(i) if i.mnemonic_span.line == line => Some(i.mnemonic.as_str()),
        _ => None,
    })
}

fn find_label(f: &FuncDecl, word: &str) -> Option<Span> {
    f.body.iter().find_map(|s| match &s.kind {
        StmtKind::Label(i) if i.name == word => Some(i.span),
        _ => None,
    })
}

fn find_reg(f: &FuncDecl, word: &str) -> Option<Span> {
    f.params.iter().find(|p| p.name.name == word).map(|p| p.name.span).or_else(|| {
        f.body.iter().find_map(|s| match &s.kind {
            StmtKind::Reg(v) => v.iter().find(|i| i.name == word).map(|i| i.span),
            _ => None,
        })
    })
}

/// The declaration span of `word` used on `line`.
pub fn definition(m: &Module, line: u32, word: &str) -> Option<Span> {
    if let Some(f) = enclosing_func(m, line) {
        let label_first = line_mnemonic(f, line).is_some_and(|mn| mn.starts_with("jmp") || mn.starts_with("br"));
        let found = if label_first {
            find_label(f, word).or_else(|| find_reg(f, word))
        } else {
            find_reg(f, word).or_else(|| find_label(f, word))
        };
        if found.is_some() {
            return found;
        }
    }
    m.items.iter().find_map(|i| match i {
        Item::Func(f) if f.name.name == word => Some(f.name.span),
        Item::Import(d) if d.qualified.name == word => Some(d.qualified.span),
        Item::Const(c) if c.name.name == word => Some(c.name.span),
        Item::Global(g) if g.name.name == word => Some(g.name.span),
        Item::Data(d) if d.name.name == word || word.strip_suffix(".len") == Some(d.name.name.as_str()) => {
            Some(d.name.span)
        }
        _ => None,
    })
}

/// A completion candidate from the module: (label, kind number, detail).
pub fn local_symbols(m: &Module, line: u32) -> Vec<(String, u32, String)> {
    let mut v = Vec::new();
    if let Some(f) = enclosing_func(m, line) {
        for p in &f.params {
            v.push((p.name.name.clone(), 6, format!("parameter: {}", p.ty.ty.name())));
        }
        for s in &f.body {
            match &s.kind {
                StmtKind::Label(i) => v.push((i.name.clone(), 18, "label".into())),
                StmtKind::Reg(r) => v.extend(r.iter().map(|i| (i.name.clone(), 6, "register".to_string()))),
                _ => {}
            }
        }
    }
    for i in &m.items {
        match i {
            Item::Func(f) => v.push((f.name.name.clone(), 3, "function".into())),
            Item::Import(d) => v.push((d.qualified.name.clone(), 3, "import".into())),
            Item::Const(c) => v.push((c.name.name.clone(), 21, "const".into())),
            Item::Global(g) => v.push((g.name.name.clone(), 6, format!("global: {}", g.ty.ty.name()))),
            Item::Data(d) => {
                v.push((d.name.name.clone(), 6, format!("data ({})", d.kind.name())));
                v.push((format!("{}.len", d.name.name), 21, "length of data".into()));
            }
            _ => {}
        }
    }
    v
}

/// Struct constants of the index: (label, detail, documentation).
pub fn struct_constants(ix: &ApiIndex) -> Vec<(String, String, String)> {
    let mut v = Vec::new();
    for t in &ix.types {
        v.push((format!("{}.size", t.name), format!("size of {} in bytes: {}", t.name, t.size), t.doc.clone()));
        v.push((format!("{}.align", t.name), format!("alignment of {}: {}", t.name, t.align), String::new()));
        if let Some(id) = t.component {
            v.push((format!("{}.id", t.name), format!("component id of {}: {}", t.name, id), String::new()));
        }
        for f in &t.fields {
            v.push((
                format!("{}.{}", t.name, f.name),
                format!("offset of {}.{} ({}): {}", t.name, f.name, f.ty.name(), f.offset),
                f.doc.clone(),
            ));
        }
    }
    v
}

/// Markdown hover text for `word`.
pub fn hover_text(ix: Option<&ApiIndex>, word: &str) -> Option<String> {
    if let Some(ix) = ix {
        if let Some((ns, name)) = word.rsplit_once('.') {
            if let Some(f) = ix.function(ns, name) {
                let ver = ix.namespace(ns).map(|n| format!(", namespace `{}` version {}", n.name, n.version)).unwrap_or_default();
                return Some(format!(
                    "```\n{}{}\n```\n\n{}\n\nowner: `{}`{}",
                    f.qualified(),
                    f.signature_text(),
                    f.doc,
                    f.owner,
                    ver
                ));
            }
            if let Some(t) = ix.struct_type(ns) {
                let doc = |d: &str| if d.is_empty() { String::new() } else { format!("\n\n{d}") };
                match name {
                    "size" => return Some(format!("```\n{ns}.size = {}\n```\n\nsize of `{ns}` in bytes{}", t.size, doc(&t.doc))),
                    "align" => return Some(format!("```\n{ns}.align = {}\n```\n\nalignment of `{ns}`", t.align)),
                    "id" if t.component.is_some() => {
                        return Some(format!("```\n{ns}.id = {}\n```\n\ncomponent id of `{ns}`", t.component.unwrap_or(0)))
                    }
                    _ => {}
                }
                if let Some(f) = t.field_named(name) {
                    let count = if f.count > 1 { format!(" x{}", f.count) } else { String::new() };
                    return Some(format!(
                        "```\n{ns}.{name} = {}\n```\n\nfield `{name}: {}{count}` of `{ns}` at offset {}, size {}{}",
                        f.offset,
                        f.ty.name(),
                        f.offset,
                        f.size(),
                        doc(&f.doc)
                    ));
                }
            }
        }
    }
    let op = Op::ALL.iter().find(|o| o.name() == word)?;
    Some(format!("```\n{} {}\n```\n\ninstruction, opcode 0x{:03X}", op.name(), op.shape().operands(), op.code()))
}

// ---- other files of a program (`use` aliases) ----

/// The path of the `use` whose alias (the `as` name or the file stem) is `alias`.
pub fn use_path(m: &Module, alias: &str) -> Option<String> {
    m.items.iter().find_map(|i| match i {
        Item::Use(u) => {
            let path = std::str::from_utf8(&u.path).ok()?;
            let name = match &u.alias {
                Some(a) => a.name.clone(),
                None => path.rsplit('/').next()?.strip_suffix(".efa").unwrap_or(path).to_string(),
            };
            (name == alias).then(|| path.to_string())
        }
        _ => None,
    })
}

fn sig_text(params: &[Param], results: &[TypeRef]) -> String {
    let p: Vec<String> = params.iter().map(|p| format!("{}: {}", p.name.name, p.ty.ty.name())).collect();
    let r = match results {
        [] => String::new(),
        [one] => format!(" -> {}", one.ty.name()),
        many => format!(" -> ({})", many.iter().map(|t| t.ty.name()).collect::<Vec<_>>().join(", ")),
    };
    format!("({}){}", p.join(", "), r)
}

/// A public item of a file: (name, completion kind, detail, doc, span of the name, item span).
pub struct PubItem {
    pub name: String,
    pub kind: u32,
    pub detail: String,
    pub doc: String,
    pub name_span: Span,
    pub item_span: Span,
}

/// The items other files may use (`pub` or exported): functions, consts, globals, data and
/// structs, with the derived `NAME.len` of data and `S.size`, `S.align`, `S.field`.
pub fn public_items(m: &Module) -> Vec<PubItem> {
    let mut v = Vec::new();
    for i in &m.items {
        match i {
            Item::Func(f) if f.public || f.exported => v.push(PubItem {
                name: f.name.name.clone(),
                kind: 3,
                detail: format!("fn{}", sig_text(&f.params, &f.results)),
                doc: String::new(),
                name_span: f.name.span,
                item_span: f.span,
            }),
            Item::Const(c) if c.public => v.push(PubItem {
                name: c.name.name.clone(),
                kind: 21,
                detail: "const".into(),
                doc: String::new(),
                name_span: c.name.span,
                item_span: c.span,
            }),
            Item::Global(g) if g.public || g.exported => v.push(PubItem {
                name: g.name.name.clone(),
                kind: 6,
                detail: format!("global: {}", g.ty.ty.name()),
                doc: String::new(),
                name_span: g.name.span,
                item_span: g.span,
            }),
            Item::Data(d) if d.public => {
                let item = |name: String, kind: u32, detail: String| PubItem {
                    name,
                    kind,
                    detail,
                    doc: String::new(),
                    name_span: d.name.span,
                    item_span: d.span,
                };
                v.push(item(d.name.name.clone(), 6, format!("data ({})", d.kind.name())));
                v.push(item(format!("{}.len", d.name.name), 21, "length of data".into()));
            }
            Item::Struct(s) if s.public || s.exported => {
                let doc = s.doc.as_ref().map(|d| String::from_utf8_lossy(d).into_owned()).unwrap_or_default();
                let item = |name: String, detail: String, doc: String| PubItem {
                    name,
                    kind: 22,
                    detail,
                    doc,
                    name_span: s.name.span,
                    item_span: s.span,
                };
                v.push(item(s.name.name.clone(), "struct".into(), doc));
                v.push(item(format!("{}.size", s.name.name), "size of the struct in bytes".into(), String::new()));
                v.push(item(format!("{}.align", s.name.name), "alignment of the struct".into(), String::new()));
                for f in &s.fields {
                    v.push(item(format!("{}.{}", s.name.name, f.name.name), "field offset".into(), String::new()));
                }
            }
            _ => {}
        }
    }
    v
}

/// The item `rest` (`square`, `msg.len`, `Point.size`) of another file, public or not.
pub fn find_item(m: &Module, rest: &str) -> Option<PubItem> {
    // Private items are found too (the assembler reports the privacy); build the list with
    // everything marked public.
    let mut all = m.clone();
    for i in &mut all.items {
        match i {
            Item::Func(f) => f.public = true,
            Item::Const(c) => c.public = true,
            Item::Global(g) => g.public = true,
            Item::Data(d) => d.public = true,
            Item::Struct(s) => s.public = true,
            _ => {}
        }
    }
    public_items(&all).into_iter().find(|p| p.name == rest)
}

// ---- events (efa-syntax 17) ----

fn event_form_text(f: &EventForm) -> String {
    match f {
        EventForm::FromIndex => String::new(),
        EventForm::NoPayload => "()".into(),
        EventForm::Struct(s, _) => format!("(&{})", s.name),
    }
}

fn event_md(ix: Option<&ApiIndex>, qualified: &str, declared: &str) -> String {
    let (ns, name) = qualified.rsplit_once('.').unwrap_or(("", qualified));
    if let Some(e) = ix.and_then(|ix| ix.event(ns, name)) {
        let payload = e.payload.as_ref().map(|p| format!("(&{p})")).unwrap_or_else(|| "()".into());
        return format!(
            "```\nevent {qualified}{payload}\n```\n\n{}\n\nid: `{qualified}.id`, owner: `{}`",
            e.doc, e.owner
        );
    }
    format!("```\nevent {qualified}{declared}\n```\n\nid: `{qualified}.id`")
}

/// Hover on an event: the name of an `import event` line, `ns.name.id`, or the name of a
/// `provide event` (payload and doc).
pub fn event_hover(ix: Option<&ApiIndex>, m: &Module, line: u32, word: &str) -> Option<String> {
    for i in &m.items {
        match i {
            Item::EventImport(d) => {
                let q = d.qualified.name.as_str();
                if (d.span.line == line && (word == q || word.strip_suffix(".id") == Some(q)))
                    || word.strip_suffix(".id") == Some(q)
                {
                    return Some(event_md(ix, q, &event_form_text(&d.form)));
                }
            }
            Item::ProvideEvent(p) if p.name.name == word => {
                let payload = p.payload.as_ref().map(|(s, _)| format!("&{}", s.name)).unwrap_or_default();
                let doc = String::from_utf8_lossy(&p.doc);
                return Some(format!("```\nprovide event {}({payload})\n```\n\n{doc}", p.name.name));
            }
            _ => {}
        }
    }
    None
}

/// Completion candidates after `import event `: (label, detail, doc).
pub fn event_candidates(ix: &ApiIndex) -> Vec<(String, String, String)> {
    ix.events
        .iter()
        .map(|e| {
            let payload = e.payload.as_ref().map(|p| format!("&{p}")).unwrap_or_else(|| "no payload".into());
            (format!("{}.{}", e.namespace, e.name), format!("event ({payload})"), e.doc.clone())
        })
        .collect()
}
