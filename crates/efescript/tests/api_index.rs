//! E4.6 (std part) and the E4 tooling end to end: the `std` registry against its index,
//! `efescript api` / `api-export`, and assembling against an index file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use efescript::asm::{assemble_with, AsmOptions};
use efescript::disasm::disassemble;
use efescript::host_std::{std_registry, StdEnv, StdOptions};
use efescript::prelude::{CaptureHost, StdHost};
use efescript::run_v2;
use efevm::api::{ApiIndex, ApiType, FieldDesc, HostFn, HostRegistry, Prim, StructDesc};
use efevm::{HostOk, Module};

const BIN: &str = env!("CARGO_BIN_EXE_efescript");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn efescript(args: &[&str]) -> Output {
    Command::new(BIN).args(args).current_dir(ROOT).output().expect("spawn efescript")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A per-test scratch directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("efescript-api-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("write temp file");
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("UTF-8 temp path")
}

fn std_index() -> ApiIndex {
    std_registry::<CaptureHost>(&StdOptions::default()).index()
}

#[test]
fn every_import_has_a_documented_index_function_of_the_same_type() {
    let reg = std_registry::<CaptureHost>(&StdOptions::default());
    let imports = reg.imports();
    let index = reg.index();
    index.validate().expect("the std index is valid");
    let mut n = 0;
    for (ns, name, ty) in imports.names() {
        let f = index.function(ns, name).unwrap_or_else(|| panic!("{ns}.{name} is not in the index"));
        assert!(!f.doc.trim().is_empty(), "{ns}.{name} has no doc");
        assert_eq!(&f.func_type(), ty, "{ns}.{name}: lowered type differs from the import");
        assert!(imports.namespace_version(ns).is_some(), "{ns} has no version");
        n += 1;
    }
    assert_eq!(n, 12, "std (8) + std.time (2) + std.fs (2), see e3-design section 4");
}

#[test]
fn every_index_function_is_an_import() {
    let reg = std_registry::<CaptureHost>(&StdOptions::default());
    let imports = reg.imports();
    let names: Vec<(String, String)> = imports.names().map(|(a, b, _)| (a.to_string(), b.to_string())).collect();
    for f in &std_index().functions {
        assert!(
            names.iter().any(|(ns, n)| *ns == f.namespace && *n == f.name),
            "{} is in the index but not callable",
            f.qualified()
        );
        assert_eq!(f.owner, "std");
        for p in f.params.iter().chain(&f.results) {
            assert!(!p.name.is_empty());
        }
    }
    let index = std_index();
    for ns in &index.namespaces {
        assert_eq!((ns.version, ns.owner.as_str()), (1, "std"), "{}", ns.name);
        assert!(!ns.doc.is_empty());
    }
}

#[test]
fn the_index_does_not_depend_on_the_options() {
    let with_root = StdOptions { fs_root: Some(std::env::temp_dir()) };
    let a = std_registry::<CaptureHost>(&StdOptions::default()).index();
    let b = std_registry::<StdHost>(&with_root).index();
    assert_eq!(a, b);
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn json_round_trip_keeps_the_hash() {
    let index = std_index();
    let json = index.to_json();
    let back = ApiIndex::from_json(&json).expect("parses");
    assert_eq!(back, index);
    assert_eq!(back.hash(), index.hash());
    assert_eq!(back.to_json(), json);
}

#[test]
fn api_export_cli_matches_the_library_index() {
    let o = efescript(&["api-export"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let from_cli = ApiIndex::from_json(&stdout(&o)).expect("the exported text parses");
    assert_eq!(from_cli, std_index());
    assert_eq!(from_cli.game, "std");

    let dir = Scratch::new("export");
    let out = dir.0.join("std.efapi");
    let o = efescript(&["api-export", "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(o.stdout.is_empty());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), std_index().to_json());
}

#[test]
fn api_cli_prints_every_function() {
    let dir = Scratch::new("print");
    let f = dir.write("std.efapi", &std_index().to_json());
    let o = efescript(&["api", s(&f)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let text = stdout(&o);
    for func in &std_index().functions {
        assert!(text.contains(&func.qualified()), "missing {} in:\n{text}", func.qualified());
        assert!(text.contains(&func.signature_text()), "missing signature of {} in:\n{text}", func.qualified());
        assert!(text.contains(&func.doc), "missing doc of {} in:\n{text}", func.qualified());
    }
    assert!(text.contains("std.fs v1 (owner std)"), "{text}");
    assert!(text.contains(&format!("fnv1a64:{:016x}", std_index().hash())), "{text}");
    assert!(text.contains("read_line(buf: bytes) -> (len: i64)"), "{text}");
}

#[test]
fn api_cli_prints_types_with_offsets() {
    let dir = Scratch::new("types");
    let f = dir.write("game.efapi", &game_index().to_json());
    let text = stdout(&efescript(&["api", s(&f)]));
    assert!(text.contains("Transform (size 20, align 4, component id 77, owner engine)"), "{text}");
    assert!(text.contains("+8   rot: f32  Rotation in radians."), "{text}");
    assert!(text.contains("+12  scale: f32 x2"), "{text}");
}

#[test]
fn api_cli_exits_2_on_read_and_parse_errors() {
    let dir = Scratch::new("bad");
    let o = efescript(&["api", s(&dir.0.join("nope.efapi"))]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(stderr(&o).contains("nope.efapi"), "{}", stderr(&o));

    let junk = dir.write("junk.efapi", "{ not json");
    let o = efescript(&["api", s(&junk)]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("junk.efapi"), "{}", stderr(&o));

    // an edited file fails the hash check
    let edited = dir.write("edited.efapi", &std_index().to_json().replace("Write the bytes", "Emit the bytes"));
    let o = efescript(&["api", s(&edited)]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("hash mismatch"), "{}", stderr(&o));

    let o = efescript(&["api"]);
    assert_eq!(o.status.code(), Some(2));
}

// ---- assembling against an index ----

/// A game-like index: `engine.world`, `engine.log` and the component `Transform`.
fn game_index() -> ApiIndex {
    let mut r: HostRegistry<()> = HostRegistry::new();
    r.set_game("demo");
    r.define_namespace("engine.world", 2, "engine", "Entities and components.").unwrap();
    r.define_namespace("engine.log", 1, "engine", "Logging.").unwrap();
    let mut t = StructDesc::new("Transform", 20, 4)
        .field(FieldDesc::new("pos", 0, Prim::F32, 2).with_doc("Position."))
        .field(FieldDesc::new("rot", 8, Prim::F32, 1).with_doc("Rotation in radians."))
        .field(FieldDesc::new("scale", 12, Prim::F32, 2))
        .doc("Position, rotation and scale.");
    t.owner = "engine".into();
    t.component = Some(77);
    r.register_type(t).unwrap();
    r.register(
        HostFn::new("engine.world", "spawn").doc("Create an entity.").result("e", ApiType::Entity),
        |_, _, _| Ok(HostOk::Continue),
    )
    .unwrap();
    r.register(
        HostFn::new("engine.log", "info").doc("Log a line.").param("text", ApiType::Str),
        |_, _, _| Ok(HostOk::Continue),
    )
    .unwrap();
    r.index()
}

const GAME_SCRIPT: &str = "\
import engine.world.spawn
import engine.log.info

msg db \"spawned\"
buf resb Transform.size

export fn main() -> i32 {
    reg e, n
    call engine.world.spawn() -> e
    call engine.log.info(msg, msg.len)
    const.i32 n, Transform.rot
    ret n
}
";

#[test]
fn assemble_an_example_against_an_index_read_from_json() {
    let json = game_index().to_json();
    let index = ApiIndex::from_json(&json).expect("index parses");
    let m = assemble_with(GAME_SCRIPT, "game.efa", &AsmOptions { api: Some(&index), ..Default::default() })
        .unwrap_or_else(|e| panic!("{e}"));
    let api = m.data().api.clone().expect("section 12");
    assert_eq!(api.hash, index.hash());
    assert_eq!(api.namespaces, vec![("engine.log".to_string(), 1), ("engine.world".to_string(), 2)]);
    // imports have the lowered index signatures
    let d = m.data();
    assert_eq!(d.types[d.imports[0].type_idx as usize].to_string(), "() -> i64");
    assert_eq!(d.types[d.imports[1].type_idx as usize].to_string(), "(i64, i64)");
    // the module survives the binary format
    let back = Module::decode(&m.encode()).expect("decodes");
    assert_eq!(back.data().api, m.data().api);

    // the host that has these functions links it; one with the std registry reports every problem
    let std_imports = std_registry::<CaptureHost>(&StdOptions::default()).imports();
    let problems: Vec<String> = std_imports.check_module(&m).iter().map(|e| e.to_string()).collect();
    assert!(problems.iter().any(|p| p.contains("engine.world")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("missing import engine.world.spawn")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("missing import engine.log.info")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains(&format!("fnv1a64:{:016x}", index.hash()))), "{problems:?}");
}

#[test]
fn disassembly_comments_section_12_and_reassembles_with_the_index() {
    let index = game_index();
    let opts = AsmOptions { api: Some(&index), ..Default::default() };
    let m = assemble_with(GAME_SCRIPT, "game.efa", &opts).unwrap();
    let text = disassemble(&m);
    let first = text.lines().next().unwrap();
    assert_eq!(first, format!("; api fnv1a64:{:016x} engine.log@1 engine.world@2", index.hash()), "{text}");
    // Without an index the text assembles but has no section 12 (the comment is only a comment);
    // with the same index it is byte-identical to the original apart from the lines section.
    let plain = efescript::asm::assemble(&text, "dis.efa");
    assert!(plain.is_ok_and(|p| p.data().api.is_none()));
    let again = assemble_with(&text, "dis.efa", &opts).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let strip = |m: &Module| {
        let mut d = m.data().clone();
        d.lines = Default::default();
        Module::new(d).unwrap().encode()
    };
    assert_eq!(strip(&m), strip(&again));
}

#[test]
fn run_and_build_with_api_through_the_cli() {
    let dir = Scratch::new("cli");
    let std_api = dir.write("std.efapi", &std_index().to_json());
    let game_api = dir.write("game.efapi", &game_index().to_json());
    let hello = dir.write(
        "hello_api.efa",
        "import std.println\nmsg db \"hi\"\nexport fn main() {\n    call std.println(msg, msg.len)\n    ret\n}\n",
    );

    // signature-less import, checked against the std index, then run
    let o = efescript(&["run", s(&hello), "--api", s(&std_api)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "hi\n");

    // without --api the same file is a diagnostic with a position
    let o = efescript(&["run", s(&hello)]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains(":1:8"), "{}", stderr(&o));
    assert!(stderr(&o).contains("has no signature"), "{}", stderr(&o));

    // an unknown import in the game index: diagnostic with a suggestion
    let typo = dir.write("typo.efa", "import engine.log.ifno\nexport fn main() {\n    ret\n}\n");
    let o = efescript(&["build", s(&typo), "--api", s(&game_api), "-o", s(&dir.0.join("t.efvm"))]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("unknown host function `engine.log.ifno`"), "{}", stderr(&o));
    assert!(stderr(&o).contains("did you mean `engine.log.info`?"), "{}", stderr(&o));

    // build a game module, then `run` (std host only) lists every link problem by name
    let script = dir.write("game.efa", GAME_SCRIPT);
    let out = dir.0.join("game.efvm");
    let o = efescript(&["build", s(&script), "--api", s(&game_api), "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let m = Module::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(m.data().api.as_ref().map(|a| a.hash), Some(game_index().hash()));
    let o = efescript(&["run", s(&out)]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("missing import engine.world.spawn"), "{err}");
    assert!(err.contains("missing import engine.log.info"), "{err}");
    assert!(err.contains("namespace engine.world"), "{err}");

    // a bad index file is an error before anything is assembled
    let o = efescript(&["run", s(&hello), "--api", s(&dir.0.join("missing.efapi"))]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn run_v2_reports_link_problems_by_name() {
    let src = "import std.nothing(i64)\nimport std.alsonot(i32)\nexport fn main() {\n    call std.nothing(1)\n    call std.alsonot(2)\n    ret\n}\n";
    let prog = run_v2::load_source(src, "t.efa").unwrap();
    let mut env = StdEnv::new(CaptureHost::new());
    let o = run_v2::run(&prog, &mut env, None);
    assert_eq!(o.exit_code, 2);
    let e = o.error.unwrap();
    assert!(e.contains("missing import std.nothing") && e.contains("missing import std.alsonot"), "{e}");
}
