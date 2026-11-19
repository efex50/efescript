//! Typed script events (E8.13): `EventDesc` in the registry and the API index (format
//! version 2), section 14 `events` (SPEC.md 9.3) and the link check of event references.

use efevm::binary::{decode, encode};
use efevm::*;

const HEADER: &[u8] = b"EFVM\x01\x00";

fn ok(_: &mut Caller<'_, ()>, _: &Args<'_>, _: &mut Results<'_>) -> Result<HostOk, Trap> {
    Ok(HostOk::Continue)
}

fn score(owner: &str) -> StructDesc {
    let mut s = StructDesc::new("Score", 8, 4)
        .field(FieldDesc::new("player", 0, Prim::U32, 1))
        .field(FieldDesc::new("points", 4, Prim::I32, 1))
        .doc("A score change.");
    s.owner = owner.into();
    s
}

/// `game` owns `Score`, `game.score_changed(&Score)` and `game.reset` (no payload);
/// `game.ui` owns `game.ui.z`, and there is a function `game.add`.
fn registry() -> HostRegistry<()> {
    let mut r = HostRegistry::new();
    r.set_game("demo");
    r.define_namespace("game", 2, "game", "The game.").unwrap();
    r.define_namespace("game.ui", 1, "game", "The UI.").unwrap();
    r.register_type(score("game")).unwrap();
    r.register(HostFn::new("game", "add").doc("Add.").param("a", ApiType::I32), ok).unwrap();
    r.register_event(EventDesc::new("game", "score_changed").payload("Score").doc("A player's score changed.")).unwrap();
    r.register_event(EventDesc::new("game", "reset").doc("The round was reset.")).unwrap();
    r.register_event(EventDesc::new("game.ui", "z").doc("A UI event.")).unwrap();
    r
}

/// `(namespace, name, payload)` of one section 14 entry.
type Ref<'a> = (&'a str, &'a str, Option<(&'a str, u32)>);

fn module_with(events: &[Ref<'_>]) -> Module {
    let mut b = ModuleBuilder::new();
    for (ns, name, p) in events {
        b.event_ref(ns, name, *p);
    }
    b.finish().expect("module")
}

fn decode_err(bytes: &[u8]) -> DecodeError {
    match decode(bytes) {
        Err(LoadError::Decode { error, .. }) => error,
        other => panic!("expected a decode error, got {other:?}"),
    }
}

fn events_error(m: ModuleData) -> (String, String) {
    match Module::new(m) {
        Err(LoadError::Verify { error: VerifyError::Events { what, reason }, .. }) => (what, reason),
        other => panic!("expected an events error, got {other:?}"),
    }
}

fn uleb(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

fn st(s: &str) -> Vec<u8> {
    let mut v = uleb(s.len() as u64);
    v.extend_from_slice(s.as_bytes());
    v
}

fn section14(payload: &[u8]) -> Vec<u8> {
    let mut b = HEADER.to_vec();
    b.push(14);
    b.extend(uleb(payload.len() as u64));
    b.extend_from_slice(payload);
    b
}

// ---------------------------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------------------------

#[test]
fn registry_event_rules() {
    let mut r = registry();
    let e = r.event("game", "score_changed").unwrap();
    assert_eq!((e.owner.as_str(), e.payload.as_deref()), ("game", Some("Score")));
    assert_eq!(r.events().map(EventDesc::qualified).collect::<Vec<_>>(), ["game.reset", "game.score_changed", "game.ui.z"]);
    let reg = |r: &mut HostRegistry<()>, e: EventDesc| r.register_event(e);
    assert_eq!(
        reg(&mut r, EventDesc::new("nope", "x").doc("d")),
        Err(RegistryError::UnknownEventNamespace { event: "nope.x".into() })
    );
    assert_eq!(reg(&mut r, EventDesc::new("game", "1x").doc("d")), Err(RegistryError::InvalidEventName { event: "game.1x".into() }));
    assert_eq!(reg(&mut r, EventDesc::new("game", "x").doc(" \n")), Err(RegistryError::EmptyEventDoc { event: "game.x".into() }));
    assert_eq!(
        reg(&mut r, EventDesc::new("game", "x").doc("d").payload("Nope")),
        Err(RegistryError::UnknownEventPayload { event: "game.x".into(), name: "Nope".into() })
    );
    assert_eq!(reg(&mut r, EventDesc::new("game", "reset").doc("d")), Err(RegistryError::DuplicateEvent { event: "game.reset".into() }));
    // An event and a function may not share a full name, in either order.
    assert_eq!(reg(&mut r, EventDesc::new("game", "add").doc("d")), Err(RegistryError::EventFunctionClash { name: "game.add".into() }));
    assert_eq!(
        r.register(HostFn::new("game", "reset").doc("x"), ok),
        Err(RegistryError::EventFunctionClash { name: "game.reset".into() })
    );
    assert_eq!(
        r.replace(HostFn::new("game", "reset").doc("x"), ok),
        Err(RegistryError::EventFunctionClash { name: "game.reset".into() })
    );
    // replace_event
    assert_eq!(r.replace_event(EventDesc::new("game", "nope").doc("d")), Err(RegistryError::NoSuchEvent { event: "game.nope".into() }));
    r.replace_event(EventDesc::new("game", "reset").doc("Reset again.").payload("Score")).unwrap();
    assert_eq!(r.event("game", "reset").unwrap().payload.as_deref(), Some("Score"));
    assert!(r.replace_event(EventDesc::new("game", "reset").doc("")).is_err());
    // Messages name the event.
    assert_eq!(RegistryError::EventFunctionClash { name: "game.add".into() }.to_string(), "game.add is both a function and an event");
    assert_eq!(
        RegistryError::UnknownEventPayload { event: "game.x".into(), name: "Nope".into() }.to_string(),
        "event game.x: payload struct `Nope` is not registered"
    );
}

#[test]
fn unregister_owner_removes_events_and_guards_payload_structs() {
    let mut r = registry();
    r.define_namespace("modx", 1, "modx", "A mod.").unwrap();
    let mut loot = StructDesc::new("ModLoot", 4, 4);
    loot.owner = "modx".into();
    r.register_type(loot).unwrap();
    r.register_event(EventDesc::new("modx", "found").payload("ModLoot").doc("Found.")).unwrap();
    r.register_event(EventDesc::new("modx", "uses_game").payload("Score").doc("Uses a game struct.")).unwrap();
    // A game event carrying the mod's struct blocks the unload.
    r.register_event(EventDesc::new("game", "looted").payload("ModLoot").doc("Looted.")).unwrap();
    assert_eq!(
        r.unregister_owner("modx"),
        Err(RegistryError::StructInUseByEvent { name: "ModLoot".into(), event: "game.looted".into() })
    );
    assert!(r.event("modx", "found").is_some());
    // Drop the blocker, then the mod unloads: namespace + 2 events + 1 struct.
    r.replace_event(EventDesc::new("game", "looted").doc("Looted.")).unwrap();
    assert_eq!(r.unregister_owner("modx"), Ok(4));
    assert!(r.event("modx", "found").is_none() && r.struct_type("ModLoot").is_none());
    assert_eq!(r.events().count(), 4);
    // Unloading the game removes its events too.
    let before = r.events().count();
    assert!(before > 0);
    let n = r.unregister_owner("game").unwrap();
    assert_eq!(r.events().count(), 0);
    assert!(n >= before);
}

#[test]
fn index_and_imports_carry_events() {
    let r = registry();
    let ix = r.index();
    ix.validate().unwrap();
    assert_eq!(ix.events.len(), 3);
    assert_eq!(ix.event("game", "score_changed").unwrap().owner, "game");
    // The registry's order is the index's sort order.
    let mut sorted = ix.clone();
    sorted.sort();
    assert_eq!(sorted, ix);
    assert_eq!(ApiIndex::from_json(&ix.to_json()), Ok(ix.clone()));
    let imp = r.imports();
    assert_eq!(imp.event("game", "score_changed"), Some(Some(("Score", 8))));
    assert_eq!(imp.event("game", "reset"), Some(None));
    assert_eq!(imp.event("game", "nope"), None);
    assert_eq!(imp.events().count(), 3);
    assert_eq!(imp.api_hash(), Some(ix.hash()));
}

#[test]
fn sort_is_by_namespace_then_name() {
    // Tuple order and full-name string order differ here: ("game", "z") < ("game.ui", "a")
    // but "game.ui.a" < "game.z". Registry, sort and writer all use the tuple order.
    let mut r = HostRegistry::<()>::new();
    r.define_namespace("game", 1, "game", "G.").unwrap();
    r.define_namespace("game.ui", 1, "game", "U.").unwrap();
    r.register_event(EventDesc::new("game.ui", "a").doc("A.")).unwrap();
    r.register_event(EventDesc::new("game", "z").doc("Z.")).unwrap();
    let ix = r.index();
    assert_eq!(ix.events.iter().map(EventDesc::qualified).collect::<Vec<_>>(), ["game.z", "game.ui.a"]);
    let mut rev = ix.clone();
    rev.events.reverse();
    rev.sort();
    assert_eq!(rev, ix);
    let back = ApiIndex::from_json(&ix.to_json()).unwrap();
    assert_eq!(back, ix);
    assert!(ix.to_json().find("\"z\"").unwrap() < ix.to_json().find("\"a\"").unwrap());
}

// ---------------------------------------------------------------------------------------------
// Index: validate, hash, merge, JSON
// ---------------------------------------------------------------------------------------------

fn invalid(ix: &ApiIndex) -> String {
    match ix.validate() {
        Err(ApiError::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn index_validate_events() {
    let base = registry().index();
    let with = |f: &dyn Fn(&mut ApiIndex)| {
        let mut x = base.clone();
        f(&mut x);
        x
    };
    assert!(invalid(&with(&|x| x.events[0].namespace = "nope".into())).contains("namespace `nope` is not listed"));
    assert!(invalid(&with(&|x| x.events[0].name = "a-b".into())).contains("not an identifier"));
    assert!(invalid(&with(&|x| x.events[1] = x.events[0].clone())).contains("duplicate event game.reset"));
    assert!(invalid(&with(&|x| x.events[0].name = "add".into())).contains("full name of a function"));
    assert!(invalid(&with(&|x| x.events[1].payload = Some("Gone".into()))).contains("payload struct `Gone` is not listed"));
    // A hash covers every event field but the game name.
    for f in [
        &(|x: &mut ApiIndex| x.events[0].doc.push('!')) as &dyn Fn(&mut ApiIndex),
        &|x| x.events[0].owner = "other".into(),
        &|x| x.events[1].payload = None,
        &|x| {
            x.events.pop();
        },
    ] {
        assert_ne!(with(f).hash(), base.hash());
    }
}

#[test]
fn index_version_2_only() {
    let j = registry().index().to_json();
    assert!(j.starts_with("{\n  \"format\": \"efapi\",\n  \"version\": 2,\n"));
    assert!(j.contains(
        "  \"events\": [\n    { \"namespace\": \"game\", \"name\": \"reset\", \"owner\": \"game\", \"payload\": null, \"doc\": \"The round was reset.\" },\n"
    ));
    assert!(j.contains("\"payload\": \"Score\""));
    for v in ["1", "0", "3", "2.0"] {
        let t = j.replacen("\"version\": 2", &format!("\"version\": {v}"), 1);
        assert!(matches!(ApiIndex::from_json(&t), Err(ApiError::Version(_))), "version {v}");
    }
    assert_eq!(ApiError::Version("1".into()).to_string(), "unsupported API index version 1 (this reader supports 2)");
}

#[test]
fn merge_events() {
    let a = registry().index();
    let mut b = registry().index();
    b.game = "other".into();
    let m = ApiIndex::merge(&[&a, &b]).unwrap();
    assert_eq!(m.events, a.events);
    // A new event in the second input is added.
    let mut c = b.clone();
    c.events.push(EventDesc { namespace: "game".into(), name: "won".into(), payload: None, doc: "Won.".into(), owner: "game".into() });
    assert_eq!(ApiIndex::merge(&[&a, &c]).unwrap().events.len(), 4);
    // Same name, other content: names the event and both games.
    let mut d = b.clone();
    d.events[0].doc = "Different.".into();
    match ApiIndex::merge(&[&a, &d]) {
        Err(ApiError::Invalid(m)) => assert_eq!(m, "event `game.reset` differs between `demo` and `other`"),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Section 14
// ---------------------------------------------------------------------------------------------

#[test]
fn section_14_round_trip_and_layout() {
    let m = module_with(&[("game", "score_changed", Some(("Score", 8))), ("game", "reset", None)]);
    assert_eq!(m.data().events.len(), 2);
    assert_eq!(m.data().events[0].payload_size(), 8);
    assert_eq!(m.data().events[1].payload_size(), 0);
    assert_eq!(m.data().events[0].qualified(), "game.score_changed");
    let payload = [
        &[2u8][..],
        &st("game"),
        &st("score_changed"),
        &[1],
        &st("Score"),
        &uleb(8),
        &st("game"),
        &st("reset"),
        &[0],
    ]
    .concat();
    assert_eq!(m.encode(), section14(&payload));
    assert_eq!(Module::decode(&section14(&payload)).unwrap().data(), m.data());
    // No events: no section.
    assert_eq!(encode(&ModuleData::default()), HEADER);
    // Large sizes round trip unverified.
    let big = ModuleData {
        events: vec![EventRef { namespace: "a".into(), name: "b".into(), payload: Some(("C".into(), u32::MAX)) }],
        ..Default::default()
    };
    assert_eq!(decode(&encode(&big)).unwrap(), big);
}

#[test]
fn builder_event_ref_ids() {
    let mut b = ModuleBuilder::new();
    assert_eq!(b.event_ref("game", "a", None), 0);
    assert_eq!(b.event_ref("game", "b", Some(("Score", 8))), 1);
    assert_eq!(b.event_ref("game", "a", None), 0);
    assert_eq!(b.event_ref("game", "b", Some(("Score", 8))), 1);
    assert_eq!(b.event_ref("other", "a", None), 2);
    let m = b.finish().unwrap();
    assert_eq!(m.data().event_namespaces(), ["game", "other"]);
    // The same event with another payload is a duplicate the verifier rejects.
    let mut b = ModuleBuilder::new();
    b.event_ref("game", "a", None);
    assert_eq!(b.event_ref("game", "a", Some(("S", 1))), 1);
    assert!(matches!(b.finish(), Err(LoadError::Verify { error: VerifyError::Events { .. }, .. })));
}

#[test]
fn section_14_verifier_rules() {
    let one = |ns: &str, name: &str, p: Option<(&str, u32)>| ModuleData {
        events: vec![EventRef { namespace: ns.into(), name: name.into(), payload: p.map(|(s, n)| (s.into(), n)) }],
        ..Default::default()
    };
    assert!(Module::new(one("game.ui", "x", Some(("S", 0)))).is_ok());
    let (w, r) = events_error(one("game..ui", "x", None));
    assert_eq!(w, "event 0 `game..ui.x`");
    assert!(r.contains("not a namespace name"), "{r}");
    assert!(events_error(one("", "x", None)).1.contains("not a namespace name"));
    assert!(events_error(one("game", "x y", None)).1.contains("name is not an identifier"));
    assert!(events_error(one("game", "x", Some(("&S", 1)))).1.contains("payload struct name `&S`"));
    let dup = ModuleData {
        events: vec![
            EventRef { namespace: "g".into(), name: "x".into(), payload: None },
            EventRef { namespace: "g".into(), name: "x".into(), payload: Some(("S".into(), 4)) },
        ],
        ..Default::default()
    };
    assert_eq!(events_error(dup), ("event 1 `g.x`".into(), "duplicate event reference".into()));
    let many = |n: usize| ModuleData {
        events: (0..n).map(|i| EventRef { namespace: "g".into(), name: format!("e{i}"), payload: None }).collect(),
        ..Default::default()
    };
    assert!(Module::new(many(1024)).is_ok());
    assert_eq!(events_error(many(1025)), ("events".into(), "1025 event references, at most 1024".into()));
    assert_eq!(
        VerifyError::Events { what: "events".into(), reason: "r".into() }.to_string(),
        "events: events: r"
    );
}

#[test]
fn section_14_decode_errors() {
    let good = [&[1u8][..], &st("g"), &st("e"), &[1], &st("S"), &uleb(4)].concat();
    assert!(decode(&section14(&good)).is_ok());
    // Bad payload flag.
    let mut p = good.clone();
    p[5] = 2;
    assert_eq!(decode_err(&section14(&p)), DecodeError::BadFlag(2));
    // Every strict prefix fails.
    for n in 0..good.len() {
        let e = decode_err(&section14(&good[..n]));
        assert!(matches!(e, DecodeError::UnexpectedEnd | DecodeError::CountTooLarge), "prefix {n}: {e:?}");
    }
    // Trailing bytes.
    let mut p = good.clone();
    p.push(0);
    assert_eq!(decode_err(&section14(&p)), DecodeError::SectionSizeMismatch(14));
    // A huge count is rejected without allocating.
    assert_eq!(decode_err(&section14(&uleb(u32::MAX as u64))), DecodeError::CountTooLarge);
    // Bad UTF-8, over-long size LEB.
    assert_eq!(decode_err(&section14(&[1, 1, 0xFF, 0, 0])), DecodeError::BadUtf8);
    let p = [&[1u8][..], &st("g"), &st("e"), &[1], &st("S"), &[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]].concat();
    assert_eq!(decode_err(&section14(&p)), DecodeError::LebTooLong);
    // Order: 14 after 13; twice is out of order; 15 is unknown.
    let iface = encode(&ModuleData { interface: Some(Interface::default()), ..Default::default() });
    let ev = section14(&good);
    let mut b = ev.clone();
    b.extend_from_slice(&iface[HEADER.len()..]);
    assert_eq!(decode_err(&b), DecodeError::SectionOutOfOrder(13));
    let mut b = iface.clone();
    b.extend_from_slice(&ev[HEADER.len()..]);
    assert!(decode(&b).is_ok());
    let mut b = ev.clone();
    b.extend_from_slice(&ev[HEADER.len()..]);
    assert_eq!(decode_err(&b), DecodeError::SectionOutOfOrder(14));
    assert_eq!(decode_err(&[HEADER, &[15, 0]].concat()), DecodeError::UnknownSection(15));
}

#[test]
fn mutated_section_14_never_panics() {
    let seed = module_with(&[("game", "score_changed", Some(("Score", 8))), ("game.ui", "z", None)]).encode();
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let imports = registry().imports();
    for _ in 0..20_000 {
        let mut b = seed.clone();
        for _ in 0..1 + next() % 3 {
            let i = (next() as usize) % b.len();
            match next() % 4 {
                0 => b[i] ^= 1 << (next() % 8),
                1 => b[i] = next() as u8,
                2 => b.truncate(i.max(1)),
                _ => b.insert(i, next() as u8),
            }
        }
        if let Ok(m) = Module::decode(&b) {
            let _ = imports.check_module(&m);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Link check
// ---------------------------------------------------------------------------------------------

#[test]
fn link_checks_event_refs() {
    let imports = registry().imports();
    let good = module_with(&[("game", "score_changed", Some(("Score", 8))), ("game", "reset", None)]);
    assert_eq!(imports.check_module(&good), vec![]);
    assert!(Instance::new(&good, &imports, Limits::default()).is_ok());

    let bad = module_with(&[
        ("game", "nope", None),
        ("game", "score_changed", Some(("Score", 12))),
        ("game", "reset", Some(("Score", 8))),
        ("game.ui", "z", None),
        ("game", "add", None),
    ]);
    let errs = imports.check_module(&bad);
    assert_eq!(
        errs,
        vec![
            LinkError::UnknownEvent { namespace: "game".into(), name: "nope".into() },
            LinkError::EventPayload {
                namespace: "game".into(),
                name: "score_changed".into(),
                module: "&Score (12 bytes)".into(),
                host: "&Score (8 bytes)".into(),
            },
            LinkError::EventPayload {
                namespace: "game".into(),
                name: "reset".into(),
                module: "&Score (8 bytes)".into(),
                host: "no payload".into(),
            },
            // A function is not an event.
            LinkError::UnknownEvent { namespace: "game".into(), name: "add".into() },
        ]
    );
    assert_eq!(errs[0].to_string(), "module uses event game.nope, which the host does not register");
    assert_eq!(
        errs[1].to_string(),
        "payload mismatch for event game.score_changed: module expects &Score (12 bytes), host has &Score (8 bytes)"
    );
    // Instance::new refuses with the first error.
    match Instance::new(&bad, &imports, Limits::default()) {
        Err(LoadError::Link(e)) => assert_eq!(e, errs[0]),
        other => panic!("{:?}", other.err()),
    }
    // Another payload struct name of the same size is a mismatch too.
    let renamed = module_with(&[("game", "score_changed", Some(("Points", 8)))]);
    assert!(matches!(imports.check_module(&renamed)[..], [LinkError::EventPayload { .. }]));
    // Empty imports know no events.
    assert_eq!(Imports::<()>::new().check_module(&good).len(), 2);
}

#[test]
fn event_errors_come_after_imports_and_before_the_hash() {
    let reg = registry();
    let imports = reg.imports();
    let mut b = ModuleBuilder::new();
    b.import("game", "missing", FuncType::new([], []));
    b.event_ref("game", "nope", None);
    b.api_ref(1, [("game", 2u32)]);
    let m = b.finish().unwrap();
    let errs = imports.check_module(&m);
    assert!(matches!(errs[0], LinkError::MissingImport { .. }), "{errs:?}");
    assert!(matches!(errs[1], LinkError::UnknownEvent { .. }), "{errs:?}");
    assert!(matches!(errs[2], LinkError::ApiHash { module: 1, .. }), "{errs:?}");
    assert_eq!(errs.len(), 3);
}
