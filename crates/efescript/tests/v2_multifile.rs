//! Multi-file `.efa` programs and the mod interface (docs/efa-syntax.md section 16).

use std::cell::RefCell;
use std::rc::Rc;

use efescript::asm::{assemble, assemble_program, assemble_programs, assemble_with, AsmOptions, Diagnostic, MemSource};
use efescript::host_std::{std_registry, StdEnv, StdOptions};
use efescript::prelude::CaptureHost;
use efevm::api::{ApiIndex, ApiType, FieldDesc, NamespaceInfo, Prim, StructDesc};
use efevm::{Instance, Limits, Module};

fn src(files: &[(&str, &str)]) -> MemSource {
    files.iter().fold(MemSource::new(), |s, (p, t)| s.with(p, t))
}

fn build(files: &[(&str, &str)]) -> Result<Module, Vec<Diagnostic>> {
    assemble_program(files[0].0, &src(files), &AsmOptions::default()).map_err(|d| d.0)
}

fn ok(files: &[(&str, &str)]) -> Module {
    match build(files) {
        Ok(m) => m,
        Err(d) => panic!("assembly failed:\n{}", efescript::asm::Diagnostics(d)),
    }
}

fn errs(files: &[(&str, &str)]) -> Vec<Diagnostic> {
    match build(files) {
        Ok(_) => panic!("expected errors"),
        Err(d) => d,
    }
}

/// The diagnostic whose message contains `msg`.
fn find<'a>(d: &'a [Diagnostic], msg: &str) -> &'a Diagnostic {
    d.iter().find(|x| x.message.contains(msg)).unwrap_or_else(|| panic!("no diagnostic with {msg:?} in {d:#?}"))
}

fn run(m: &Module) -> (Vec<u64>, String) {
    let cap = CaptureHost::shared();
    let mut env = StdEnv::new(cap.clone());
    let imports = std_registry::<Rc<RefCell<CaptureHost>>>(&StdOptions::default()).imports();
    let mut inst = Instance::new(m, &imports, Limits::default()).expect("links");
    let r = inst.call("main", &[], &mut env, 1_000_000).expect("runs");
    let out = cap.borrow().out_string();
    (r, out)
}

const MAIN: &str = r#"use "util/math.efa" as m
import std.println(i64, i64)

export fn main() -> i64 {
    reg x, t
    call m.square(7) -> x
    i64.add x, x, m.LIMIT
    global.get t, m.counter
    i64.add x, x, t
    i64.add x, x, m.Pt.size
    i64.add x, x, m.Pt.y
    call std.println(m.msg, m.msg.len)
    ret x
}
"#;

const MATH: &str = r#"import std.println(i64, i64)
pub const LIMIT = 100
pub global mut counter: i64 = 1000
pub msg db "from math"

pub struct Pt {
    x: i32
    y: f64
}

pub fn square(n: i64) -> i64 {
    call helper()
    i64.mul n, n, n
    ret n
}

fn helper() {
    call std.println(note, note.len)
    ret
}

note db "helper"
"#;

#[test]
fn two_files_assemble_and_run() {
    let m = ok(&[("main.efa", MAIN), ("util/math.efa", MATH)]);
    let (r, out) = run(&m);
    // 49 + 100 + 1000 + 16 (size) + 8 (offset of y)
    assert_eq!(r, vec![1173]);
    assert_eq!(out, "helper\nfrom math\n");
    // One import for both files.
    assert_eq!(m.data().imports.len(), 1);
}

#[test]
fn names_and_lines_per_file() {
    let m = ok(&[("main.efa", MAIN), ("util/math.efa", MATH)]);
    let d = m.data();
    let names: Vec<&str> = d.names.funcs.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, vec!["std.println", "main", "util.math.square", "util.math.helper"]);
    assert_eq!(d.lines.files, vec!["main.efa".to_string(), "util/math.efa".to_string()]);
    // main's instructions map to file 0, square's to file 1.
    let f = |i: u32| d.lines.funcs.iter().find(|l| l.func == i).unwrap();
    assert!(f(1).entries.iter().all(|e| e.file == 0));
    assert!(f(2).entries.iter().all(|e| e.file == 1));
    // Data of both files is one layout: main has none, math's `msg` at 1024, `note` at 1040.
    let offs: Vec<u64> = d.data.iter().map(|s| s.offset).collect();
    assert_eq!(offs, vec![1024, 1040]);
}

#[test]
fn single_file_program_is_byte_identical() {
    let text = "import std.println(i64, i64)\nconst N = 3\ng0 db \"hi\", 10\nglobal mut c: i64 = N\ntable [main]\nexport fn main() -> i64 {\n    reg x\n    call std.println(g0, g0.len)\n    const.i64 x, N\n    ret x\n}\n";
    let a = assemble_with(text, "main.efa", &AsmOptions::default()).unwrap();
    let b = assemble_program("main.efa", &src(&[("main.efa", text)]), &AsmOptions::default()).unwrap();
    assert_eq!(a.encode(), b.encode());
    assert!(a.data().interface.is_none());
}

#[test]
fn default_alias_nested_paths_and_dotdot() {
    let main = "use \"lib/a.efa\"\nexport fn main() -> i64 {\n    reg x\n    call a.f() -> x\n    ret x\n}\n";
    // `lib/a.efa` uses `b.efa` relative to its own directory, and `../top.efa` back at the root.
    let a = "use \"b.efa\"\nuse \"../top.efa\" as t\npub fn f() -> i64 {\n    reg x, y\n    call b.g() -> x\n    i64.add x, x, t.ONE\n    ret x\n}\n";
    let b = "pub fn g() -> i64 {\n    ret 41\n}\n";
    let top = "pub const ONE = 1\n";
    let m = ok(&[("main.efa", main), ("lib/a.efa", a), ("lib/b.efa", b), ("top.efa", top)]);
    assert_eq!(run(&m).0, vec![42]);
    // File order is depth-first in `use` order.
    assert_eq!(m.data().lines.files, vec!["main.efa", "lib/a.efa", "lib/b.efa", "top.efa"]);
}

#[test]
fn use_path_errors() {
    let cases: &[(&str, &str)] = &[
        ("use \"/abs.efa\" as x", "is absolute"),
        ("use \"a\\\\b.efa\" as x", "use `/` as the separator"),
        ("use \"x.txt\" as x", "does not end in `.efa`"),
        ("use \"a//b.efa\" as x", "empty path segment"),
        ("use \"../out.efa\" as x", "leaves the root directory"),
        ("use \"missing.efa\"", "cannot read `missing.efa`"),
        ("use \"my-file.efa\" as x", "`my-file` is not an identifier"),
    ];
    for (line, msg) in cases {
        let text = format!("{line}\nexport fn main() {{\n    ret\n}}\n");
        let d = errs(&[("main.efa", &text), ("my-file.efa", "pub const A = 1\n")]);
        let e = find(&d, msg);
        assert_eq!((e.file.as_str(), e.line), ("main.efa", 1), "{line}");
    }
    // A file name that is not an identifier cannot be used (it is the namespace and the default alias).
    let d = errs(&[("main.efa", "use \"9lives.efa\"\nexport fn main() {\n    ret\n}\n"), ("9lives.efa", "")]);
    find(&d, "`9lives` is not an identifier");
}

#[test]
fn cycles_are_allowed() {
    let main = "use \"a.efa\"\nexport fn main() -> i64 {\n    reg x\n    call a.even(10) -> x\n    ret x\n}\n";
    let a = "use \"b.efa\"\npub fn even(n: i64) -> i64 {\n    reg t\n    i64.eqz t, n\n    br_if t, yes\n    i64.sub n, n, 1\n    call b.odd(n) -> n\n    ret n\nyes:\n    ret 1\n}\n";
    let b = "use \"a.efa\"\npub fn odd(n: i64) -> i64 {\n    reg t\n    i64.eqz t, n\n    br_if t, no\n    i64.sub n, n, 1\n    call a.even(n) -> n\n    ret n\nno:\n    ret 0\n}\n";
    let m = ok(&[("main.efa", main), ("a.efa", a), ("b.efa", b)]);
    assert_eq!(run(&m).0, vec![1]);
    // A file may use the root and itself; each file is assembled once.
    let main2 = "use \"main.efa\" as me\nuse \"a.efa\"\nuse \"a.efa\" as again\npub const K = 5\nexport fn main() -> i64 {\n    reg x\n    call again.even(3) -> x\n    i64.add x, x, me.K\n    ret x\n}\n";
    let m = ok(&[("main.efa", main2), ("a.efa", a), ("b.efa", b)]);
    assert_eq!(run(&m).0, vec![5]);
    assert_eq!(m.data().lines.files.len(), 3);
}

#[test]
fn alias_conflicts() {
    let other = ("x.efa", "pub const A = 1\n");
    let base = |head: &str| format!("{head}\nexport fn main() {{\n    ret\n}}\n");
    let d = errs(&[("main.efa", &base("use \"x.efa\"\nuse \"x.efa\" as x")), other]);
    assert_eq!(find(&d, "duplicate alias `x`").note.as_deref(), Some("first defined at line 1"));
    let d = errs(&[("main.efa", &base("use \"x.efa\" as K\nconst K = 1")), other]);
    find(&d, "alias `K` conflicts with a constant");
    let d = errs(&[("main.efa", &base("use \"x.efa\" as std\nimport std.println(i64, i64)")), other]);
    find(&d, "alias `std` conflicts with the first segment of an imported namespace");
    let d = errs(&[("main.efa", &base("use \"x.efa\" as main")), other]);
    find(&d, "alias `main` conflicts with a function");
    let d = errs(&[("main.efa", &base("use \"x.efa\" as S\nstruct S {\n    a: u8\n}")), other]);
    find(&d, "alias `S` conflicts with a struct");
    let d = errs(&[("main.efa", &base("use \"x.efa\" as g\nglobal g: i64 = 0")), other]);
    find(&d, "alias `g` conflicts with a global");
    let d = errs(&[("main.efa", &base("use \"x.efa\" as buf\nbuf resb 8")), other]);
    find(&d, "alias `buf` conflicts with a data item");
}

#[test]
fn private_items_are_refused_with_a_note() {
    let lib = "fn hidden() {\n    ret\n}\nglobal g: i64 = 0\nconst C = 1\nd db 1\nstruct S {\n    a: u8\n}\n";
    let uses: &[(&str, &str)] = &[
        ("call l.hidden()", "`hidden` is private to `l.efa`"),
        ("global.get x, l.g", "`g` is private to `l.efa`"),
        ("const.i64 x, l.C", "`C` is private to `l.efa`"),
        ("const.i64 x, l.d", "`d` is private to `l.efa`"),
        ("const.i64 x, l.d.len", "`d.len` is private to `l.efa`"),
        ("i64.add x, x, l.S.size", "`S.size` is private to `l.efa`"),
    ];
    for (stmt, msg) in uses {
        let main = format!("use \"l.efa\"\nexport fn main() {{\n    reg x\n    {stmt}\n    ret\n}}\n");
        let d = errs(&[("main.efa", &main), ("l.efa", lib)]);
        assert_eq!(d.len(), 1, "{d:#?}");
        let e = find(&d, msg);
        assert_eq!((e.file.as_str(), e.line), ("main.efa", 4));
        assert!(e.note.as_deref().is_some_and(|n| n.starts_with("declared at l.efa:")), "{:?}", e.note);
    }
    // An undotted name never finds another file's item.
    let main = "use \"l.efa\"\nexport fn main() {\n    call shared()\n    ret\n}\n";
    let d = errs(&[("main.efa", main), ("l.efa", "pub fn shared() {\n    ret\n}\n")]);
    find(&d, "unknown function `shared`");
}

#[test]
fn export_is_public_and_table_entries_may_be_qualified() {
    let main = "use \"l.efa\"\ntable [l.f, null]\nexport fn main() -> i64 {\n    reg x, t\n    const.i32 x, 0\n    call_indirect () -> i64, x() -> x\n    global.get t, l.counter\n    i64.add x, x, t\n    ret x\n}\n";
    let lib = "export fn f() -> i64 {\n    ret 5\n}\nexport global counter: i64 = 2\n";
    let m = ok(&[("main.efa", main), ("l.efa", lib)]);
    assert!(m.export_func("f").is_some());
    assert_eq!(run(&m).0, vec![7]);
}

#[test]
fn imports_merge_and_conflict() {
    let main = "use \"b.efa\"\nimport std.println(i64, i64)\nexport fn main() {\n    call b.f()\n    ret\n}\n";
    let b = "import std.print_i64(i64)\nimport std.println(i64, i64)\npub fn f() {\n    call std.print_i64(3)\n    ret\n}\n";
    let m = ok(&[("main.efa", main), ("b.efa", b)]);
    let imps: Vec<String> = m.data().imports.iter().map(|i| format!("{}.{}", i.namespace, i.name)).collect();
    assert_eq!(imps, vec!["std.println", "std.print_i64"]);
    assert_eq!(run(&m).1, "3");
    let bad = "import std.println(i64)\npub fn f() {\n    ret\n}\n";
    let d = errs(&[("main.efa", main), ("b.efa", bad)]);
    let e = find(&d, "import `std.println` is declared as (i64) here but as (i64, i64) in `main.efa`");
    assert_eq!(e.file, "b.efa");
    assert_eq!(e.note.as_deref(), Some("first defined in main.efa:2"));
}

#[test]
fn export_and_provide_names_are_unique_in_the_program() {
    let main = "use \"b.efa\"\nexport fn run() {\n    ret\n}\nexport fn main() {\n    ret\n}\nprovide p() = main, \"doc\"\n";
    let b = "export global run: i64 = 0\nfn q() {\n    ret\n}\nprovide p() = q, \"doc\"\n";
    let d = errs(&[("main.efa", main), ("b.efa", b)]);
    let e = find(&d, "duplicate export name `run`");
    assert_eq!((e.file.as_str(), e.note.as_deref()), ("b.efa", Some("first defined in main.efa:2")));
    let e = find(&d, "duplicate provide name `p`");
    assert_eq!(e.file, "b.efa");
}

#[test]
fn memory_and_scratch_only_in_root_and_one_table() {
    let main = "use \"b.efa\"\ntable [main]\nexport fn main() {\n    ret\n}\n";
    let b = "memory 2\nscratch 64\ntable [f]\nfn f() {\n    ret\n}\n";
    let d = errs(&[("main.efa", main), ("b.efa", b)]);
    find(&d, "`memory` may only appear in the root file `main.efa`");
    find(&d, "`scratch` may only appear in the root file `main.efa`");
    let e = find(&d, "duplicate `table` declaration");
    assert_eq!((e.file.as_str(), e.note.as_deref()), ("b.efa", Some("first defined in main.efa:2")));
}

#[test]
fn pub_is_refused_where_it_means_nothing() {
    for item in ["pub import std.println(i64, i64)", "pub use \"x.efa\"", "pub memory 1", "pub table [main]", "pub scratch 8", "pub provide p() = main, \"d\""] {
        let text = format!("{item}\nexport fn main() {{\n    ret\n}}\n");
        let d = assemble(&text, "t.efa").unwrap_err().0;
        assert!(d[0].message.starts_with("`pub` is not allowed on"), "{item}: {:?}", d);
    }
}

#[test]
fn reserved_words() {
    for w in ["use", "as", "pub", "provide", "struct", "scratch"] {
        let text = format!("export fn main() {{\n    reg {w}\n    ret\n}}\n");
        let d = assemble(&text, "t.efa").unwrap_err().0;
        assert!(d[0].message.contains("is a reserved word"), "{w}: {:?}", d);
        let text = format!("export fn main() {{\n{w}:\n    ret\n}}\n");
        let d = assemble(&text, "t.efa").unwrap_err().0;
        assert!(d[0].message.contains("is a reserved word"), "{w}: {:?}", d);
    }
    for w in ["use", "pub", "provide", "struct", "scratch"] {
        let d = assemble(&format!("{w} db 1\n"), "t.efa").unwrap_err().0;
        assert_eq!(d[0].message, format!("`{w}` is a reserved word and cannot be a data name"));
    }
    let d = assemble("as db 1\n", "t.efa").unwrap_err().0;
    assert!(d[0].message.contains("reserved word"), "{:?}", d);
}

const PROVIDER: &str = r#"use "items.efa"
provide give(item: str, count: i32) -> (status: i32) = items.add,
    "Add `count` of `item` to the bag; 0 ok, 1 bag full."
provide total() -> (n: i64), "How many items the bag holds."
provide put(slot: &Slot, ok: bool) = put, "Store a slot."

export struct Slot, "One stack." {
    id: u32
    count: i32, "How many."
    pos: f32[2]
    owner: entity
}

fn total() -> i64 {
    ret 0
}

fn put(s: i64, ok: i32) {
    ret
}

hello db "ready"

export fn main() {
    ret
}
"#;

const ITEMS: &str = r#"pub fn add(name: i64, len: i64, count: i32) -> i32 {
    ret 0
}
"#;

#[test]
fn provides_structs_and_scratch_in_section_13() {
    let m = ok(&[("main.efa", PROVIDER), ("items.efa", ITEMS)]);
    let iface = m.data().interface.as_ref().expect("section 13");
    let names: Vec<&str> = iface.provides.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["give", "total", "put"]);
    let give = &iface.provides[0];
    assert_eq!(give.func, m.data().names.funcs.iter().find(|(_, n)| n == "items.add").unwrap().0);
    assert_eq!(give.params[0].ty, ApiType::Str);
    assert_eq!(give.results[0].name, "status");
    assert_eq!(iface.provides[2].params[0].ty, ApiType::Struct("Slot".into()));
    let slot = iface.struct_desc("Slot").unwrap();
    assert_eq!((slot.size, slot.align, slot.owner.as_str()), (24, 8, ""));
    let offs: Vec<(&str, u32, u32)> = slot.fields.iter().map(|f| (f.name.as_str(), f.offset, f.count)).collect();
    assert_eq!(offs, vec![("id", 0, 1), ("count", 4, 1), ("pos", 8, 2), ("owner", 16, 1)]);
    assert_eq!(slot.fields[1].doc, "How many.");
    assert_eq!(slot.doc, "One stack.");
    // Default scratch: 4096 bytes after the data (`hello`, 5 bytes at 1024), 8-aligned.
    assert_eq!(iface.scratch, Some((1032, 4096)));
    // The index of the mod is derived from it.
    let ix = iface.index("bag", 1, "A bag.");
    assert!(ix.function("bag", "give").is_some());
}

#[test]
fn scratch_rules() {
    let prov = |head: &str, params: &str| {
        format!("{head}\nprovide p({params}), \"d\"\nfn p({}) {{\n    ret\n}}\n", if params.is_empty() { "" } else { "a: i64, b: i64" })
    };
    // Scalars only: no scratch region, but section 13 exists for the provide.
    let m = ok(&[("main.efa", &prov("", ""))]);
    assert_eq!(m.data().interface.as_ref().unwrap().scratch, None);
    // Explicit size, data-less program: at 1024.
    let m = ok(&[("main.efa", &prov("scratch 100", "s: str"))]);
    assert_eq!(m.data().interface.as_ref().unwrap().scratch, Some((1024, 100)));
    // `scratch 0` with a provide that needs one.
    let d = errs(&[("main.efa", &prov("scratch 0", "s: str"))]);
    find(&d, "`scratch 0` leaves no room");
    // Limit.
    let d = errs(&[("main.efa", &prov("scratch 1048577", "s: str"))]);
    find(&d, "scratch size 1048577 is out of range");
    // `scratch 0` without such a provide: no region, and without a provide no section 13.
    let m = ok(&[("main.efa", "scratch 0\nexport fn main() {\n    ret\n}\n")]);
    assert!(m.data().interface.is_none());
    // The region counts toward the default memory.
    let m = ok(&[("main.efa", &prov("scratch 1048576", "s: str"))]);
    assert_eq!(m.data().memory.min_pages, 17);
    // ... and must fit an explicit one.
    let d = errs(&[("main.efa", &prov("memory 1\nscratch 65536", "s: str"))]);
    let e = find(&d, "the data needs 66560 bytes but `memory 1` provides only 65536");
    assert_eq!(e.note.as_deref(), Some("the scratch region of 65536 bytes follows the data"));
}

#[test]
fn provide_signature_checks() {
    let one = |text: &str| errs(&[("main.efa", text)]);
    let d = one("provide p(a: i32) -> (r: i64), \"d\"\nfn p(a: i64) -> i64 {\n    ret a\n}\n");
    let e = find(&d, "provide `p` lowers to `(i32) -> i64`, but its function `p` has `(i64) -> i64`");
    assert_eq!(e.line, 1);
    let d = one("provide p() -> (r: str), \"d\"\nfn p() -> (i64, i64) {\n    ret (0, 0)\n}\n");
    find(&d, "result `r` of provide `p` must be a scalar");
    let d = one("provide p(a: ptr), \"d\"\nfn p(a: i64) {\n    ret\n}\n");
    find(&d, "`ptr` cannot be used in provide `p`");
    let d = one("provide p(), \"   \"\nfn p() {\n    ret\n}\n");
    find(&d, "the doc of provide `p` must not be blank");
    let d = one("provide p(a: &S), \"d\"\npub struct S {\n    x: u8\n}\nfn p(a: i64) {\n    ret\n}\n");
    find(&d, "struct `S` is used by provide `p` and must be `export`");
    let d = one("provide p(a: &Nope), \"d\"\nfn p(a: i64) {\n    ret\n}\n");
    find(&d, "unknown struct `Nope`");
    let d = one("provide p(a: wat), \"d\"\nfn p(a: i64) {\n    ret\n}\n");
    find(&d, "unknown provide type `wat`");
    let d = one("provide p(a: i64, a: i64), \"d\"\nfn p(a: i64, b: i64) {\n    ret\n}\n");
    find(&d, "duplicate parameter `a` in provide `p`");
    let d = one("provide p(), \"d\"\n");
    find(&d, "unknown function `p`");
    // Across files: the function must be public; its struct too.
    let d = errs(&[("main.efa", "use \"l.efa\"\nprovide p() = l.f, \"d\"\n"), ("l.efa", "fn f() {\n    ret\n}\n")]);
    find(&d, "`f` is private to `l.efa`");
    let lib = "export struct T {\n    a: i64\n}\npub fn f(t: i64) {\n    ret\n}\n";
    let m = ok(&[("main.efa", "use \"l.efa\"\nprovide p(t: &l.T) = l.f, \"d\"\n"), ("l.efa", lib)]);
    let iface = m.data().interface.as_ref().unwrap();
    assert_eq!(iface.provides[0].params[0].ty, ApiType::Struct("T".into()));
    assert!(iface.struct_desc("T").is_some());
    // Exported struct names are unique in the program.
    let d = errs(&[("main.efa", "use \"l.efa\"\nexport struct T {\n    a: u8\n}\n"), ("l.efa", lib)]);
    find(&d, "duplicate exported struct `T`");
}

#[test]
fn struct_layout_and_constants() {
    let text = "struct S {\n    a: u8\n    b: u16\n    c: u8[3]\n    size: u64\n}\nconst N = 2\nstruct V {\n    xs: f32[N]\n}\nexport fn main() -> i64 {\n    reg x\n    const.i64 x, S.size\n    i64.add x, x, S.b\n    i64.add x, x, S.c\n    i64.add x, x, S.align\n    i64.add x, x, V.size\n    ret x\n}\n";
    let m = assemble(text, "t.efa").unwrap();
    // S: a@0, b@2, c@4..7, size@8 (the field loses to the structural constant), size 16, align 8.
    assert_eq!(run(&m).0, vec![16 + 2 + 4 + 8 + 8]);
    // A plain struct alone does not create section 13.
    assert!(m.data().interface.is_none());
    for (bad, msg) in [
        ("struct E {\n}\n", "needs at least one field"),
        ("struct E {\n    a: u7\n}\n", "unknown field type `u7`"),
        ("struct E {\n    a: u8[0]\n}\n", "array length 0 must be at least 1"),
        ("struct E {\n    a: u8\n    a: u8\n}\n", "duplicate field `a`"),
        ("struct E {\n    a: u8\n}\nE db 1\n", "duplicate definition of `E`"),
    ] {
        let d = assemble(bad, "t.efa").unwrap_err().0;
        assert!(d.iter().any(|x| x.message.contains(msg)), "{bad}: {d:?}");
    }
}

fn game_index() -> ApiIndex {
    let mut t = StructDesc::new("Transform", 12, 4).field(FieldDesc::new("pos", 0, Prim::F32, 3));
    t.owner = "engine".into();
    t.component = Some(3);
    ApiIndex {
        game: "game".into(),
        namespaces: vec![NamespaceInfo { name: "engine".into(), version: 1, owner: "engine".into(), doc: "e".into() }],
        types: vec![t],
        functions: vec![],
        ..Default::default()
    }
}

#[test]
fn index_structs_are_copied_with_their_owner() {
    let ix = game_index();
    let text = "export struct Own {\n    a: i32\n}\nprovide mv(t: &Transform, o: &Own), \"Move.\"\nfn mv(t: i64, o: i64) {\n    ret\n}\n";
    let m = assemble_with(text, "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).unwrap();
    let iface = m.data().interface.as_ref().unwrap();
    let names: Vec<(&str, &str)> = iface.structs.iter().map(|s| (s.name.as_str(), s.owner.as_str())).collect();
    assert_eq!(names, vec![("Own", ""), ("Transform", "engine")]);
    assert_eq!(iface.struct_desc("Transform").unwrap().component, Some(3));
    // A local struct may not shadow an index struct.
    let d = assemble_with("struct Transform {\n    a: u8\n}\n", "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).unwrap_err().0;
    assert!(d[0].message.contains("same name as a struct of the API index"));
}

#[test]
fn several_roots_link_into_one_module() {
    let a = "export fn main() -> i64 {\n    ret 1\n}\n";
    let b = "export fn other() -> i64 {\n    ret 2\n}\n";
    let s = src(&[("dir/a.efa", a), ("dir/b.efa", b)]);
    let m = assemble_programs(&["dir/a.efa", "dir/b.efa"], &s, &AsmOptions::default()).unwrap();
    assert!(m.export_func("other").is_some());
    let names: Vec<&str> = m.data().names.funcs.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, vec!["main", "b.other"]);
}

#[test]
fn diagnostics_are_sorted_by_file_order() {
    let main = "use \"b.efa\"\nexport fn main() {\n    i64.ad r0, r0, r0\n    ret\n}\n";
    let b = "fn f() {\n    nop x\n    ret\n}\n";
    let d = errs(&[("main.efa", main), ("b.efa", b)]);
    let files: Vec<&str> = d.iter().map(|x| x.file.as_str()).collect();
    assert_eq!(files, vec!["main.efa", "b.efa"]);
    // Syntax errors of every file are reported before anything is linked.
    let d = errs(&[("main.efa", "use \"b.efa\"\nfn (\n"), ("b.efa", "fn (\n")]);
    assert_eq!(d.len(), 2);
}

#[cfg(unix)]
#[test]
fn fs_source_follows_use_and_refuses_escapes() {
    use efescript::asm::{FileSource, FsSource};
    let dir = std::env::temp_dir().join(format!("efa_multifile_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj/util")).unwrap();
    std::fs::write(dir.join("proj/main.efa"), MAIN).unwrap();
    std::fs::write(dir.join("proj/util/math.efa"), MATH).unwrap();
    std::fs::write(dir.join("outside.efa"), "pub const A = 1\n").unwrap();
    std::os::unix::fs::symlink(dir.join("outside.efa"), dir.join("proj/link.efa")).unwrap();
    let root = format!("{}/proj/main.efa", dir.display());
    let (fs, file) = FsSource::for_file(&root);
    let m = assemble_program(&file, &fs, &AsmOptions::default()).unwrap();
    assert_eq!(run(&m).0, vec![1173]);
    // The root keeps its name as typed; the used file is shown below the same directory.
    assert_eq!(m.data().lines.files[0], root);
    assert_eq!(m.data().lines.files[1], format!("{}/proj/util/math.efa", dir.display()));
    // A symlink out of the root is refused.
    let e = fs.read("link.efa").unwrap_err();
    assert!(e.contains("outside the root"), "{e}");
    assert!(fs.read("../outside.efa").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
