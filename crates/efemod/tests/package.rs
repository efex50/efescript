use std::collections::BTreeMap;

use efemod::container;
use efemod::{check_module, Dependency, Manifest, Package, PackageError, Permissions, ReadLimits, Version};
use efevm::{ApiType, FuncType, FunctionBuilder, Interface, ModuleBuilder, Param, Provide, ValType};

// ---- helpers ------------------------------------------------------------------------

fn module_with(imports: &[(&str, &str)], iface: bool) -> Vec<u8> {
    let mut mb = ModuleBuilder::new();
    for (ns, n) in imports {
        mb.import(ns, n, FuncType::new([ValType::I64, ValType::I64], []));
    }
    let mut f = FunctionBuilder::new(1);
    f.ret(0);
    let func = mb.add_func(FuncType::new([ValType::I32], [ValType::I32]), f);
    mb.export_func("main", func);
    mb.memory(1, Some(4));
    if iface {
        mb.interface(Interface {
            provides: vec![Provide {
                name: "run".into(),
                func,
                doc: "Run it.".into(),
                params: vec![Param::new("x", ApiType::I32)],
                results: vec![Param::new("r", ApiType::I32)],
            }],
            ..Default::default()
        });
    }
    mb.finish().unwrap().encode()
}

fn module() -> Vec<u8> {
    module_with(&[("engine.log", "info")], true)
}

fn manifest() -> Manifest {
    let mut m = Manifest::new("inventory", Version::new(1, 2, 0));
    m.permissions.namespaces = vec!["engine.log".into()];
    m
}

fn assets() -> BTreeMap<String, Vec<u8>> {
    let mut a = BTreeMap::new();
    a.insert("items.json".to_string(), b"[1,2,3]".to_vec());
    a.insert("img/a.png".to_string(), vec![7u8; 500]);
    a.insert("Zed.txt".to_string(), b"z".to_vec());
    a
}

fn read(bytes: &[u8]) -> Result<Package, PackageError> {
    Package::read(bytes, &ReadLimits::default())
}

/// An archive with the given manifest text and module bytes plus extra entries.
fn archive(manifest: &str, module: &[u8], extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut v: Vec<(&str, &[u8])> = vec![("manifest.json", manifest.as_bytes()), ("module.efvm", module)];
    v.extend_from_slice(extra);
    container::write(&v)
}

fn man_err(text: &str) -> String {
    match Manifest::from_json(text) {
        Err(PackageError::Manifest(m)) => m,
        other => panic!("expected a manifest error for {text}, got {other:?}"),
    }
}

/// The valid manifest text with `edit` applied to the JSON text.
fn with(edit: impl Fn(&str) -> String) -> String {
    edit(&example_text())
}

fn example_text() -> String {
    r#"{
  "format": "efmod",
  "format_version": 1,
  "id": "inventory",
  "version": "1.2.0",
  "name": "Inventory",
  "description": "Items, bags and a shop.",
  "authors": ["Efe"],
  "license": "MIT",
  "game": "script_demo",
  "api_version": 1,
  "dependencies": [
    { "id": "core_items", "req": "^1.2" }
  ],
  "permissions": {
    "namespaces": ["engine.log", "engine.world"],
    "budget": 2000000,
    "memory_pages": 64
  }
}
"#
    .to_string()
}

fn example() -> Manifest {
    let mut m = Manifest::new("inventory", Version::new(1, 2, 0));
    m.name = Some("Inventory".into());
    m.description = Some("Items, bags and a shop.".into());
    m.authors = vec!["Efe".into()];
    m.license = Some("MIT".into());
    m.game = Some("script_demo".into());
    m.dependencies = vec![Dependency { id: "core_items".into(), req: "^1.2".parse().unwrap() }];
    m.permissions = Permissions {
        namespaces: vec!["engine.log".into(), "engine.world".into()],
        budget: Some(2_000_000),
        memory_pages: Some(64),
    };
    m
}

// ---- manifest -----------------------------------------------------------------------

#[test]
fn example_writer_output_is_exact() {
    assert_eq!(example().to_json(), example_text());
}

#[test]
fn example_parses_and_round_trips() {
    let m = Manifest::from_json(&example_text()).unwrap();
    assert_eq!(m, example());
    assert_eq!(Manifest::from_json(&m.to_json()).unwrap(), m);
}

#[test]
fn minimal_manifest_omits_optionals() {
    let m = Manifest::new("a", Version::new(0, 1, 0));
    assert_eq!(
        m.to_json(),
        "{\n  \"format\": \"efmod\",\n  \"format_version\": 1,\n  \"id\": \"a\",\n  \"version\": \"0.1.0\",\n  \"api_version\": 1\n}\n"
    );
    assert_eq!(Manifest::from_json(&m.to_json()).unwrap(), m);
}

#[test]
fn string_escaping_round_trips() {
    let mut m = manifest();
    m.name = Some("Q\"uote \\ back \u{e9}\u{4e2d} \u{1f600}".into());
    m.description = Some("line1\nline2\ttab".into());
    m.authors = vec!["a\u{1}b".into(), "x/y".into()];
    let t = m.to_json();
    assert!(t.contains("\\n") && t.contains("\\u0001") && t.contains("\\\""));
    assert_eq!(Manifest::from_json(&t).unwrap(), m);
}

#[test]
fn unknown_keys_are_ignored() {
    let t = with(|s| s.replacen("\"id\"", "\"future\": {\"x\": [1]},\n  \"id\"", 1));
    assert_eq!(Manifest::from_json(&t).unwrap(), example());
}

#[test]
fn defaults_and_names() {
    let m = Manifest::new("abc", Version::new(1, 0, 0));
    assert_eq!(m.display_name(), "abc");
    assert_eq!(m.namespace_doc(), "Functions provided by the mod `abc`.");
    assert_eq!(example().display_name(), "Inventory");
    assert_eq!(example().namespace_doc(), "Items, bags and a shop.");
    let t = "{\"format\":\"efmod\",\"format_version\":1,\"id\":\"a\",\"version\":\"1.0.0\"}";
    assert_eq!(Manifest::from_json(t).unwrap().api_version, 1);
}

#[test]
fn rejects_bad_ids() {
    for bad in ["", "A", "1a", "a-b", "a b", "\u{e9}", &"a".repeat(65)] {
        let t = with(|s| s.replacen("\"id\": \"inventory\"", &format!("\"id\": \"{bad}\""), 1));
        assert!(man_err(&t).starts_with("id:"), "{bad}");
    }
    let ok = "a".repeat(64);
    let t = with(|s| s.replacen("\"id\": \"inventory\"", &format!("\"id\": \"{ok}\""), 1));
    assert!(Manifest::from_json(&t).is_ok());
}

#[test]
fn rejects_reserved_ids() {
    for r in ["engine", "std", "efe"] {
        let t = with(|s| s.replacen("\"id\": \"inventory\"", &format!("\"id\": \"{r}\""), 1));
        let m = man_err(&t);
        assert!(m.starts_with("id:") && m.contains("reserved"), "{m}");
    }
}

#[test]
fn rejects_bad_version() {
    for bad in ["1.2", "1.2.3+b", "01.2.3", "x", ""] {
        let t = with(|s| s.replacen("\"1.2.0\"", &format!("\"{bad}\""), 1));
        assert!(man_err(&t).starts_with("version:"), "{bad}");
    }
}

#[test]
fn rejects_bad_range_with_key_path() {
    let t = with(|s| s.replace("^1.2", "^1.x"));
    assert_eq!(man_err(&t), "dependencies[0].req: `^1.x` is not a range");
    let t = with(|s| {
        s.replace(
            "{ \"id\": \"core_items\", \"req\": \"^1.2\" }",
            "{ \"id\": \"a\", \"req\": \"1\" }, { \"id\": \"b\", \"req\": \"1\" }, { \"id\": \"c\", \"req\": \"^1.x\" }",
        )
    });
    assert_eq!(man_err(&t), "dependencies[2].req: `^1.x` is not a range");
}

#[test]
fn rejects_duplicate_and_self_dependencies() {
    let two = "{ \"id\": \"a\", \"req\": \"1\" }, { \"id\": \"a\", \"req\": \"2\" }";
    let t = with(|s| s.replace("{ \"id\": \"core_items\", \"req\": \"^1.2\" }", two));
    assert_eq!(man_err(&t), "dependencies[1].id: `a` is listed twice");
    let t = with(|s| s.replace("\"core_items\"", "\"inventory\""));
    assert!(man_err(&t).starts_with("dependencies[0].id:"));
    let t = with(|s| s.replace("\"core_items\"", "\"Bad\""));
    assert!(man_err(&t).starts_with("dependencies[0].id:"));
}

#[test]
fn rejects_too_many_dependencies() {
    let deps: Vec<String> = (0..65).map(|i| format!("{{\"id\":\"d{i}\",\"req\":\"1\"}}")).collect();
    let t = with(|s| s.replace("{ \"id\": \"core_items\", \"req\": \"^1.2\" }", &deps.join(",")));
    assert!(man_err(&t).starts_with("dependencies:"));
    let t = with(|s| s.replace("{ \"id\": \"core_items\", \"req\": \"^1.2\" }", &deps[..64].join(",")));
    assert!(Manifest::from_json(&t).is_ok());
}

#[test]
fn rejects_bad_namespaces() {
    for bad in ["", "a..b", "a.", ".a", "a-b", "1a"] {
        let t = with(|s| s.replace("\"engine.world\"", &format!("\"{bad}\"")));
        assert!(man_err(&t).starts_with("permissions.namespaces[1]:"), "{bad}");
    }
    let t = with(|s| s.replace("\"engine.world\"", "\"engine.log\""));
    assert_eq!(man_err(&t), "permissions.namespaces[1]: `engine.log` is listed twice");
    let list: Vec<String> = (0..65).map(|i| format!("\"n{i}\"")).collect();
    let t = with(|s| s.replace("\"engine.log\", \"engine.world\"", &list.join(",")));
    assert!(man_err(&t).starts_with("permissions.namespaces:"));
}

#[test]
fn rejects_budget_and_memory_out_of_range() {
    for bad in ["0", "1000000000001", "-1", "1.5", "\"7\""] {
        let t = with(|s| s.replace("2000000", bad));
        assert!(man_err(&t).starts_with("permissions.budget:"), "{bad}");
    }
    let t = with(|s| s.replace("2000000", "1000000000000"));
    assert!(Manifest::from_json(&t).is_ok());
    for bad in ["0", "65537", "4294967296", "-1", "true"] {
        let t = with(|s| s.replace(": 64", &format!(": {bad}")));
        assert!(man_err(&t).starts_with("permissions.memory_pages:"), "{bad}");
    }
    let t = with(|s| s.replace(": 64", ": 65536"));
    assert!(Manifest::from_json(&t).is_ok());
}

#[test]
fn rejects_api_version_out_of_range() {
    for bad in ["0", "4294967296", "-1", "1.0", "\"1\""] {
        let t = with(|s| s.replace("\"api_version\": 1", &format!("\"api_version\": {bad}")));
        assert!(man_err(&t).starts_with("api_version:"), "{bad}");
    }
    let t = with(|s| s.replace("\"api_version\": 1", "\"api_version\": 4294967295"));
    assert_eq!(Manifest::from_json(&t).unwrap().api_version, u32::MAX);
}

#[test]
fn rejects_wrong_format() {
    let t = with(|s| s.replace("\"efmod\"", "\"zip\""));
    assert!(man_err(&t).starts_with("format:"));
    let t = with(|s| s.replace("\"efmod\"", "1"));
    assert!(man_err(&t).starts_with("format:"));
    let t = with(|s| s.replace("\"format_version\": 1", "\"format_version\": 2"));
    assert!(man_err(&t).starts_with("format_version:"));
    let t = with(|s| s.replace("\"format_version\": 1", "\"format_version\": \"1\""));
    assert!(man_err(&t).starts_with("format_version:"));
}

#[test]
fn rejects_missing_required_keys() {
    for key in ["format", "format_version", "id", "version"] {
        let t = with(|s| {
            s.lines().filter(|l| !l.trim_start().starts_with(&format!("\"{key}\""))).collect::<Vec<_>>().join("\n")
        });
        assert_eq!(man_err(&t), format!("{key}: required"));
    }
    let t = with(|s| s.replace("\"id\": \"core_items\", ", ""));
    assert_eq!(man_err(&t), "dependencies[0].id: required");
    let t = with(|s| s.replace(", \"req\": \"^1.2\"", ""));
    assert_eq!(man_err(&t), "dependencies[0].req: required");
}

#[test]
fn rejects_wrong_json_types() {
    let cases: &[(&str, &str, &str)] = &[
        ("\"id\": \"inventory\"", "\"id\": 5", "id:"),
        ("\"version\": \"1.2.0\"", "\"version\": 1", "version:"),
        ("\"name\": \"Inventory\"", "\"name\": []", "name:"),
        ("\"description\": \"Items, bags and a shop.\"", "\"description\": null", "description:"),
        ("[\"Efe\"]", "\"Efe\"", "authors:"),
        ("[\"Efe\"]", "[1]", "authors[0]:"),
        ("\"license\": \"MIT\"", "\"license\": true", "license:"),
        ("\"game\": \"script_demo\"", "\"game\": {}", "game:"),
        ("[\n    { \"id\": \"core_items\", \"req\": \"^1.2\" }\n  ]", "{}", "dependencies:"),
        ("{ \"id\": \"core_items\", \"req\": \"^1.2\" }", "\"core_items\"", "dependencies[0]:"),
        ("\"req\": \"^1.2\"", "\"req\": 1", "dependencies[0].req:"),
        ("[\"engine.log\", \"engine.world\"]", "\"engine.log\"", "permissions.namespaces:"),
        ("[\"engine.log\", \"engine.world\"]", "[3]", "permissions.namespaces[0]:"),
    ];
    for (from, to, prefix) in cases {
        let t = with(|s| s.replace(from, to));
        assert_ne!(t, example_text(), "{from}");
        let m = man_err(&t);
        assert!(m.starts_with(prefix), "{m} should start with {prefix}");
    }
    let t = with(|s| s.replace("\"permissions\": {", "\"permissions\": [] , \"x\": {"));
    assert!(man_err(&t).starts_with("permissions:"));
    assert!(man_err("[]").contains("object"));
    assert!(man_err("").contains("JSON"));
    assert!(man_err("{\"a\":1,\"a\":2}").contains("JSON"));
}

#[test]
fn rejects_length_and_content_limits() {
    let t = with(|s| s.replace("\"Inventory\"", "\"\""));
    assert!(man_err(&t).starts_with("name:"));
    let t = with(|s| s.replace("\"Inventory\"", &format!("\"{}\"", "x".repeat(129))));
    assert!(man_err(&t).starts_with("name:"));
    let t = with(|s| s.replace("\"Inventory\"", "\"a\\u0007b\""));
    assert!(man_err(&t).starts_with("name:"));
    let t = with(|s| s.replace("Items, bags and a shop.", &"d".repeat(2049)));
    assert!(man_err(&t).starts_with("description:"));
    let t = with(|s| s.replace("Items, bags and a shop.", &"d".repeat(2048)));
    assert!(Manifest::from_json(&t).is_ok());
    let t = with(|s| s.replace("[\"Efe\"]", "[\"\"]"));
    assert!(man_err(&t).starts_with("authors[0]:"));
    let many: Vec<String> = (0..33).map(|i| format!("\"a{i}\"")).collect();
    let t = with(|s| s.replace("[\"Efe\"]", &format!("[{}]", many.join(","))));
    assert!(man_err(&t).starts_with("authors:"));
    let t = with(|s| s.replace("\"MIT\"", &format!("\"{}\"", "l".repeat(65))));
    assert!(man_err(&t).starts_with("license:"));
    let t = with(|s| s.replace("\"script_demo\"", "\"\""));
    assert!(man_err(&t).starts_with("game:"));
    let t = with(|s| s.replace("\"script_demo\"", &format!("\"{}\"", "g".repeat(65))));
    assert!(man_err(&t).starts_with("game:"));
}

#[test]
fn validate_checks_constructed_manifests() {
    let mut m = manifest();
    assert!(m.validate().is_ok());
    m.id = "Bad".into();
    assert!(matches!(m.validate(), Err(PackageError::Manifest(_))));
    let mut m = manifest();
    m.api_version = 0;
    assert!(m.validate().is_err());
    let mut m = manifest();
    m.permissions.budget = Some(0);
    assert!(m.validate().is_err());
}

// ---- meta ---------------------------------------------------------------------------

use efemod::{MAX_META_BYTES, MAX_META_DEPTH};
use efevm::api::json::Value;

fn num(n: &str) -> Value {
    Value::Num(n.into())
}

/// A meta object of exactly `depth` levels (the object itself is depth 1).
fn deep_meta(depth: usize) -> Value {
    let mut v = num("1");
    for _ in 0..depth - 1 {
        v = Value::Arr(vec![v]);
    }
    Value::Obj(vec![("k".into(), v)])
}

fn sample_meta() -> Value {
    Value::Obj(vec![
        ("title".into(), Value::Str("Tab\there \"quoted\" \u{1} é".into())),
        ("count".into(), num("3")),
        ("ratio".into(), num("-2.5e10")),
        ("flag".into(), Value::Bool(false)),
        ("nothing".into(), Value::Null),
        ("tags".into(), Value::Arr(vec![Value::Str("a".into()), Value::Obj(vec![])])),
        ("nested".into(), Value::Obj(vec![("z".into(), num("1.0")), ("a".into(), num("0"))])),
    ])
}

#[test]
fn meta_round_trips_through_json() {
    let mut m = example();
    m.meta = Some(sample_meta());
    let text = m.to_json();
    assert!(text.ends_with(
        "\"meta\": {\"title\":\"Tab\\there \\\"quoted\\\" \\u0001 é\",\"count\":3,\"ratio\":-2.5e10,\"flag\":false,\"nothing\":null,\"tags\":[\"a\",{}],\"nested\":{\"z\":1.0,\"a\":0}}\n}\n"
    ), "{text}");
    let back = Manifest::from_json(&text).unwrap();
    assert_eq!(back, m);
    assert_eq!(back.to_json(), text);
}

#[test]
fn meta_is_optional_and_last() {
    assert!(!example().to_json().contains("\"meta\""));
    let mut m = example();
    m.meta = Some(Value::Obj(vec![]));
    let text = m.to_json();
    assert!(text.ends_with("  \"meta\": {}\n}\n"), "{text}");
    assert_eq!(Manifest::from_json(&text).unwrap().meta, Some(Value::Obj(vec![])));
}

#[test]
fn meta_must_be_an_object() {
    for bad in ["\"meta\": 3", "\"meta\": [1]", "\"meta\": null", "\"meta\": \"x\""] {
        let t = with(|s| s.replace("\"api_version\": 1,", &format!("\"api_version\": 1, {bad},")));
        assert!(man_err(&t).starts_with("meta: must be a JSON object"), "{bad}");
    }
    let mut m = example();
    m.meta = Some(num("1"));
    assert!(m.validate().unwrap_err().to_string().contains("meta: must be a JSON object"));
}

#[test]
fn meta_depth_limit() {
    let mut m = example();
    m.meta = Some(deep_meta(MAX_META_DEPTH));
    m.validate().unwrap();
    assert_eq!(Manifest::from_json(&m.to_json()).unwrap().meta, m.meta);
    m.meta = Some(deep_meta(MAX_META_DEPTH + 1));
    let e = m.validate().unwrap_err().to_string();
    assert!(e.contains("meta: nesting depth 17 is more than 16"), "{e}");
    let t = m.to_json();
    assert!(man_err(&t).starts_with("meta: nesting depth 17"));
}

#[test]
fn meta_size_limit() {
    let at_limit = |n: usize| {
        let mut m = example();
        m.meta = Some(Value::Obj(vec![("k".into(), Value::Str("a".repeat(n)))]));
        m
    };
    // `{"k":"` + n bytes + `"}` is n + 8 bytes of canonical JSON.
    let ok = at_limit(MAX_META_BYTES - 8);
    ok.validate().unwrap();
    assert_eq!(Manifest::from_json(&ok.to_json()).unwrap(), ok);
    let big = at_limit(MAX_META_BYTES - 7);
    let e = big.validate().unwrap_err().to_string();
    assert!(e.contains("meta:") && e.contains("at most 65536"), "{e}");
    assert!(man_err(&big.to_json()).starts_with("meta:"));
}

#[test]
fn meta_survives_package_write_and_read() {
    let mut m = example_without_deps();
    m.meta = Some(sample_meta());
    let bytes = Package::write(&m, &module(), &assets()).unwrap();
    let p = read(&bytes).unwrap();
    assert_eq!(p.manifest.meta, Some(sample_meta()));
    assert_eq!(p.manifest, m);
}

// ---- package round trip -------------------------------------------------------------

#[test]
fn round_trip() {
    let mb = module();
    let bytes = Package::write(&example_without_deps(), &mb, &assets()).unwrap();
    let p = read(&bytes).unwrap();
    assert_eq!(p.manifest, example_without_deps());
    assert_eq!(p.module_bytes, mb);
    assert_eq!(p.module.encode(), mb);
    assert_eq!(p.assets, assets());
    assert!(p.ignored.is_empty());
    assert_eq!(p.asset("items.json"), Some(&b"[1,2,3]"[..]));
    assert_eq!(p.asset("img/a.png").map(|a| a.len()), Some(500));
    assert_eq!(p.asset("nope"), None);
    let again = Package::write(&example_without_deps(), &mb, &assets()).unwrap();
    assert_eq!(bytes, again);
    // Rewriting what was read gives the same bytes.
    assert_eq!(Package::write(&p.manifest, &p.module_bytes, &p.assets).unwrap(), bytes);
}

fn example_without_deps() -> Manifest {
    let mut m = example();
    m.dependencies.clear();
    m.permissions.namespaces = vec!["engine.log".into(), "engine.world".into()];
    m
}

#[test]
fn canonical_entry_order() {
    let bytes = Package::write(&manifest(), &module(), &assets()).unwrap();
    let names: Vec<String> = container::read(&bytes, &ReadLimits::default()).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["manifest.json", "module.efvm", "assets/Zed.txt", "assets/img/a.png", "assets/items.json"]);
}

#[test]
fn write_validates_manifest_and_asset_names() {
    let mut m = manifest();
    m.id = "Bad".into();
    assert!(matches!(Package::write(&m, &module(), &BTreeMap::new()), Err(PackageError::Manifest(_))));
    for bad in ["../x", "a/../b", "C:x", "a\\b", "", "/abs", "a//b", "dir/", "a\u{1}b"] {
        let mut a = BTreeMap::new();
        a.insert(bad.to_string(), vec![1]);
        assert!(
            matches!(Package::write(&manifest(), &module(), &a), Err(PackageError::BadEntryName { .. })),
            "{bad:?}"
        );
    }
    let mut a = BTreeMap::new();
    a.insert("A.png".to_string(), vec![1]);
    a.insert("a.png".to_string(), vec![2]);
    assert!(matches!(Package::write(&manifest(), &module(), &a), Err(PackageError::BadEntryName { .. })));
}

// ---- reader rules -------------------------------------------------------------------

#[test]
fn missing_entries() {
    let b = container::write(&[("module.efvm", &module())]);
    assert_eq!(read(&b).unwrap_err(), PackageError::Missing("manifest.json"));
    let b = container::write(&[("manifest.json", manifest().to_json().as_bytes())]);
    assert_eq!(read(&b).unwrap_err(), PackageError::Missing("module.efvm"));
    let b = container::write(&[]);
    assert!(matches!(read(&b), Err(PackageError::Missing(_))));
}

#[test]
fn bad_manifest_and_module_in_archive() {
    let b = archive("{", &module(), &[]);
    assert!(matches!(read(&b), Err(PackageError::Manifest(_))));
    let b = container::write(&[("manifest.json", &[0xff, 0xfe][..]), ("module.efvm", &module())]);
    assert!(matches!(read(&b), Err(PackageError::Manifest(_))));
    let b = archive(&manifest().to_json(), b"not a module", &[]);
    assert!(matches!(read(&b), Err(PackageError::Module(_))));
}

#[test]
fn undeclared_import_is_listed_sorted() {
    let mb = module_with(&[("engine.world", "x"), ("engine.log", "info"), ("zeta", "f"), ("engine.world", "y")], false);
    let b = archive(&manifest().to_json(), &mb, &[]);
    assert_eq!(
        read(&b).unwrap_err(),
        PackageError::UndeclaredImport { namespaces: vec!["engine.world".into(), "zeta".into()] }
    );
}

#[test]
fn prefix_rule_needs_dot_boundary() {
    // `engine.lo` must not cover `engine.log`.
    let mut m = manifest();
    m.permissions.namespaces = vec!["engine.lo".into()];
    let mb = module_with(&[("engine.log", "info")], false);
    let b = archive(&m.to_json(), &mb, &[]);
    assert!(matches!(read(&b), Err(PackageError::UndeclaredImport { .. })));
    // A prefix covers deeper namespaces.
    m.permissions.namespaces = vec!["engine".into()];
    assert!(read(&archive(&m.to_json(), &mb, &[])).is_ok());
}

#[test]
fn own_namespace_import_is_a_manifest_error() {
    let mut m = manifest();
    m.permissions.namespaces.push("inventory".into());
    let mb = module_with(&[("inventory", "f")], false);
    let b = archive(&m.to_json(), &mb, &[]);
    assert!(matches!(read(&b), Err(PackageError::Manifest(_))));
    let mb = module_with(&[("inventory.sub", "f")], false);
    let b = archive(&m.to_json(), &mb, &[]);
    assert!(matches!(read(&b), Err(PackageError::Manifest(_))));
    // A longer id sharing the text prefix is a different namespace.
    let mb = module_with(&[("inventory2", "f")], false);
    m.permissions.namespaces.push("inventory2".into());
    assert!(read(&archive(&m.to_json(), &mb, &[])).is_ok());
}

#[test]
fn dependency_namespace_import_is_accepted() {
    let mut m = Manifest::new("inventory", Version::new(1, 0, 0));
    m.dependencies = vec![Dependency { id: "core_items".into(), req: "^1".parse().unwrap() }];
    let mb = module_with(&[("core_items", "a"), ("core_items.x", "b")], false);
    let b = archive(&m.to_json(), &mb, &[]);
    assert!(read(&b).is_ok());
    let module = efevm::Module::decode(&mb).unwrap();
    assert!(check_module(&m, &module).is_ok());
    // Not covered: a sibling id sharing the text prefix.
    let mb = module_with(&[("core_items_extra", "a")], false);
    let module = efevm::Module::decode(&mb).unwrap();
    assert_eq!(
        check_module(&m, &module),
        Err(PackageError::UndeclaredImport { namespaces: vec!["core_items_extra".into()] })
    );
}

#[test]
fn ignored_entries_are_listed() {
    let extra: &[(&str, &[u8])] = &[
        ("assets/a.txt", b"a"),
        ("__MACOSX/._x", b"junk"),
        ("README.md", b"hi"),
        ("assets_not/x", b"y"),
    ];
    let b = archive(&manifest().to_json(), &module(), extra);
    let p = read(&b).unwrap();
    assert_eq!(p.ignored, ["__MACOSX/._x", "README.md", "assets_not/x"]);
    assert_eq!(p.assets.keys().collect::<Vec<_>>(), ["a.txt"]);
}

#[test]
fn bad_entry_names_are_rejected_even_when_ignored() {
    let b = archive(&manifest().to_json(), &module(), &[("../evil", b"x")]);
    assert!(matches!(read(&b), Err(PackageError::BadEntryName { .. })));
}

#[test]
fn limits_apply() {
    let b = Package::write(&manifest(), &module(), &assets()).unwrap();
    let small = ReadLimits { file_size: 10, ..ReadLimits::default() };
    assert!(matches!(Package::read(&b, &small), Err(PackageError::TooLarge { .. })));
    let small = ReadLimits { asset_size: 100, ..ReadLimits::default() };
    assert!(matches!(Package::read(&b, &small), Err(PackageError::TooLarge { .. })));
}

#[test]
fn index_owner_is_the_id() {
    let b = Package::write(&manifest(), &module(), &BTreeMap::new()).unwrap();
    let p = read(&b).unwrap();
    let ix = p.index();
    assert_eq!(ix.game, "inventory");
    assert_eq!(ix.namespaces.len(), 1);
    let ns = &ix.namespaces[0];
    assert_eq!((ns.name.as_str(), ns.owner.as_str(), ns.version), ("inventory", "inventory", 1));
    assert_eq!(ns.doc, "Functions provided by the mod `inventory`.");
    assert_eq!(ix.functions.len(), 1);
    assert_eq!(ix.functions[0].qualified(), "inventory.run");
    assert_eq!(ix.functions[0].owner, "inventory");

    let mut m = manifest();
    m.description = Some("Docs here.".into());
    m.api_version = 3;
    let p = read(&Package::write(&m, &module(), &BTreeMap::new()).unwrap()).unwrap();
    assert_eq!(p.index().namespaces[0].doc, "Docs here.");
    assert_eq!(p.index().namespaces[0].version, 3);
}

#[test]
fn index_without_interface_has_only_the_namespace() {
    let mb = module_with(&[], false);
    let p = read(&archive(&manifest().to_json(), &mb, &[])).unwrap();
    let ix = p.index();
    assert_eq!(ix.namespaces.len(), 1);
    assert_eq!(ix.namespaces[0].name, "inventory");
    assert!(ix.functions.is_empty() && ix.types.is_empty());
}

// ---- never panics -------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, data: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(4) {
        match rng.below(4) {
            0 if !data.is_empty() => {
                let i = rng.below(data.len());
                data[i] ^= 1 << rng.below(8);
            }
            1 if !data.is_empty() => {
                let n = rng.below(data.len());
                data.truncate(n);
            }
            2 => {
                let i = rng.below(data.len() + 1);
                let b = rng.next() as u8;
                data.insert(i, b);
            }
            _ if !data.is_empty() => {
                let i = rng.below(data.len());
                data[i] = b"{}[]\",:\\0123 \0"[rng.below(14)];
            }
            _ => {}
        }
    }
}

#[test]
fn never_panics_on_mutated_manifests() {
    let mb = module();
    let base = example_without_deps().to_json().into_bytes();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut ok = 0;
    for _ in 0..3000 {
        let mut t = base.clone();
        mutate(&mut rng, &mut t);
        // Text level (lossy keeps it a &str) and byte level inside a valid archive.
        let s = String::from_utf8_lossy(&t).into_owned();
        let _ = Manifest::from_json(&s);
        let b = container::write(&[("manifest.json", &t), ("module.efvm", &mb)]);
        if read(&b).is_ok() {
            ok += 1;
        }
    }
    assert!(ok > 0, "some mutations should still be valid");
}

#[test]
fn never_panics_on_mutated_archives() {
    let valid = Package::write(&example_without_deps(), &module(), &assets()).unwrap();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for _ in 0..3000 {
        let mut b = valid.clone();
        mutate(&mut rng, &mut b);
        let _ = read(&b);
    }
    for n in 0..valid.len().min(400) {
        let _ = read(&valid[..n]);
    }
}
