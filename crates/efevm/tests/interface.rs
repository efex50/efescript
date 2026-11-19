//! Section 13 `interface` (SPEC.md 9.2): round trips, the exact byte layout, every decode
//! error, one rejecting test per verifier rule, and `Interface::host_fns` / `index`.

use efevm::binary::{decode, encode};
use efevm::*;

const HEADER: &[u8] = b"EFVM\x01\x00";

/// Function indices of `base()`: import 0, `take` 1, `twice` 2.
const IMPORT: u32 = 0;
const TAKE: u32 = 1;
const TWICE: u32 = 2;

fn item() -> StructDesc {
    StructDesc::new("Item", 8, 4)
        .field(FieldDesc::new("count", 0, Prim::U32, 1).with_doc("How many."))
        .field(FieldDesc::new("weight", 4, Prim::F32, 1))
        .doc("An item.")
}

fn transform() -> StructDesc {
    let mut t = StructDesc::new("Transform", 12, 4).field(FieldDesc::new("pos", 0, Prim::F32, 3)).doc("Position.");
    t.owner = "engine".into();
    t.component = Some(7);
    t
}

fn give() -> Provide {
    Provide {
        name: "give".into(),
        func: TAKE,
        doc: "Give an item.".into(),
        params: vec![Param::new("text", ApiType::Str), Param::new("item", ApiType::Struct("Item".into()))],
        results: vec![Param::new("n", ApiType::I64)],
    }
}

fn double() -> Provide {
    Provide {
        name: "double".into(),
        func: TWICE,
        doc: "Twice x.".into(),
        params: vec![Param::new("x", ApiType::I32)],
        results: vec![Param::new("y", ApiType::I32)],
    }
}

fn dropped() -> ProvidedEvent {
    ProvidedEvent { name: "dropped".into(), payload: Some("Item".into()), doc: "An item was dropped.".into() }
}

fn emptied() -> ProvidedEvent {
    ProvidedEvent { name: "emptied".into(), payload: None, doc: "The inventory is empty.".into() }
}

fn valid() -> Interface {
    Interface {
        scratch: Some((64, 256)),
        structs: vec![item(), transform()],
        provides: vec![give(), double()],
        events: vec![dropped(), emptied()],
    }
}

/// One import, two functions, one page of memory, a data segment at [16, 32).
fn base(iface: Option<Interface>) -> ModuleData {
    let mut b = ModuleBuilder::new();
    b.import("env", "host", FuncType::new([ValType::I32], [ValType::I32]));
    b.memory(1, None);
    b.data(16, &[7; 16]);
    let mut fb = FunctionBuilder::new(4);
    fb.ret(0);
    let take = b.add_func(FuncType::new([ValType::I64; 3], [ValType::I64]), fb);
    let mut fb = FunctionBuilder::new(2);
    fb.op3(Op::I32Add, 0, 0, 0);
    fb.ret(0);
    let twice = b.add_func(FuncType::new([ValType::I32], [ValType::I32]), fb);
    assert_eq!((take, twice), (TAKE, TWICE));
    if let Some(i) = iface {
        b.interface(i);
    }
    b.finish_unverified().expect("builds")
}

fn with(f: impl FnOnce(&mut Interface)) -> ModuleData {
    let mut i = valid();
    f(&mut i);
    base(Some(i))
}

/// The verifier's `(what, reason)` for a module that must be rejected by an interface rule.
fn rejected(m: ModuleData) -> (String, String) {
    match Module::new(m) {
        Err(LoadError::Verify { error: VerifyError::Interface { what, reason }, .. }) => (what, reason),
        other => panic!("expected an interface error, got {other:?}"),
    }
}

fn check(m: ModuleData, what: &str, reason: &str) {
    let (w, r) = rejected(m);
    assert_eq!(w, what, "reason was {r:?}");
    assert!(r.contains(reason), "{what}: {r:?} does not contain {reason:?}");
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

fn section13(payload: &[u8]) -> Vec<u8> {
    let mut b = HEADER.to_vec();
    b.push(13);
    b.extend(uleb(payload.len() as u64));
    b.extend_from_slice(payload);
    b
}

fn decode_err(bytes: &[u8]) -> DecodeError {
    match decode(bytes) {
        Err(LoadError::Decode { error, .. }) => error,
        other => panic!("expected a decode error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Round trips and layout
// ---------------------------------------------------------------------------------------------

#[test]
fn valid_interface_verifies_and_round_trips() {
    let m = base(Some(valid()));
    let module = Module::new(m.clone()).expect("verifies");
    let bytes = module.encode();
    assert_eq!(decode(&bytes).unwrap(), m);
    assert_eq!(Module::decode(&bytes).unwrap().data(), &m);
    assert_eq!(encode(&decode(&bytes).unwrap()), bytes);
}

#[test]
fn empty_interface_is_written_and_none_is_not() {
    let empty = ModuleData { interface: Some(Interface::default()), ..Default::default() };
    let bytes = encode(&empty);
    assert_eq!(bytes, [HEADER, &[13, 4, 0, 0, 0, 0]].concat());
    assert_eq!(decode(&bytes).unwrap(), empty);
    assert!(Module::new(empty).is_ok());
    assert_eq!(encode(&ModuleData::default()), HEADER);
    assert_eq!(decode(HEADER).unwrap().interface, None);
}

#[test]
fn every_api_type_and_prim_round_trips() {
    let mut s = StructDesc::new("All", 128, 8);
    for (i, p) in Prim::ALL.into_iter().enumerate() {
        s.fields.push(FieldDesc::new(format!("f{i}"), 8 * i as u32, p, 1));
    }
    let types = [
        ApiType::I32,
        ApiType::I64,
        ApiType::F32,
        ApiType::F64,
        ApiType::Bool,
        ApiType::Entity,
        ApiType::Str,
        ApiType::Bytes,
        ApiType::Struct("All".into()),
    ];
    let p = Provide {
        name: "p".into(),
        func: 0,
        doc: "d".into(),
        params: types.iter().enumerate().map(|(i, t)| Param::new(format!("a{i}"), t.clone())).collect(),
        results: types[..6].iter().enumerate().map(|(i, t)| Param::new(format!("r{i}"), t.clone())).collect(),
    };
    let m = ModuleData {
        interface: Some(Interface {
            scratch: Some((u64::MAX, u32::MAX)),
            structs: vec![s],
            provides: vec![p],
            events: vec![
                ProvidedEvent { name: "e".into(), payload: Some("All".into()), doc: "d".into() },
                ProvidedEvent::default(),
            ],
        }),
        ..Default::default()
    };
    assert_eq!(decode(&encode(&m)).unwrap(), m);
}

#[test]
fn byte_layout_matches_spec() {
    let mut s = StructDesc::new("S", 8, 4).field(FieldDesc::new("a", 4, Prim::F32, 1).with_doc("fa")).doc("ds");
    s.owner = "eng".into();
    s.component = Some(0x0403_0201);
    let p = Provide {
        name: "p".into(),
        func: 200,
        doc: "dp".into(),
        params: vec![Param::new("x", ApiType::Struct("S".into())), Param::new("t", ApiType::Str)],
        results: vec![Param::new("r", ApiType::Bool)],
    };
    let m = ModuleData {
        interface: Some(Interface {
            scratch: Some((128, 300)),
            structs: vec![s],
            provides: vec![p],
            events: vec![
                ProvidedEvent { name: "e".into(), payload: Some("S".into()), doc: "de".into() },
                ProvidedEvent { name: "f".into(), payload: None, doc: "".into() },
            ],
        }),
        ..Default::default()
    };
    let payload = [
        // scratch: flag, uleb u64 offset, uleb size
        &[1][..],
        &uleb(128),
        &uleb(300),
        // vec(struct): name, owner, cflag + 4 LE bytes, doc, size, align, vec(field)
        &[1],
        &st("S"),
        &st("eng"),
        &[1, 1, 2, 3, 4],
        &st("ds"),
        &[8, 4],
        // field: name, prim, count, offset, doc
        &[1],
        &st("a"),
        &[8, 1, 4],
        &st("fa"),
        // vec(provide): name, func, doc, params, results
        &[1],
        &st("p"),
        &uleb(200),
        &st("dp"),
        &[2],
        &st("x"),
        &[8],
        &st("S"),
        &st("t"),
        &[6],
        &[1],
        &st("r"),
        &[4],
        // vec(pevent): name, payload flag (+ struct name), doc
        &[2],
        &st("e"),
        &[1],
        &st("S"),
        &st("de"),
        &st("f"),
        &[0],
        &st(""),
    ]
    .concat();
    assert_eq!(encode(&m), section13(&payload));
    assert_eq!(decode(&section13(&payload)).unwrap(), m);
}

#[test]
fn ptr_is_not_encodable_and_rejected() {
    let mut i = valid();
    i.provides[1].params[0].ty = ApiType::Ptr;
    let m = base(Some(i));
    // The encoder cannot fail; it writes a byte the decoder rejects.
    assert_eq!(decode_err(&encode(&m)), DecodeError::BadApiType(0xFF));
    check(m, "provide `double`", "ptr");
}

#[test]
fn section_13_order() {
    let api = ModuleData { api: Some(ApiRef::default()), ..Default::default() };
    let iface = ModuleData { interface: Some(Interface::default()), ..Default::default() };
    let both = ModuleData { interface: Some(Interface::default()), ..api.clone() };
    let bytes = encode(&both);
    assert_eq!(decode(&bytes).unwrap(), both);
    // 13 before 12
    let mut b = encode(&iface);
    b.extend_from_slice(&encode(&api)[HEADER.len()..]);
    assert_eq!(decode_err(&b), DecodeError::SectionOutOfOrder(12));
    // 13 twice
    let mut b = encode(&iface);
    b.extend_from_slice(&encode(&iface)[HEADER.len()..]);
    assert_eq!(decode_err(&b), DecodeError::SectionOutOfOrder(13));
    // ids 0 and 15..=127 are still rejected
    for id in [0u8, 15, 127] {
        assert_eq!(decode_err(&[HEADER, &[id, 0]].concat()), DecodeError::UnknownSection(id));
    }
}

// ---------------------------------------------------------------------------------------------
// Decode errors
// ---------------------------------------------------------------------------------------------

/// A payload with one struct `S` (one field) and one provide `p` (one param, one result);
/// returns it with the positions of interesting bytes.
struct Layout {
    bytes: Vec<u8>,
    scratch_flag: usize,
    cflag: usize,
    prim: usize,
    param_type: usize,
    result_type: usize,
    struct_count: usize,
    event_flag: usize,
}

fn layout() -> Layout {
    let mut b = vec![0u8]; // no scratch
    let scratch_flag = 0;
    let struct_count = b.len();
    b.push(1);
    b.extend(st("S"));
    b.extend(st(""));
    let cflag = b.len();
    b.push(0);
    b.extend(st(""));
    b.extend([4, 4, 1]);
    b.extend(st("a"));
    let prim = b.len();
    b.extend([4, 1, 0]);
    b.extend(st(""));
    b.push(1);
    b.extend(st("p"));
    b.push(0);
    b.extend(st("d"));
    b.push(1);
    b.extend(st("x"));
    let param_type = b.len();
    b.push(0);
    b.push(1);
    b.extend(st("r"));
    let result_type = b.len();
    b.push(1);
    b.push(1); // one provided event
    b.extend(st("e"));
    let event_flag = b.len();
    b.push(1);
    b.extend(st("S"));
    b.extend(st("d"));
    Layout { bytes: b, scratch_flag, cflag, prim, param_type, result_type, struct_count, event_flag }
}

#[test]
fn layout_seed_decodes() {
    let l = layout();
    let m = decode(&section13(&l.bytes)).unwrap();
    let i = m.interface.unwrap();
    assert_eq!(i.structs[0].fields[0].ty, Prim::U32);
    assert_eq!(i.provides[0].params[0].ty, ApiType::I32);
    assert_eq!(i.provides[0].results[0].ty, ApiType::I64);
    assert_eq!(i.events, vec![ProvidedEvent { name: "e".into(), payload: Some("S".into()), doc: "d".into() }]);
}

#[test]
fn bad_api_type() {
    let l = layout();
    for b in [9u8, 10, 0x7F, 0xFF] {
        for at in [l.param_type, l.result_type] {
            let mut p = l.bytes.clone();
            p[at] = b;
            assert_eq!(decode_err(&section13(&p)), DecodeError::BadApiType(b));
        }
    }
}

#[test]
fn bad_prim() {
    let l = layout();
    for b in [12u8, 0x80, 0xFF] {
        let mut p = l.bytes.clone();
        p[l.prim] = b;
        assert_eq!(decode_err(&section13(&p)), DecodeError::BadPrim(b));
    }
}

#[test]
fn bad_flags() {
    let l = layout();
    for at in [l.scratch_flag, l.cflag, l.event_flag] {
        let mut p = l.bytes.clone();
        p[at] = 2;
        assert_eq!(decode_err(&section13(&p)), DecodeError::BadFlag(2));
    }
}

#[test]
fn truncation_and_size() {
    let l = layout();
    // Every strict prefix of the payload, framed with its own (shorter) size, is rejected.
    for n in 0..l.bytes.len() {
        let e = decode_err(&section13(&l.bytes[..n]));
        assert!(matches!(e, DecodeError::UnexpectedEnd | DecodeError::CountTooLarge), "prefix {n}: {e:?}");
    }
    // Trailing bytes inside the section.
    let mut p = l.bytes.clone();
    p.push(0);
    assert_eq!(decode_err(&section13(&p)), DecodeError::SectionSizeMismatch(13));
}

#[test]
fn counts_strings_and_lebs() {
    let l = layout();
    let mut p = l.bytes.clone();
    p[l.struct_count] = 0x7F;
    assert_eq!(decode_err(&section13(&p)), DecodeError::CountTooLarge);
    // A huge provide count after an empty struct list: no allocation, just an error.
    let p = [&[0u8, 0][..], &uleb(u32::MAX as u64)].concat();
    assert_eq!(decode_err(&section13(&p)), DecodeError::CountTooLarge);
    // Invalid UTF-8 in a struct name.
    let p = [0u8, 1, 1, 0xFF];
    assert_eq!(decode_err(&section13(&p)), DecodeError::BadUtf8);
    // Over-long scratch size (u32 in 6 bytes).
    let p = [1u8, 0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 0, 0];
    assert_eq!(decode_err(&section13(&p)), DecodeError::LebTooLong);
    // Scratch offset is a u64: 2^63 fits, 11 bytes do not.
    let p = [&[1u8][..], &uleb(1 << 63), &[5, 0, 0, 0]].concat();
    assert_eq!(decode(&section13(&p)).unwrap().interface.unwrap().scratch, Some((1 << 63, 5)));
    let p = [1u8, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 5, 0, 0];
    assert_eq!(decode_err(&section13(&p)), DecodeError::LebTooLong);
}

#[test]
fn mutated_interface_never_panics() {
    let seeds = [encode(&base(Some(valid()))), section13(&layout().bytes)];
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for it in 0..20_000 {
        let mut b = seeds[it % 2].clone();
        for _ in 0..1 + next() % 3 {
            let n = b.len();
            let i = (next() as usize) % n;
            match next() % 4 {
                0 => b[i] ^= 1 << (next() % 8),
                1 => b[i] = next() as u8,
                2 => b.truncate(i),
                _ => b.insert(i, next() as u8),
            }
            if b.is_empty() {
                b.push(0);
            }
        }
        let _ = Module::decode(&b);
    }
}

// ---------------------------------------------------------------------------------------------
// Verifier rules (SPEC.md 9.2)
// ---------------------------------------------------------------------------------------------

#[test]
fn rule1_counts() {
    let many = |n: usize, f: &dyn Fn(usize) -> Provide| (0..n).map(f).collect::<Vec<_>>();
    let mk = |i: usize| Provide { name: format!("p{i}"), ..double() };
    assert!(Module::new(with(|i| i.provides = many(1024, &mk))).is_ok());
    check(with(|i| i.provides = many(1025, &mk)), "interface", "1025 provides");
    check(
        with(|i| i.structs = (0..257).map(|k| StructDesc::new(format!("S{k}"), 0, 1)).collect()),
        "interface",
        "257 structs",
    );
    check(
        with(|i| i.structs[0].fields = (0..257).map(|k| FieldDesc::new(format!("f{k}"), k, Prim::U8, 1)).collect()),
        "struct `Item`",
        "257 fields",
    );
    check(
        with(|i| i.provides[1].params = (0..65).map(|k| Param::new(format!("a{k}"), ApiType::I32)).collect()),
        "provide `double`",
        "65 parameters",
    );
    check(
        with(|i| i.provides[1].results = (0..17).map(|k| Param::new(format!("r{k}"), ApiType::I32)).collect()),
        "provide `double`",
        "17 results",
    );
}

#[test]
fn rule2_names_and_doc() {
    check(with(|i| i.provides[1].name = "do-it".into()), "provide `do-it`", "not an identifier");
    check(with(|i| i.provides[1].name = "".into()), "provide ``", "not an identifier");
    check(with(|i| i.provides[1].name = "give".into()), "provide `give`", "duplicate provide");
    check(with(|i| i.provides[1].params[0].name = "1x".into()), "provide `double`", "`1x` is not an identifier");
    check(with(|i| i.provides[1].results[0].name = "x".into()), "provide `double`", "duplicate parameter or result name `x`");
    check(
        with(|i| i.provides[0].params[1].name = "text".into()),
        "provide `give`",
        "duplicate parameter or result name `text`",
    );
    check(with(|i| i.provides[1].doc = " \n\t".into()), "provide `double`", "doc is blank");
    check(with(|i| i.provides[1].doc = "".into()), "provide `double`", "doc is blank");
}

#[test]
fn rule3_defined_function() {
    check(with(|i| i.provides[1].func = IMPORT), "provide `double`", "is an import");
    check(with(|i| i.provides[1].func = 3), "provide `double`", "out of range");
    check(with(|i| i.provides[1].func = u32::MAX), "provide `double`", "out of range");
}

#[test]
fn rule4_signature() {
    check(with(|i| i.provides[1].func = TAKE), "provide `double`", "signature lowers to (i32) -> i32");
    check(with(|i| i.provides[1].params[0].ty = ApiType::I64), "provide `double`", "signature");
    check(with(|i| i.provides[1].results.clear()), "provide `double`", "signature");
    // bool and entity lower like i32 and i64
    assert!(Module::new(with(|i| {
        i.provides[1].params[0].ty = ApiType::Bool;
        i.provides[0].results[0].ty = ApiType::Entity;
    }))
    .is_ok());
}

#[test]
fn rule5_scalar_results() {
    for t in [ApiType::Str, ApiType::Bytes] {
        let m = with(|i| {
            i.provides[0].params = vec![Param::new("a", ApiType::I64)];
            i.provides[0].results = vec![Param::new("n", t.clone())];
        });
        // (i64) -> (i64, i64) does not match either function, but rule 5 is reported first.
        check(m, "provide `give`", "results must be scalar");
    }
    check(with(|i| i.provides[0].results[0].ty = ApiType::Struct("Item".into())), "provide `give`", "results must be scalar");
    check(with(|i| i.provides[0].results[0].ty = ApiType::Ptr), "provide `give`", "ptr");
    check(with(|i| i.provides[0].params[1].ty = ApiType::Ptr), "provide `give`", "`item` is a ptr");
}

#[test]
fn rule6_listed_structs() {
    check(
        with(|i| i.provides[0].params[1].ty = ApiType::Struct("Missing".into())),
        "provide `give`",
        "struct `Missing` is not listed",
    );
    // A copied struct may be used as well as an own one.
    assert!(Module::new(with(|i| i.provides[0].params[1].ty = ApiType::Struct("Transform".into()))).is_ok());
}

#[test]
fn rule7_structs() {
    check(with(|i| i.structs[0].align = 3), "struct `Item`", "not a power of two");
    check(with(|i| i.structs[0].name = "It em".into()), "struct `It em`", "not an identifier");
    check(with(|i| i.structs[1].name = "Item".into()), "struct `Item`", "duplicate struct name");
    check(with(|i| i.structs[0].component = Some(1)), "struct `Item`", "cannot have a component id");
    check(with(|i| i.structs[1].owner = "engine..x".into()), "struct `Transform`", "not a namespace");
    check(with(|i| i.structs[1].owner = " ".into()), "struct `Transform`", "not a namespace");
    check(
        with(|i| {
            let mut t = transform();
            t.name = "Other".into();
            i.structs.push(t);
        }),
        "struct `Other`",
        "duplicate component id 7",
    );
    assert!(Module::new(with(|i| i.structs[1].owner = "my_game.core".into())).is_ok());
}

#[test]
fn rule8_scratch() {
    check(with(|i| i.scratch = None), "scratch", "provide `give` has a str, bytes or struct parameter");
    check(with(|i| i.scratch = Some((64, 0))), "scratch", "at least 1 byte");
    // `bytes` alone and a struct alone also need it; both signatures lower to (i64, i64, i64).
    let only = [
        vec![Param::new("a", ApiType::Bytes), Param::new("b", ApiType::I64)],
        vec![Param::new("a", ApiType::I64), Param::new("b", ApiType::I64), Param::new("c", ApiType::Struct("Item".into()))],
    ];
    for params in only {
        let m = with(|i| {
            i.scratch = None;
            i.provides[0].params = params.clone();
        });
        check(m, "scratch", "required");
    }
    // Without non-scalar parameters scratch may be absent or empty.
    let scalars_only = |s: Option<(u64, u32)>| {
        with(|i| {
            i.provides.remove(0);
            i.scratch = s;
        })
    };
    assert!(Module::new(scalars_only(None)).is_ok());
    assert!(Module::new(scalars_only(Some((0, 0)))).is_ok());
    check(scalars_only(Some((4, 0))), "scratch", "not a multiple of 8");
    check(with(|i| i.scratch = Some((68, 8))), "scratch", "offset 68 is not a multiple of 8");
    check(with(|i| i.scratch = Some((0, 1_048_577))), "scratch", "larger than 1048576");
    check(with(|i| i.scratch = Some((65536 - 8, 16))), "scratch", "outside the initial memory");
    check(with(|i| i.scratch = Some((u64::MAX - 7, 16))), "scratch", "outside the initial memory");
    check(with(|i| i.scratch = Some((8, 16))), "scratch", "overlaps data segment 0");
    check(with(|i| i.scratch = Some((24, 8))), "scratch", "overlaps data segment 0");
    // Adjacent to the data segment [16, 32) on either side, and exactly the end of memory.
    assert!(Module::new(with(|i| i.scratch = Some((0, 16)))).is_ok());
    assert!(Module::new(with(|i| i.scratch = Some((32, 65536 - 32)))).is_ok());
    // An empty data segment overlaps nothing.
    let mut m = with(|i| i.scratch = Some((64, 64)));
    m.data.push(DataSegment { offset: 80, bytes: vec![] });
    assert!(Module::new(m).is_ok());
    // A size-0 scratch inside a data segment overlaps nothing either.
    assert!(Module::new(scalars_only(Some((24, 0)))).is_ok());
    // Size 1 MiB exactly is allowed when memory is large enough.
    let mut m = with(|i| i.scratch = Some((65536, 1_048_576)));
    m.memory.min_pages = 17;
    assert!(Module::new(m).is_ok());
}

#[test]
fn builder_finish_verifies_interface() {
    let mut b = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(1);
    fb.ret(0);
    let f = b.add_func(FuncType::new([], [ValType::I32]), fb);
    b.interface(Interface {
        provides: vec![Provide { name: "n".into(), func: f, doc: "".into(), params: vec![], results: vec![] }],
        ..Default::default()
    });
    assert!(matches!(
        b.finish(),
        Err(LoadError::Verify { error: VerifyError::Interface { ref what, .. }, .. }) if what == "provide `n`"
    ));
    let e = VerifyError::Interface { what: "provide `n`".into(), reason: "doc is blank".into() };
    assert_eq!(e.to_string(), "interface: provide `n`: doc is blank");
}

// ---------------------------------------------------------------------------------------------
// host_fns and index
// ---------------------------------------------------------------------------------------------

#[test]
fn lookups() {
    let i = valid();
    assert_eq!(i.provide("give"), Some(&give()));
    assert_eq!(i.provide("nope"), None);
    assert_eq!(i.struct_desc("Transform"), Some(&transform()));
    assert!(i.needs_scratch());
    assert!(i.provides[0].needs_scratch() && !i.provides[1].needs_scratch());
    assert_eq!(i.provides[0].func_type(), FuncType::new([ValType::I64; 3], [ValType::I64]));
    assert_eq!(i.own_structs().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Item"]);
    assert_eq!(i.copied_structs().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Transform"]);
    assert!(!Interface::default().needs_scratch());
}

#[test]
fn host_fns_describe_provides() {
    let fns = valid().host_fns("inventory");
    assert_eq!(fns.len(), 2);
    let g = &fns[0];
    assert_eq!((g.namespace.as_str(), g.name.as_str(), g.doc.as_str(), g.owner.as_str()), ("inventory", "give", "Give an item.", ""));
    assert_eq!(g.params, give().params);
    assert_eq!(g.results, give().results);
    assert_eq!(g.func_type(), give().func_type());
    assert_eq!(fns[1].qualified(), "inventory.double");
    assert_eq!(fns[1].signature_text(), "(x: i32) -> (y: i32)");
}

#[test]
fn index_is_valid_and_round_trips() {
    let ix = valid().index("inventory", 3, "Inventory mod.");
    ix.validate().expect("valid index");
    assert_eq!(ApiIndex::from_json(&ix.to_json()), Ok(ix.clone()));
    assert_eq!(ix.game, "inventory");
    assert_eq!(
        ix.namespaces,
        vec![NamespaceInfo { name: "inventory".into(), version: 3, owner: "inventory".into(), doc: "Inventory mod.".into() }]
    );
    // sorted by name
    assert_eq!(ix.qualified_names(), ["inventory.double", "inventory.give"]);
    assert!(ix.functions.iter().all(|f| f.owner == "inventory"));
    assert_eq!(ix.struct_type("Item").unwrap().owner, "inventory");
    let t = ix.struct_type("Transform").unwrap();
    assert_eq!((t.owner.as_str(), t.component), ("engine", Some(7)));
    assert_eq!(ix.function("inventory", "give").unwrap().func_type(), give().func_type());
    // The empty interface gives just the namespace.
    let e = Interface::default().index("m", 1, "M.");
    e.validate().unwrap();
    assert_eq!(ApiIndex::from_json(&e.to_json()), Ok(e.clone()));
    assert!(e.functions.is_empty() && e.types.is_empty());
}

#[test]
fn index_matches_what_the_registry_builds() {
    let iface = valid();
    let id = "inventory";
    let mut reg = HostRegistry::<()>::new();
    reg.set_game(id);
    reg.define_namespace(id, 3, id, "Inventory mod.").unwrap();
    for s in &iface.structs {
        let mut s = s.clone();
        if s.owner.is_empty() {
            s.owner = id.into();
        }
        reg.register_type(s).unwrap();
    }
    for f in iface.host_fns(id) {
        reg.register(f, |_c, _a, _r| Ok(HostOk::Continue)).unwrap();
    }
    for e in iface.event_descs(id) {
        reg.register_event(e).unwrap();
    }
    assert_eq!(reg.index(), iface.index(id, 3, "Inventory mod."));
}

// ---------------------------------------------------------------------------------------------
// Provided events (SPEC.md 9.2 rule 9)
// ---------------------------------------------------------------------------------------------

#[test]
fn rule9_provided_events() {
    assert!(Module::new(base(Some(valid()))).is_ok());
    check(with(|i| i.events[0].name = "drop-ped".into()), "event `drop-ped`", "not an identifier");
    check(with(|i| i.events[1].name = "dropped".into()), "event `dropped`", "duplicate event name");
    check(with(|i| i.events[1].name = "give".into()), "event `give`", "a provide has the same name");
    check(with(|i| i.events[1].doc = "  ".into()), "event `emptied`", "doc is blank");
    check(
        with(|i| i.events[0].payload = Some("Missing".into())),
        "event `dropped`",
        "payload struct `Missing` is not listed",
    );
    // A copied struct may be a payload too.
    assert!(Module::new(with(|i| i.events[0].payload = Some("Transform".into()))).is_ok());
    let mk = |k: usize| ProvidedEvent { name: format!("e{k}"), ..emptied() };
    assert!(Module::new(with(|i| i.events = (0..1024).map(mk).collect())).is_ok());
    check(with(|i| i.events = (0..1025).map(mk).collect()), "interface", "1025 events");
}

#[test]
fn provided_events_in_the_index() {
    let i = valid();
    assert_eq!(i.event("dropped"), Some(&dropped()));
    assert_eq!(i.event("nope"), None);
    let d = i.event_descs("inventory");
    assert_eq!(d.len(), 2);
    assert_eq!(
        d[0],
        EventDesc {
            namespace: "inventory".into(),
            name: "dropped".into(),
            payload: Some("Item".into()),
            doc: "An item was dropped.".into(),
            owner: String::new(),
        }
    );
    assert_eq!(d[1].qualified(), "inventory.emptied");
    let ix = i.index("inventory", 3, "Inventory mod.");
    ix.validate().unwrap();
    assert_eq!(ix.events.len(), 2);
    assert!(ix.events.iter().all(|e| e.owner == "inventory" && e.namespace == "inventory"));
    assert_eq!(ix.event("inventory", "dropped").unwrap().payload.as_deref(), Some("Item"));
}
