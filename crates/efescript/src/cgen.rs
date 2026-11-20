//! C bindings from an API index: `efescript api-export --c`.
//!
//! One header, `efe_api.h`, for C and C++ guests built with clang for `wasm32` (also usable
//! from Zig through `@cImport`). It follows the wasm ABI of `docs/e7.4-wasm-design.md` and is
//! built on the same model as the Rust backend (`bindmodel`):
//!
//! - `typedef struct { uint64_t bits; } efe_entity;` (passed as one `i64`, the wasm C ABI of a
//!   single-field struct);
//! - one `efe_<Name>` struct per layout with static asserts (`EFE_STATIC_ASSERT`) on size, alignment and offsets;
//! - every function is a declaration with
//!   `__attribute__((import_module("<ns>@<ver>"), import_name("<name>")))` named
//!   `<namespace with dots as underscores>_<name>`; `str` and `bytes` are `(ptr, len)` pairs
//!   (`<param>_ptr`, `<param>_len`);
//! - events are `<ns>_<event>_send`, `_count` and `_read`, imported as `<event>.send` etc.;
//! - the `efe.api` custom section (`.custom_section.efe.api`) is defined in the one translation
//!   unit that defines `EFE_API_IMPLEMENTATION` before including the header.

use crate::bindmodel::{build_tree, err, layout, single_result, BindError, Node};
use efevm::api::{ApiIndex, ApiType, EventDesc, HostFn, Prim, StructDesc};
use std::collections::BTreeSet;
use std::fmt::Write;

/// C and C++ keywords: a parameter or field with one of these names gets a trailing `_`.
const KEYWORDS: &[&str] = &[
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor", "bool", "break", "case", "catch", "char", "char8_t",
    "char16_t", "char32_t", "class", "compl", "concept", "const", "consteval", "constexpr", "constinit", "const_cast", "continue",
    "co_await", "co_return", "co_yield", "decltype", "default", "delete", "do", "double", "dynamic_cast", "else", "enum",
    "explicit", "export", "extern", "false", "float", "for", "friend", "goto", "if", "inline", "int", "long", "mutable",
    "namespace", "new", "noexcept", "not", "not_eq", "nullptr", "operator", "or", "or_eq", "private", "protected", "public",
    "register", "reinterpret_cast", "requires", "restrict", "return", "short", "signed", "sizeof", "static", "static_assert",
    "static_cast", "struct", "switch", "template", "this", "thread_local", "throw", "true", "try", "typedef", "typeid",
    "typename", "typeof", "union", "unsigned", "using", "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
    "NULL", "EOF", "errno", "offsetof", "assert", "stdin", "stdout", "stderr",
];

/// A local name (parameter, field): `_`-suffixed when it is a keyword.
fn local(name: &str) -> String {
    if KEYWORDS.contains(&name) { format!("{name}_") } else { name.to_string() }
}

/// A doc as a `/** ... */` comment.
fn doc(out: &mut String, text: &str, extra: &[String]) {
    let mut lines: Vec<String> = text.lines().map(|l| l.trim().replace("*/", "* /").replace("/*", "/ *")).collect();
    lines.extend(extra.iter().cloned());
    match lines.as_slice() {
        [] => {}
        [one] => {
            let _ = writeln!(out, "/** {one} */");
        }
        many => {
            let _ = writeln!(out, "/**");
            for l in many {
                let _ = writeln!(out, "{}", format!(" * {l}").trim_end());
            }
            let _ = writeln!(out, " */");
        }
    }
}

fn prim_c(p: Prim) -> &'static str {
    match p {
        Prim::U8 => "uint8_t",
        Prim::I8 => "int8_t",
        Prim::U16 => "uint16_t",
        Prim::I16 => "int16_t",
        Prim::U32 => "uint32_t",
        Prim::I32 => "int32_t",
        Prim::U64 => "uint64_t",
        Prim::I64 => "int64_t",
        Prim::F32 => "float",
        Prim::F64 => "double",
        Prim::Bool => "bool",
        Prim::Entity => "efe_entity",
    }
}

fn struct_c(name: &str) -> Result<String, BindError> {
    if name.starts_with("mod:") {
        return err(format!("struct `{name}` is a mod-defined component and has no C name"));
    }
    Ok(format!("efe_{name}"))
}

fn scalar_c(t: &ApiType) -> &'static str {
    match t {
        ApiType::I32 => "int32_t",
        ApiType::I64 => "int64_t",
        ApiType::F32 => "float",
        ApiType::F64 => "double",
        ApiType::Bool => "bool",
        ApiType::Entity => "efe_entity",
        ApiType::Ptr => "void*",
        _ => unreachable!("not a scalar"),
    }
}

/// Generates the C header for `ix`.
pub fn generate(ix: &ApiIndex) -> Result<String, BindError> {
    let root = build_tree(ix)?;
    let mut g = Gen { ix, out: String::new(), names: BTreeSet::new() };
    g.run(&root)?;
    Ok(g.out)
}

struct Gen<'a> {
    ix: &'a ApiIndex,
    out: String,
    /// Every global C name emitted so far (types, functions, macros).
    names: BTreeSet<String>,
}

impl<'a> Gen<'a> {
    fn claim(&mut self, name: &str, what: &str) -> Result<(), BindError> {
        if !self.names.insert(name.to_string()) {
            return err(format!("{what}: the C name `{name}` is already taken by another item; rename one of them in the index"));
        }
        Ok(())
    }

    fn run(&mut self, root: &Node<'a>) -> Result<(), BindError> {
        let ix = self.ix;
        let hash = ix.hash();
        let game = if ix.game.is_empty() { "(unnamed)" } else { ix.game.as_str() };
        for n in ["efe_entity", "efe_api_hash_bytes", "EFE_API_GAME", "EFE_API_HASH", "EFE_API_H"] {
            self.names.insert(n.to_string());
        }
        let _ = writeln!(self.out, "/* @generated by `efescript api-export --c`. Do not edit: regenerate it instead.");
        let _ = writeln!(self.out, " * API index `{game}`, hash fnv1a64:{hash:016x}.");
        let _ = writeln!(self.out, " *");
        let _ = writeln!(self.out, " * C and C++ bindings for guests built with clang for wasm32 (docs/e7.4-wasm-design.md).");
        let _ = writeln!(self.out, " * Define EFE_API_IMPLEMENTATION in exactly one translation unit before including this");
        let _ = writeln!(self.out, " * header to emit the `efe.api` custom section there. */");
        let _ = writeln!(self.out, "#ifndef EFE_API_H");
        let _ = writeln!(self.out, "#define EFE_API_H");
        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>");
        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "#ifdef __cplusplus\nextern \"C\" {{\n#define EFE_STATIC_ASSERT static_assert\n#define EFE_ALIGNOF alignof\n#else\n#define EFE_STATIC_ASSERT _Static_assert\n#define EFE_ALIGNOF _Alignof\n#endif");
        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "/** The name of the index this header was generated from. */");
        let _ = writeln!(self.out, "#define EFE_API_GAME {game:?}");
        let _ = writeln!(self.out, "/** The FNV-1a hash of the index (also the `efe.api` custom section). */");
        let _ = writeln!(self.out, "#define EFE_API_HASH 0x{hash:016x}ULL");
        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "/** An entity handle; passed to and from the host as one i64. */");
        let _ = writeln!(self.out, "typedef struct {{ uint64_t bits; }} efe_entity;");

        for t in &ix.types {
            if t.name.starts_with("mod:") {
                let _ = writeln!(self.out, "\n/* Skipped: `{}` is a mod-defined component, not usable in signatures. */", t.name);
                continue;
            }
            self.strukt(t)?;
        }
        self.node(root, "")?;

        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "#ifdef EFE_API_IMPLEMENTATION");
        let _ = writeln!(self.out, "/* The `efe.api` custom section: the index hash, 8 bytes little-endian. */");
        let bytes = hash.to_le_bytes().iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", ");
        let _ = writeln!(self.out, "__attribute__((section(\".custom_section.efe.api\"), used))");
        let _ = writeln!(self.out, "static const uint8_t efe_api_hash_bytes[8] = {{ {bytes} }};");
        let _ = writeln!(self.out, "#endif");
        let _ = writeln!(self.out);
        let _ = writeln!(self.out, "#ifdef __cplusplus\n}}\n#endif\n");
        let _ = writeln!(self.out, "#endif /* EFE_API_H */");
        Ok(())
    }

    fn strukt(&mut self, t: &StructDesc) -> Result<(), BindError> {
        let name = struct_c(&t.name)?;
        self.claim(&name, &format!("struct `{}`", t.name))?;
        let lay = layout(t)?;
        let mut seen = BTreeSet::new();
        for f in &lay.fields {
            if !seen.insert(local(&f.name)) {
                return err(format!("struct `{}`: field `{}` collides with another field after keyword escaping", t.name, f.name));
            }
        }
        let o = &mut self.out;
        let _ = writeln!(o);
        let comp = t.component.map(|c| format!(", component id {c}")).unwrap_or_default();
        let head = if t.doc.is_empty() { format!("Layout `{}` of the API index.", t.name) } else { t.doc.clone() };
        doc(o, &head, &[format!("Size {}, alignment {}, owner `{}`{comp}.", t.size, t.align, t.owner)]);
        let attr = if lay.natural_align < t.align { format!(" __attribute__((aligned({})))", t.align) } else { String::new() };
        let _ = writeln!(o, "typedef struct {name} {{");
        for f in &lay.fields {
            let n = local(&f.name);
            match f.ty {
                None => {
                    let _ = writeln!(o, "    uint8_t {n}[{}];", f.count);
                }
                Some(p) => {
                    let arr = if f.count > 1 { format!("[{}]", f.count) } else { String::new() };
                    let d = if f.doc.is_empty() { String::new() } else { format!(" /**< {} */", f.doc.trim().replace("*/", "* /").replace('\n', " ")) };
                    let _ = writeln!(o, "    {} {n}{arr};{d}", prim_c(p));
                }
            }
        }
        let _ = writeln!(o, "}} {name}{attr};");
        let _ = writeln!(o, "EFE_STATIC_ASSERT(sizeof({name}) == {}, \"{name}: size differs from the API index\");", t.size);
        let _ = writeln!(o, "EFE_STATIC_ASSERT(EFE_ALIGNOF({name}) == {}, \"{name}: alignment differs from the API index\");", t.align);
        for f in lay.fields.iter().filter(|f| !f.is_pad()) {
            let _ = writeln!(o, "EFE_STATIC_ASSERT(offsetof({name}, {}) == {}, \"{name}.{}: offset differs from the API index\");", local(&f.name), f.offset, f.name);
        }
        if let Some(c) = t.component {
            let m = format!("EFE_COMPONENT_{}", t.name);
            self.claim(&m, &format!("struct `{}`", t.name))?;
            let _ = writeln!(self.out, "/** The component id scripts pass to `engine.world.*`. */\n#define {m} {c}u");
        }
        Ok(())
    }

    fn node(&mut self, n: &Node<'a>, path: &str) -> Result<(), BindError> {
        if let Some(i) = n.info {
            let prefix = i.name.replace('.', "_");
            let o = &mut self.out;
            let _ = writeln!(o);
            let d = if i.doc.is_empty() { format!("Namespace `{}`.", i.name) } else { i.doc.clone() };
            doc(o, &d, &[]);
            let m = format!("{}_VERSION", prefix.to_uppercase());
            let _ = writeln!(o, "#define {m} {}", i.version);
            self.claim(&m, &format!("namespace `{}`", i.name))?;
            for f in &n.funcs {
                self.function(f, i.version)?;
            }
            for e in &n.events {
                self.event(e, i.version)?;
            }
        }
        for (name, child) in &n.children {
            let p = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
            self.node(child, &p)?;
        }
        Ok(())
    }

    fn function(&mut self, f: &HostFn, version: u32) -> Result<(), BindError> {
        let q = f.qualified();
        let cname = format!("{}_{}", f.namespace.replace('.', "_"), f.name);
        self.claim(&cname, &q)?;
        let mut params: Vec<String> = Vec::new();
        let mut names = BTreeSet::new();
        for p in &f.params {
            let n = local(&p.name);
            let b = p.name.as_str();
            let (decl, used): (Vec<String>, Vec<String>) = match &p.ty {
                ApiType::Str => (vec![format!("const char* {b}_ptr"), format!("size_t {b}_len")], vec![format!("{b}_ptr"), format!("{b}_len")]),
                ApiType::Bytes => (vec![format!("const uint8_t* {b}_ptr"), format!("size_t {b}_len")], vec![format!("{b}_ptr"), format!("{b}_len")]),
                ApiType::Struct(s) => (vec![format!("const {}* {n}", struct_c(s)?)], vec![n.clone()]),
                t => (vec![format!("{} {n}", scalar_c(t))], vec![n.clone()]),
            };
            for u in used {
                if !names.insert(u.clone()) {
                    return err(format!("{q}: parameter names collide after lowering (`{u}`)"));
                }
            }
            params.extend(decl);
        }
        let ret = single_result(f)?.map(scalar_c).unwrap_or("void");
        let o = &mut self.out;
        let _ = writeln!(o);
        doc(o, &f.doc, &[format!("Index signature: `{}`.", f.signature_text())]);
        let _ = writeln!(o, "__attribute__((import_module(\"{}@{version}\"), import_name({:?})))", f.namespace, f.name);
        let plist = if params.is_empty() { "void".to_string() } else { params.join(", ") };
        let _ = writeln!(o, "{ret} {cname}({plist});");
        Ok(())
    }

    fn event(&mut self, e: &EventDesc, version: u32) -> Result<(), BindError> {
        let q = e.qualified();
        let base = format!("{}_{}", e.namespace.replace('.', "_"), e.name);
        let payload = match &e.payload {
            Some(p) => Some(struct_c(p)?),
            None => None,
        };
        let send = format!("{base}_send");
        let count = format!("{base}_count");
        let read = format!("{base}_read");
        self.claim(&send, &format!("event `{q}`"))?;
        self.claim(&count, &format!("event `{q}`"))?;
        let ns = &e.namespace;
        let o = &mut self.out;
        let _ = writeln!(o);
        doc(o, &e.doc, &[format!("Event `{q}`; imports `{}.send`, `{}.count`{} of `{ns}@{version}`.", e.name, e.name, if payload.is_some() { ", `.read`" } else { "" })]);
        let imp = |o: &mut String, op: &str| {
            let _ = writeln!(o, "__attribute__((import_module(\"{ns}@{version}\"), import_name(\"{}.{op}\")))", e.name);
        };
        imp(o, "send");
        match &payload {
            Some(p) => {
                let _ = writeln!(o, "void {send}(const {p}* payload); /**< queues the event; readable next frame */");
            }
            None => {
                let _ = writeln!(o, "void {send}(void); /**< queues the event; readable next frame */");
            }
        }
        imp(o, "count");
        let _ = writeln!(o, "int32_t {count}(void); /**< events readable this frame */");
        if let Some(p) = &payload {
            self.claim(&read, &format!("event `{q}`"))?;
            let o = &mut self.out;
            imp(o, "read");
            let _ = writeln!(o, "int32_t {read}(uint32_t index, {p}* out); /**< payload of the index-th event; 0 ok, -1 none */");
        }
        Ok(())
    }
}
