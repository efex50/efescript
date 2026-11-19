//! `efescript pack`, `inspect`, `api-export` and `--api x.efmod` (e8-design.md sections 8, 10).

use std::path::PathBuf;
use std::process::{Command, Output};

use efemod::{Package, ReadLimits};
use efescript::pack::{self, pack_dir, read_mod_toml};
use efevm::api::ApiIndex;

const BIN: &str = env!("CARGO_BIN_EXE_efescript");

fn efescript(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().expect("spawn efescript")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A unique directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("efescript-pack-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).expect("dirs");
        }
        std::fs::write(&p, text).expect("write temp file");
        p
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn s(&self, name: &str) -> String {
        self.path(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BAG_TOML: &str = r#"[mod]
id = "bag"
version = "1.2.0"
name = "Bag"
description = "Items in a bag."
authors = ["Efe"]
license = "MIT"
"#;

const BAG_MAIN: &str = r#"use "items.efa"
provide give(item: str, count: i32) -> (status: i32) = items.add,
    "Add `count` of `item` to the bag; 0 ok, 1 bag full."

export fn main() {
    ret
}
"#;

const BAG_ITEMS: &str = r#"pub fn add(name: i64, len: i64, count: i32) -> i32 {
    ret 0
}
"#;

/// The mod folder `bag/` below `s`.
fn bag(s: &Scratch) -> PathBuf {
    s.write("bag/mod.toml", BAG_TOML);
    s.write("bag/src/main.efa", BAG_MAIN);
    s.write("bag/src/items.efa", BAG_ITEMS);
    s.write("bag/assets/data/items.json", "{\"sword\": 1}");
    s.path("bag")
}

const SHOP_TOML: &str = r#"[mod]
id = "shop"
version = "0.1.0"

[dependencies]
bag = "^1.2"
"#;

const SHOP_MAIN: &str = r#"import bag.give

export fn main() {
    call bag.give(hello, hello.len, 3)
    ret
}

hello db "sword"
"#;

fn read_pkg(bytes: &[u8]) -> Package {
    Package::read(bytes, &ReadLimits::default()).expect("package reads")
}

#[test]
fn packs_a_mod_folder() {
    let s = Scratch::new("basic");
    let dir = bag(&s);
    let p = pack_dir(&dir, &[]).expect("pack");
    assert_eq!(p.file_name, "bag-1.2.0.efmod");
    let pkg = read_pkg(&p.bytes);
    assert_eq!(pkg.manifest.id, "bag");
    assert_eq!(pkg.manifest.version.to_string(), "1.2.0");
    assert_eq!(pkg.manifest.name.as_deref(), Some("Bag"));
    assert_eq!(pkg.manifest.authors, vec!["Efe".to_string()]);
    let iface = pkg.module.data().interface.as_ref().expect("section 13");
    assert_eq!(iface.provides.len(), 1);
    assert_eq!(iface.provides[0].name, "give");
    assert_eq!(pkg.asset("data/items.json"), Some(&b"{\"sword\": 1}"[..]));
    assert!(pkg.ignored.is_empty());
    // The package's index exposes the provide under the mod id.
    let ix = pkg.index();
    assert_eq!(ix.functions[0].qualified(), "bag.give");
    assert_eq!(ix.functions[0].owner, "bag");
}

#[test]
fn packing_twice_is_byte_identical() {
    let s = Scratch::new("twice");
    let dir = bag(&s);
    s.write("bag/assets/a.txt", "a");
    s.write("bag/assets/z/b.txt", "b");
    let a = pack_dir(&dir, &[]).unwrap();
    let b = pack_dir(&dir, &[]).unwrap();
    assert_eq!(a.bytes, b.bytes);
}

#[test]
fn mod_toml_rejects_unknown_keys_and_tables() {
    let e = read_mod_toml("[mod]\nid = \"a\"\nversion = \"1.0.0\"\nauthor = \"x\"\n").unwrap_err();
    assert!(e.contains("author"), "{e}");
    let e = read_mod_toml("[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[extras]\nx = 1\n").unwrap_err();
    assert!(e.contains("extras"), "{e}");
    let e = read_mod_toml("[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[build]\nentri = \"x\"\n").unwrap_err();
    assert!(e.contains("entri"), "{e}");
    let e = read_mod_toml("[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[permissions]\nbudgets = 1\n").unwrap_err();
    assert!(e.contains("budgets"), "{e}");

    // Through pack_dir too.
    let s = Scratch::new("unknown");
    s.write("m/mod.toml", "[mod]\nid = \"m\"\nversion = \"1.0.0\"\ncolour = \"red\"\n");
    s.write("m/src/main.efa", "export fn main() {\n    ret\n}\n");
    let e = pack_dir(&s.path("m"), &[]).unwrap_err();
    assert!(e.contains("colour"), "{e}");
}

#[test]
fn undeclared_import_is_rejected() {
    let s = Scratch::new("undeclared");
    s.write("m/mod.toml", "[mod]\nid = \"m\"\nversion = \"1.0.0\"\n");
    s.write("m/src/main.efa", "import engine.log.info(i64, i64)\nexport fn main() {\n    ret\n}\n");
    let e = pack_dir(&s.path("m"), &[]).unwrap_err();
    assert!(e.contains("engine.log"), "{e}");
    // Declaring the namespace makes it pack.
    s.write("m/mod.toml", "[mod]\nid = \"m\"\nversion = \"1.0.0\"\n[permissions]\nnamespaces = [\"engine.log\"]\n");
    pack_dir(&s.path("m"), &[]).expect("declared import packs");
}

#[test]
fn build_paths_stay_inside_the_mod_folder() {
    let s = Scratch::new("inside");
    s.write("m/mod.toml", "[mod]\nid = \"m\"\nversion = \"1.0.0\"\n[build]\nentry = \"../x.efa\"\n");
    let e = pack_dir(&s.path("m"), &[]).unwrap_err();
    assert!(e.contains("inside the mod folder"), "{e}");
    s.write("m/mod.toml", "[mod]\nid = \"m\"\nversion = \"1.0.0\"\n[build]\nassets = \"/etc\"\n");
    s.write("m/src/main.efa", "export fn main() {\n    ret\n}\n");
    let e = pack_dir(&s.path("m"), &[]).unwrap_err();
    assert!(e.contains("inside the mod folder"), "{e}");
}

fn log_index(game: &str, names: &[&str]) -> ApiIndex {
    use efevm::api::{ApiType, HostFn, HostRegistry};
    let mut r: HostRegistry<()> = HostRegistry::new();
    r.set_game(game);
    r.define_namespace("engine.log", 1, "engine", "Logging.").unwrap();
    for name in names {
        r.register(
            HostFn::new("engine.log", *name).doc("Log a line.").param("text", ApiType::Str),
            |_, _, _| Ok(efevm::HostOk::Continue),
        )
        .unwrap();
    }
    r.index()
}

#[test]
fn build_api_and_extra_apis_are_merged() {
    let s = Scratch::new("merge");
    let a = log_index("gameA", &["info", "debug"]);
    let b = log_index("gameB", &["info", "warn"]);
    s.write("m/a.efapi", &a.to_json());
    s.write(
        "m/mod.toml",
        "[mod]\nid = \"m\"\nversion = \"1.0.0\"\n[permissions]\nnamespaces = [\"engine.log\"]\n[build]\napi = [\"a.efapi\"]\n",
    );
    s.write(
        "m/src/main.efa",
        "import engine.log.debug\nimport engine.log.warn\nmsg db \"hi\"\nexport fn main() {\n    call engine.log.debug(msg, msg.len)\n    call engine.log.warn(msg, msg.len)\n    ret\n}\n",
    );
    // `warn` is only in the extra index.
    let e = pack_dir(&s.path("m"), &[]).unwrap_err();
    assert!(e.contains("engine.log.warn"), "{e}");
    let p = pack_dir(&s.path("m"), &[&b]).expect("merged");
    let merged = ApiIndex::merge(&[&b, &a]).unwrap();
    assert_eq!(p.module.data().api.as_ref().unwrap().hash, merged.hash());
}

#[test]
fn a_mod_builds_against_another_efmod() {
    let s = Scratch::new("dep");
    let bag_dir = bag(&s);
    let bag_pkg = pack_dir(&bag_dir, &[]).unwrap();
    let bag_file = s.path("bag-1.2.0.efmod");
    std::fs::write(&bag_file, &bag_pkg.bytes).unwrap();

    s.write("shop/mod.toml", SHOP_TOML);
    s.write("shop/src/main.efa", SHOP_MAIN);
    let ix = pack::read_api_input(&bag_file).expect("efmod as api input");
    assert_eq!(ix.functions[0].qualified(), "bag.give");
    let shop = pack_dir(&s.path("shop"), &[&ix]).expect("shop packs against bag.efmod");
    let imports: Vec<String> =
        shop.module.data().imports.iter().map(|i| format!("{}.{}", i.namespace, i.name)).collect();
    assert_eq!(imports, vec!["bag.give".to_string()]);
    let pkg = read_pkg(&shop.bytes);
    assert_eq!(pkg.manifest.dependencies[0].id, "bag");

    // Without the dependency declared, check_module refuses it.
    s.write("shop/mod.toml", "[mod]\nid = \"shop\"\nversion = \"0.1.0\"\n");
    let e = pack_dir(&s.path("shop"), &[&ix]).unwrap_err();
    assert!(e.contains("bag"), "{e}");
}

#[test]
fn read_api_input_sniffs_content() {
    let s = Scratch::new("sniff");
    let bag_dir = bag(&s);
    let p = pack_dir(&bag_dir, &[]).unwrap();
    std::fs::write(s.path("noext"), &p.bytes).unwrap();
    assert_eq!(pack::read_api_input(&s.path("noext")).unwrap().functions[0].name, "give");
    let json = pack::read_api_input(&s.path("noext")).unwrap().to_json();
    s.write("noext2", &json);
    assert!(pack::read_api_input(&s.path("noext2")).is_ok());
    s.write("junk.bin", "hello");
    assert!(pack::read_api_input(&s.path("junk.bin")).is_err());
}

#[test]
fn cli_pack_inspect_and_api_export() {
    let s = Scratch::new("cli");
    bag(&s);
    let out = s.s("out/bag.efmod");
    std::fs::create_dir_all(s.path("out")).unwrap();
    let o = efescript(&["pack", &s.s("bag"), "-o", &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    let pkg = read_pkg(&std::fs::read(&out).unwrap());
    assert_eq!(pkg.manifest.id, "bag");

    let o = efescript(&["inspect", &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    let t = stdout(&o);
    assert!(t.contains("bag 1.2.0"), "{t}");
    assert!(t.contains("give(item: str, count: i32) -> (status: i32)"), "{t}");
    assert!(t.contains("data/items.json"), "{t}");

    let o = efescript(&["api-export", &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    let ix = ApiIndex::from_json(&stdout(&o)).expect("api-export parses");
    assert_eq!(ix.functions[0].qualified(), "bag.give");
    assert!(ix.namespaces.iter().all(|n| n.owner == "bag"), "{:?}", ix.namespaces);
    assert!(ix.functions.iter().all(|f| f.owner == "bag"));

    // A bare module needs --id.
    let efvm = s.s("bare.efvm");
    std::fs::write(&efvm, pkg.module.encode()).unwrap();
    let o = efescript(&["api-export", &efvm]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--id"), "{}", stderr(&o));
    let o = efescript(&["api-export", &efvm, "--id", "bag"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let ix = ApiIndex::from_json(&stdout(&o)).unwrap();
    assert_eq!(ix.functions[0].owner, "bag");
    let o = efescript(&["inspect", &efvm]);
    assert!(stdout(&o).contains("give"), "{}", stdout(&o));
}

#[test]
fn cli_builds_against_an_efmod_api() {
    let s = Scratch::new("cli-dep");
    bag(&s);
    s.write("shop/mod.toml", SHOP_TOML);
    s.write("shop/src/main.efa", SHOP_MAIN);
    let bag_file = s.s("bag.efmod");
    let o = efescript(&["pack", &s.s("bag"), "-o", &bag_file]);
    assert!(o.status.success(), "{}", stderr(&o));
    let shop_file = s.s("shop.efmod");
    let o = efescript(&["pack", &s.s("shop"), "--api", &bag_file, "-o", &shop_file]);
    assert!(o.status.success(), "{}", stderr(&o));
    // `build` accepts an .efmod too.
    let o = efescript(&["build", &s.s("shop/src/main.efa"), "--api", &bag_file, "-o", &s.s("shop.efvm")]);
    assert!(o.status.success(), "{}", stderr(&o));
    // A pack failure exits 2 with the reason.
    let o = efescript(&["pack", &s.s("shop")]);
    assert_eq!(o.status.code(), Some(2));
}
