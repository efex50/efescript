//! E8.13: typed script events in `.efa` (docs/efa-syntax.md section 17).

use efescript::asm::{assemble_program, assemble_with, AsmOptions, Diagnostic, MemSource};
use efevm::api::{ApiIndex, ApiType, FieldDesc, HostFn, HostRegistry, Prim, StructDesc};
use efevm::{EventDesc, HostOk, Module};

/// An index with `game.score_changed(&Score)`, `game.reset()`, `game.lives(&Lives)`,
/// `engine.events.send`, the struct `Score` (8 bytes) and `Lives` (4 bytes).
fn index() -> ApiIndex {
    let mut r: HostRegistry<()> = HostRegistry::new();
    r.set_game("demo");
    r.define_namespace("game", 3, "demo", "The game.").unwrap();
    r.define_namespace("engine.events", 1, "engine", "Events.").unwrap();
    r.register_type(
        StructDesc::new("Score", 8, 4)
            .field(FieldDesc::new("points", 0, Prim::I32, 1))
            .field(FieldDesc::new("delta", 4, Prim::I32, 1))
            .doc("A score."),
    )
    .unwrap();
    r.register_type(StructDesc::new("Lives", 4, 4).field(FieldDesc::new("n", 0, Prim::I32, 1)).doc("Lives.")).unwrap();
    r.register_event(EventDesc::new("game", "score_changed").doc("The score changed.").payload("Score"))
        .unwrap();
    r.register_event(EventDesc::new("game", "reset").doc("Reset.")).unwrap();
    r.register_event(EventDesc::new("game", "lives").doc("Lives.").payload("Lives")).unwrap();
    r.register(
        HostFn::new("engine.events", "send").doc("Send.").param("ev", ApiType::I32).param("payload", ApiType::Ptr),
        |_, _, _| Ok(HostOk::Continue),
    )
    .unwrap();
    r.index()
}

fn with_api(src: &str) -> Result<Module, Vec<Diagnostic>> {
    let ix = index();
    assemble_with(src, "t.efa", &AsmOptions { api: Some(&ix), ..Default::default() }).map_err(|d| d.0)
}

fn plain(src: &str) -> Result<Module, Vec<Diagnostic>> {
    assemble_with(src, "t.efa", &AsmOptions::default()).map_err(|d| d.0)
}

fn ok_api(src: &str) -> Module {
    with_api(src).unwrap_or_else(|d| panic!("{}", efescript::asm::Diagnostics(d)))
}

fn ok_plain(src: &str) -> Module {
    plain(src).unwrap_or_else(|d| panic!("{}", efescript::asm::Diagnostics(d)))
}

fn err_text(r: Result<Module, Vec<Diagnostic>>) -> String {
    match r {
        Ok(_) => panic!("expected errors"),
        Err(d) => efescript::asm::Diagnostics(d).to_string(),
    }
}

fn evs(m: &Module) -> Vec<(String, String, Option<(String, u32)>)> {
    m.data().events.iter().map(|e| (e.namespace.clone(), e.name.clone(), e.payload.clone())).collect()
}

fn ev(ns: &str, name: &str, p: Option<(&str, u32)>) -> (String, String, Option<(String, u32)>) {
    (ns.to_string(), name.to_string(), p.map(|(s, n)| (s.to_string(), n)))
}

/// The immediates of every `const.i32` / `const.i64` instruction of function `f`.
fn consts(m: &Module, f: usize) -> Vec<u32> {
    m.data().functions[f]
        .code
        .iter()
        .filter(|i| i.op.name() == "const.i32" || i.op.name() == "const.i64")
        .map(|i| i.b)
        .collect()
}

const SEND: &str = "\
import engine.events.send
import event game.score_changed
import event game.reset

buf resb 8

export fn main() {
    reg n
    call engine.events.send(game.reset.id, 0)
    call engine.events.send(game.score_changed.id, buf)
    ret
}
";

#[test]
fn event_import_with_an_index_takes_the_payload_from_it() {
    let m = ok_api(SEND);
    assert_eq!(evs(&m), vec![ev("game", "score_changed", Some(("Score", 8))), ev("game", "reset", None)]);
    // namespace `game` is recorded in section 12
    let api = m.data().api.clone().expect("section 12");
    assert!(api.namespaces.contains(&("game".to_string(), 3)), "{:?}", api.namespaces);
    // survives the binary format
    let back = Module::decode(&m.encode()).expect("decodes");
    assert_eq!(back.data().events, m.data().events);
    // links against the registry that defines the events (events are checked by name and size)
}

#[test]
fn ids_are_immediates_in_operand_position() {
    let m = ok_api(SEND);
    // score_changed is id 0, reset is id 1; the call arguments are loaded as constants
    let c = consts(&m, 0);
    assert!(c.contains(&1) && c.contains(&0), "{c:?}");
    let m = ok_api(
        "import event game.reset\nimport event game.lives\nexport fn main() -> i32 {\n    reg n\n    const.i32 n, game.lives.id\n    ret n\n}\n",
    );
    assert_eq!(evs(&m), vec![ev("game", "reset", None), ev("game", "lives", Some(("Lives", 4)))]);
    assert_eq!(consts(&m, 0), vec![1]);
    // usable in a global initialiser and a data directive too
    let m = ok_api(
        "import event game.reset\nimport event game.lives\nglobal last: i32 = game.lives.id\ntbl dd game.reset.id, game.lives.id\nexport fn main() {\n    ret\n}\n",
    );
    assert_eq!(m.data().globals[0].init, 1);
    assert_eq!(m.data().data[0].bytes, vec![0, 0, 0, 0, 1, 0, 0, 0]);
}

#[test]
fn event_id_is_not_a_layout_constant() {
    let t = err_text(with_api("import event game.reset\nbuf resb game.reset.id\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("is an event id, which is only known when the program is linked"), "{t}");
}

#[test]
fn unknown_event_names_it_and_suggests() {
    let t = err_text(with_api("import event game.score_changd\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("unknown event `game.score_changd`"), "{t}");
    assert!(t.contains("did you mean `game.score_changed`?"), "{t}");
    let t = err_text(with_api("import event nope.x\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("unknown event `nope.x`"), "{t}");
}

#[test]
fn explicit_payload_must_match_the_index() {
    assert!(with_api("import event game.score_changed(&Score)\nexport fn main() {\n    ret\n}\n").is_ok());
    assert!(with_api("import event game.reset()\nexport fn main() {\n    ret\n}\n").is_ok());
    let t = err_text(with_api("import event game.score_changed(&Lives)\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("payload mismatch for event `game.score_changed`: declared `&Lives`, but the API index has `&Score`"), "{t}");
    let t = err_text(with_api("import event game.score_changed()\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("declared no payload, but the API index has `&Score`"), "{t}");
    let t = err_text(with_api("import event game.reset(&Score)\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("declared `&Score`, but the API index has no payload"), "{t}");
    let t = err_text(with_api("import event game.reset(&Nope)\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("unknown struct `Nope`"), "{t}");
}

#[test]
fn without_an_index_the_payload_form_is_required() {
    let t = err_text(plain("import event game.reset\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("event `game.reset` has no payload: write `import event game.reset(&Struct)` or `import event game.reset()`"), "{t}");
    let m = ok_plain(
        "struct Score {\n    points: i32\n    delta: i32\n}\nimport event game.score_changed(&Score)\nimport event game.reset()\nexport fn main() {\n    ret\n}\n",
    );
    assert_eq!(evs(&m), vec![ev("game", "score_changed", Some(("Score", 8))), ev("game", "reset", None)]);
    assert!(m.data().api.is_none());
    let t = err_text(plain("import event game.x(&Score)\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("unknown struct `Score`"), "{t}");
}

#[test]
fn bad_event_imports() {
    let t = err_text(plain("import event reset()\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("event name `reset` must be qualified as `namespace.name`"), "{t}");
    let t = err_text(plain("import event game.reset()\nimport event game.reset()\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("duplicate import of event `game.reset`"), "{t}");
    let t = err_text(plain("import event game.reset(\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("expected"), "{t}");
    let t = err_text(plain("import game.reset()\nimport event game.reset()\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("`game.reset` is an event here but a function import"), "{t}");
    // an alias may not hide the namespace of an event
    let files = MemSource::new()
        .with("main.efa", "use \"game.efa\"\nimport event game.reset()\nexport fn main() {\n    ret\n}\n")
        .with("game.efa", "pub const X = 1\n");
    let t = match assemble_program("main.efa", &files, &AsmOptions::default()) {
        Ok(_) => panic!("expected errors"),
        Err(d) => d.to_string(),
    };
    assert!(t.contains("alias `game` conflicts with the first segment of an imported namespace"), "{t}");
}

#[test]
fn merge_across_files_numbers_in_first_appearance_order() {
    let files = MemSource::new()
        .with(
            "main.efa",
            "use \"b.efa\"\nstruct Score {\n    points: i32\n    delta: i32\n}\nimport event game.score_changed(&Score)\nexport fn main() {\n    call b.go()\n    ret\n}\n",
        )
        .with(
            "b.efa",
            "struct Score {\n    points: i32\n    delta: i32\n}\nimport event game.reset()\nimport event game.score_changed(&Score)\npub fn go() -> i32 {\n    reg n\n    const.i32 n, game.score_changed.id\n    const.i32 n, game.reset.id\n    ret n\n}\n",
        );
    let m = assemble_program("main.efa", &files, &AsmOptions::default()).unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(evs(&m), vec![ev("game", "score_changed", Some(("Score", 8))), ev("game", "reset", None)]);
    assert_eq!(consts(&m, 1), vec![0, 1]);
}

#[test]
fn conflicting_payloads_across_files_name_both() {
    let files = MemSource::new()
        .with("main.efa", "use \"b.efa\"\nimport event game.reset()\nexport fn main() {\n    call b.go()\n    ret\n}\n")
        .with(
            "b.efa",
            "struct Score {\n    points: i32\n}\nimport event game.reset(&Score)\npub fn go() {\n    ret\n}\n",
        );
    let t = match assemble_program("main.efa", &files, &AsmOptions::default()) {
        Ok(_) => panic!("expected errors"),
        Err(d) => d.to_string(),
    };
    assert!(t.contains("event `game.reset` is declared with &Score (4 bytes) here but with no payload in `main.efa`"), "{t}");
    assert!(t.contains("first defined in main.efa:2"), "{t}");
}

#[test]
fn provided_event_goes_into_section_13_with_its_payload_struct() {
    let src = "\
export struct Hit, \"A hit.\" {
    damage: i32
    who: entity
}
provide event hit(&Hit), \"A hit landed.\"
provide event cleared(), \"All cleared.\"
provide event moved(&Lives), \"Moved.\"
export fn main() {
    ret
}
";
    let m = ok_api(src);
    let i = m.data().interface.clone().expect("section 13");
    let names: Vec<_> = i.events.iter().map(|e| (e.name.as_str(), e.payload.as_deref(), e.doc.as_str())).collect();
    assert_eq!(names, vec![("hit", Some("Hit"), "A hit landed."), ("cleared", None, "All cleared."), ("moved", Some("Lives"), "Moved.")]);
    let own: Vec<_> = i.structs.iter().map(|s| (s.name.as_str(), s.owner.as_str())).collect();
    assert_eq!(own, vec![("Hit", ""), ("Lives", "demo")]);
    assert!(i.provides.is_empty());
    let back = Module::decode(&m.encode()).expect("decodes");
    assert_eq!(back.data().interface, m.data().interface);
}

#[test]
fn provided_event_rules() {
    // the payload must be an `export struct`
    let t = err_text(plain("struct P {\n    a: i32\n}\nprovide event e(&P), \"d\"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("struct `P` is the payload of provided event `e` and must be `export`"), "{t}");
    // blank doc
    let t = err_text(plain("provide event e(), \" \"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("the doc of provided event `e` must not be blank"), "{t}");
    // unique names
    let t = err_text(plain("provide event e(), \"d\"\nprovide event e(), \"d\"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("duplicate provided event `e`"), "{t}");
    // rule 9: not equal to a provide name
    let t = err_text(plain(
        "provide e() = f, \"d\"\nprovide event e(), \"d\"\nfn f() {\n    ret\n}\nexport fn main() {\n    ret\n}\n",
    ));
    assert!(t.contains("provided event `e` has the same name as provide `e`"), "{t}");
    // identifier name, not reserved
    let t = err_text(plain("provide event a.b(), \"d\"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("not a valid event name"), "{t}");
    let t = err_text(plain("provide event use(), \"d\"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("reserved word"), "{t}");
    // syntax
    let t = err_text(plain("provide event e()\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("`,` and the doc string of the event"), "{t}");
    // `pub` is not allowed
    let t = err_text(plain("pub provide event e(), \"d\"\nexport fn main() {\n    ret\n}\n"));
    assert!(t.contains("`pub` is not allowed on `provide`"), "{t}");
}

#[test]
fn a_mod_sends_its_own_event() {
    let src = "\
struct Hit {
    damage: i32
}
export struct Hit2 {
    damage: i32
}
import engine.events.send
import event mymod.hit2(&Hit2)
provide event hit2(&Hit2), \"A hit.\"
export fn main() {
    call engine.events.send(mymod.hit2.id, 0)
    ret
}
";
    // not in the index, but the program provides `hit2`: accepted, namespace is not checked
    let m = ok_api(src);
    assert_eq!(evs(&m), vec![ev("mymod", "hit2", Some(("Hit2", 4)))]);
    assert_eq!(m.data().interface.as_ref().unwrap().events.len(), 1);
    // a different payload than the provided one
    let t = err_text(with_api(&src.replace("import event mymod.hit2(&Hit2)", "import event mymod.hit2(&Hit)")));
    assert!(t.contains("event `mymod.hit2` is imported with payload `&Hit` but the program provides `hit2` with `&Hit2`"), "{t}");
    // a plain unknown event is still an error
    let t = err_text(with_api(&src.replace("mymod.hit2(&Hit2)", "mymod.other(&Hit2)").replace("mymod.hit2.id", "mymod.other.id")));
    assert!(t.contains("unknown event `mymod.other`"), "{t}");
    // the program may provide it from another file
    let files = MemSource::new()
        .with(
            "main.efa",
            "use \"p.efa\"\nstruct Hit2 {\n    damage: i32\n}\nimport event mymod.hit2(&Hit2)\nexport fn main() {\n    call p.f()\n    ret\n}\n",
        )
        .with("p.efa", "export struct Hit2 {\n    damage: i32\n}\nprovide event hit2(&Hit2), \"A hit.\"\npub fn f() {\n    ret\n}\n");
    // Hit2 is exported in p.efa and a different struct in main.efa; same name and size
    let ix = index();
    let m = assemble_program("main.efa", &files, &AsmOptions { api: Some(&ix), ..Default::default() }).unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(evs(&m), vec![ev("mymod", "hit2", Some(("Hit2", 4)))]);
}

#[test]
fn event_is_a_contextual_word() {
    // usable as any name; `import event.x(...)` and `provide event(...)` keep their old meaning
    let src = "\
import std.println(i64, i64)
const event = 3
event db 1
fn event_x() {
    reg event
    ret
}
provide event() = event_x, \"A provide called event.\"
export fn main() {
    reg event
    const.i32 event, 1
    ret
}
";
    // `event db 1` clashes with the const of the same name; use distinct names
    let t = err_text(plain(src));
    assert!(t.contains("duplicate definition of `event`"), "{t}");
    let m = ok_plain(
        "import std.println(i64, i64)\nconst event = 3\nfn event_x() {\n    ret\n}\nprovide event() = event_x, \"A provide called event.\"\nexport fn main() {\n    reg event\n    const.i32 event, event\n    ret\n}\n",
    );
    assert_eq!(m.data().interface.as_ref().unwrap().provides[0].name, "event");
    assert!(m.data().events.is_empty());
    // a function import in a namespace called `event`
    let m = ok_plain("import event.ping(i32)\nexport fn main() {\n    reg n\n    const.i32 n, 1\n    call event.ping(n)\n    ret\n}\n");
    assert_eq!(m.data().imports[0].namespace, "event");
    assert!(m.data().events.is_empty());
    // a label called `event`
    ok_plain("export fn main() {\nevent:\n    ret\n}\n");
}

#[test]
fn programs_without_events_are_unchanged() {
    let src = "import std.println(i64, i64)\nmsg db \"hi\"\nexport fn main() {\n    call std.println(msg, msg.len)\n    ret\n}\n";
    let m = ok_plain(src);
    assert!(m.data().events.is_empty());
    assert!(m.data().interface.is_none());
    // the section is not even written
    let bytes = m.encode();
    assert!(Module::decode(&bytes).unwrap().data().events.is_empty());
}
