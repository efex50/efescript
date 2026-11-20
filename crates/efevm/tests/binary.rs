//! Binary format tests (SPEC.md sections 9, 9.1, 10): round trips, format details, never-panics.

mod common;
use common::try_host;
use efevm::binary::{decode, encode};
use efevm::*;
use std::collections::HashSet;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// Header of an empty module: magic, version 2, checksum (CRC-32 of nothing is 0).
const HEADER: &[u8] = b"EFVM\x02\x00\0\0\0\0";

/// A copy of `b` with its header checksum recomputed, for hand-built or spliced bytes.
fn sealed(b: &[u8]) -> Vec<u8> {
    let mut b = b.to_vec();
    efevm::binary::seal(&mut b);
    b
}

// ---------------------------------------------------------------------------------------------
// The big module: one instruction per op (plus extras), multi-byte LEBs everywhere.
// ---------------------------------------------------------------------------------------------

const NIMPORTS: u32 = 2;
const NFILLERS: u32 = 134;
const CALLEE: u32 = NIMPORTS + NFILLERS; // 136
const BIG: u32 = CALLEE + 1; // 137
const NGLOBALS: u32 = 140;
const NTYPES: usize = 140;
const REGS: [u32; 9] = [300, 65534, 129, 16384, 255, 128, 7, 40000, 1];
const IMM32: [i32; 8] = [i32::MIN, i32::MAX, -1, 63, 64, -64, -65, 0x1234_5678];
const IMM64: [i64; 8] = [i64::MIN, i64::MAX, -1, 63, -64, -65, 0x1234_5678_9ABC_DEF0, 1 << 40];
const F32_BITS: [u32; 8] =
    [0x7FC0_1234, 0xFFC0_0001, 0x8000_0000, 0x7F80_0000, 0xFF80_0000, 0x0000_0001, 0x3F80_0000, 0x7F7F_FFFF];
const F64_BITS: [u64; 8] = [
    0x7FF8_0000_DEAD_BEEF,
    0xFFF8_0000_0000_0001,
    0x8000_0000_0000_0000,
    0x7FF0_0000_0000_0000,
    0xFFF0_0000_0000_0000,
    0x0000_0000_0000_0001,
    0x3FF0_0000_0000_0000,
    0x7FEF_FFFF_FFFF_FFFF,
];

fn rr(k: usize, j: usize) -> u32 {
    REGS[(k + j) % REGS.len()]
}

fn f32c(d: u32, bits: u32) -> Inst {
    Inst::new(Op::ConstF32, d, bits, 0)
}
fn f64c(d: u32, bits: u64) -> Inst {
    Inst::new(Op::ConstF64, d, bits as u32, (bits >> 32) as u32)
}

/// Builds the big module; `n_hint` is the final code length (used for labels), 0 on the sizing pass.
fn build_big(n_hint: u32) -> ModuleData {
    let last = n_hint.saturating_sub(1);
    let mut types: Vec<FuncType> = (0..NTYPES)
        .map(|i| match i % 4 {
            0 => FuncType::new([ValType::I32], [ValType::I32]),
            1 => FuncType::new([], []),
            2 => FuncType::new(
                [ValType::I64, ValType::I64, ValType::F32, ValType::F64],
                [ValType::I32, ValType::I64],
            ),
            _ => FuncType::new([ValType::F64], []),
        })
        .collect();
    types[0] = FuncType::new([ValType::I32], [ValType::I32]);
    types[1] = FuncType::new([], []);
    types[2] = FuncType::new([ValType::I64, ValType::F32], [ValType::F64]);
    types[130] = FuncType::new([ValType::I32], [ValType::I32]);

    let imports = vec![
        Import { namespace: "env".into(), name: "imp".into(), type_idx: 0 },
        Import { namespace: "env.other".into(), name: "other".into(), type_idx: 2 },
    ];

    let mut functions = Vec::new();
    for _ in 0..NFILLERS {
        functions.push(Function {
            type_idx: 1,
            nregs: 1,
            code: vec![Inst::new(Op::Ret, 0, 0, 0)],
            br_tables: vec![],
        });
    }
    functions.push(Function {
        type_idx: 0,
        nregs: 1,
        code: vec![Inst::new(Op::Ret, 0, 0, 0)],
        br_tables: vec![],
    });

    // The big function.
    let mut code: Vec<Inst> = Vec::new();
    let mut br_tables: Vec<BrTable> = Vec::new();
    for (k, &op) in Op::ALL.iter().enumerate() {
        let (a, b, c, d) = (rr(k, 0), rr(k, 1), rr(k, 2), rr(k, 3));
        let inst = match op.shape() {
            Shape::None => Inst::new(op, 0, 0, 0),
            Shape::D => Inst::new(op, a, 0, 0),
            Shape::DA => Inst::new(op, a, b, 0),
            Shape::DAB | Shape::ABC => Inst::new(op, a, b, c),
            Shape::ConstI32 => Inst::const_i32(a, IMM32[k % 8]),
            Shape::ConstI64 => Inst::const_i64(a, IMM64[k % 8]),
            Shape::ConstF32 => f32c(a, F32_BITS[k % 8]),
            Shape::ConstF64 => f64c(a, F64_BITS[k % 8]),
            Shape::Select => Inst::select(a, b, c, d),
            Shape::Load | Shape::Store => Inst::new(op, a, b, 0x1234_5678),
            Shape::GlobalGet => Inst::new(op, a, NGLOBALS - 1, 0),
            Shape::GlobalSet => Inst::new(op, NGLOBALS - 2, a, 0),
            Shape::Jmp => Inst::new(op, last, 0, 0),
            Shape::Br => Inst::new(op, a, last, 0),
            Shape::BrTable => {
                br_tables.push(BrTable {
                    targets: vec![0, 1, 127, 128, last, last / 2, 5],
                    default: last,
                });
                Inst::new(op, a, (br_tables.len() - 1) as u32, 0)
            }
            Shape::Call => Inst::new(op, CALLEE, 65534, 0),
            Shape::CallIndirect => Inst::new(op, 130, b, 65534),
            Shape::Ret => Inst::new(op, 65534, 0, 0),
        };
        code.push(inst);
    }
    // Extras: extreme immediates and more br_tables.
    for v in IMM32 {
        code.push(Inst::const_i32(300, v));
    }
    for v in IMM64 {
        code.push(Inst::const_i64(65534, v));
    }
    for v in F32_BITS {
        code.push(f32c(129, v));
    }
    for v in F64_BITS {
        code.push(f64c(16384, v));
    }
    br_tables.push(BrTable { targets: vec![], default: 0 });
    code.push(Inst::new(Op::BrTable, 300, (br_tables.len() - 1) as u32, 0));
    br_tables.push(BrTable { targets: (0..200).map(|i| i % (last + 1)).collect(), default: 3 });
    code.push(Inst::new(Op::BrTable, 65534, (br_tables.len() - 1) as u32, 0));
    code.push(Inst::new(Op::Ret, 0, 0, 0));
    if n_hint != 0 {
        assert_eq!(code.len() as u32, n_hint);
    }
    functions.push(Function { type_idx: 0, nregs: 65535, code, br_tables });

    let table: Vec<Option<u32>> = (0..300u32)
        .map(|i| match i % 4 {
            0 => None,
            1 => Some(0),
            2 => Some(CALLEE),
            _ => Some(BIG),
        })
        .collect();

    let globals: Vec<Global> = (0..NGLOBALS)
        .map(|i| {
            let ty = [ValType::I32, ValType::I64, ValType::F32, ValType::F64][(i % 4) as usize];
            let init = match (ty, i) {
                (ValType::I32, 0) => i32::MIN as u32 as u64,
                (ValType::I32, _) => 0x7FFF_FFFF - i as u64,
                (ValType::I64, 1) => i64::MIN as u64,
                (ValType::I64, _) => (i64::MAX as u64) - i as u64,
                (ValType::F32, 2) => 0x7FC0_1234,
                (ValType::F32, _) => f32::to_bits(-0.0) as u64,
                (ValType::F64, 3) => 0xFFF8_0000_0000_0007,
                (ValType::F64, _) => (-0.0f64).to_bits(),
            };
            Global { ty, mutable: i != 4, init }
        })
        .collect();

    ModuleData {
        types,
        imports,
        functions,
        table,
        memory: MemoryType { min_pages: 1, max_pages: Some(3) },
        globals,
        exports: vec![
            Export { name: "big".into(), kind: ExportKind::Func(BIG) },
            Export { name: "callee".into(), kind: ExportKind::Func(CALLEE) },
            Export { name: "imp".into(), kind: ExportKind::Func(0) },
            Export { name: "g0".into(), kind: ExportKind::Global(0) },
            Export { name: "g139".into(), kind: ExportKind::Global(NGLOBALS - 1) },
            Export { name: "".into(), kind: ExportKind::Global(5) },
        ],
        data: vec![
            DataSegment { offset: 65530, bytes: vec![0, 1, 2, 3, 4, 0xFF] },
            DataSegment { offset: 100, bytes: vec![] },
            DataSegment { offset: 0, bytes: (0..=255u8).collect() },
        ],
        names: Names {
            funcs: vec![(0, "imp".into()), (CALLEE, "callee".into()), (BIG, "big \u{1F600}".into())],
            regs: vec![(BIG, 65534, "last".into()), (BIG, 300, "r300".into()), (CALLEE, 0, "x".into())],
        },
        lines: Lines {
            files: vec!["a.ef".into(), "dir/b.ef".into()],
            funcs: vec![
                FuncLines {
                    func: CALLEE,
                    entries: vec![LineEntry { inst: 0, file: 0, line: 1, column: 1 }],
                },
                FuncLines {
                    func: BIG,
                    entries: vec![
                        LineEntry { inst: 0, file: 1, line: 1_000_000, column: 200 },
                        LineEntry { inst: 5, file: 1, line: 7, column: 9 },
                        LineEntry { inst: last, file: 0, line: u32::MAX, column: u32::MAX },
                    ],
                },
            ],
        },
        api: Some(ApiRef {
            hash: 0x9f0c_2b5e_17a4_d3c8,
            namespaces: vec![("env".into(), 1), ("engine.world".into(), u32::MAX)],
        }),
        interface: None,
        events: vec![
            EventRef { namespace: "game".into(), name: "score_changed".into(), payload: Some(("Score".into(), 8)) },
            EventRef { namespace: "engine.input".into(), name: "jump".into(), payload: None },
        ],
    }
}

fn big_module() -> ModuleData {
    let n = build_big(0).functions.last().unwrap().code.len() as u32;
    build_big(n)
}

fn big_bytes() -> Vec<u8> {
    encode(&big_module())
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn read_uleb(b: &[u8], pos: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let x = b[*pos];
        *pos += 1;
        v |= ((x & 0x7F) as u64) << shift;
        if x & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

/// Splits a well-formed encoding into (id, start, end) byte ranges of its sections.
fn sections(b: &[u8]) -> Vec<(u8, usize, usize)> {
    let mut out = Vec::new();
    let mut pos = HEADER.len();
    while pos < b.len() {
        let start = pos;
        let id = b[pos];
        pos += 1;
        let size = read_uleb(b, &mut pos) as usize;
        pos += size;
        out.push((id, start, pos));
    }
    assert_eq!(pos, b.len());
    out
}

/// Decodes `bytes` after sealing them, so the error comes from the part under test.
fn decode_err(bytes: &[u8]) -> DecodeError {
    match Module::decode(&sealed(bytes)) {
        Err(LoadError::Decode { error, .. }) => error,
        other => panic!("expected decode error, got {other:?}"),
    }
}

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

// ---------------------------------------------------------------------------------------------
// 1. Every instruction round-trips
// ---------------------------------------------------------------------------------------------

#[test]
fn every_instruction_round_trips() {
    let data = big_module();
    let code = &data.functions.last().unwrap().code;
    assert!(code.len() > 128, "labels must need 2-byte LEBs");

    // Coverage: every op appears in the big function.
    let covered: HashSet<Op> = code.iter().map(|i| i.op).collect();
    let all: HashSet<Op> = Op::ALL.iter().copied().collect();
    assert_eq!(covered, all);
    // Each op's non-trivial instruction is the k-th one.
    for (k, op) in Op::ALL.iter().enumerate() {
        assert_eq!(code[k].op, *op);
    }

    // It verifies.
    let module = match Module::new(data.clone()) {
        Ok(m) => m,
        Err(e) => panic!("big module does not verify: {e}"),
    };
    let bytes = module.encode();
    assert_eq!(bytes, encode(&data));
    assert!(bytes.starts_with(&HEADER[..6]));

    let decoded = Module::decode(&bytes).expect("decode+verify");
    assert_eq!(decoded.data(), &data);
    let raw = decode(&bytes).expect("unverified decode");
    assert_eq!(raw, data);
    assert_eq!(encode(&raw), bytes);
    assert_eq!(decoded.encode(), bytes);
}

#[test]
fn every_op_alone_round_trips() {
    // Each op in its own tiny function body (operands from the big module), checked individually.
    let data = big_module();
    let code = &data.functions.last().unwrap().code;
    for (k, op) in Op::ALL.iter().enumerate() {
        let mut m = data.clone();
        let f = m.functions.last_mut().unwrap();
        f.code = vec![code[k], Inst::new(Op::Ret, 0, 0, 0)];
        // keep only br_tables needed: the op's table (if any) becomes table 0
        let mut ins = code[k];
        if *op == Op::BrTable {
            let t = data.functions.last().unwrap().br_tables[ins.b as usize].clone();
            f.br_tables = vec![t];
            ins.b = 0;
            f.code[0] = ins;
        } else {
            f.br_tables = vec![];
        }
        let (want_code, want_tables) = (f.code.clone(), f.br_tables.clone());
        let bytes = encode(&m);
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", op.name()));
        let got = back.functions.last().unwrap();
        assert_eq!(got.code, want_code, "{}", op.name());
        assert_eq!(got.br_tables, want_tables, "{}", op.name());
    }
}

// ---------------------------------------------------------------------------------------------
// 2. Every section
// ---------------------------------------------------------------------------------------------

fn all_sections_module(memory: MemoryType) -> ModuleData {
    ModuleData {
        types: vec![
            FuncType::new([], []),
            FuncType::new([ValType::I32, ValType::I64, ValType::F32, ValType::F64], [ValType::F64]),
        ],
        imports: vec![
            Import { namespace: "env".into(), name: "a".into(), type_idx: 1 },
            Import { namespace: "".into(), name: "".into(), type_idx: 9999 },
        ],
        functions: vec![
            Function {
                type_idx: 0,
                nregs: 0,
                code: vec![],
                br_tables: vec![],
            },
            Function {
                type_idx: 1,
                nregs: 65535,
                code: vec![
                    Inst::new(Op::BrTable, 65535, 0, 0), // unverifiable (reg >= nregs-ish), still round trips
                    Inst::const_i64(1, i64::MIN),
                    Inst::new(Op::Ret, 0, 0, 0),
                ],
                br_tables: vec![BrTable { targets: vec![9, 8, 7000], default: 1 }],
            },
        ],
        table: vec![None, Some(0), None, Some(u32::MAX - 1), Some(2)],
        memory,
        globals: vec![
            Global { ty: ValType::I32, mutable: false, init: (-5i32) as u32 as u64 },
            Global { ty: ValType::I32, mutable: true, init: 0 },
            Global { ty: ValType::I64, mutable: true, init: i64::MIN as u64 },
            Global { ty: ValType::I64, mutable: false, init: u64::MAX },
            Global { ty: ValType::F32, mutable: false, init: f32::to_bits(-0.0) as u64 },
            Global { ty: ValType::F32, mutable: true, init: 0x7FC0_0001 },
            Global { ty: ValType::F64, mutable: true, init: 0xFFF8_0000_0000_0001 },
            Global { ty: ValType::F64, mutable: false, init: 1.5f64.to_bits() },
        ],
        exports: vec![
            Export { name: "f".into(), kind: ExportKind::Func(1) },
            Export { name: "g".into(), kind: ExportKind::Global(2) },
            Export { name: "same-index".into(), kind: ExportKind::Func(u32::MAX) },
        ],
        data: vec![
            DataSegment { offset: 0, bytes: vec![1, 2, 3] },
            DataSegment { offset: 12, bytes: vec![] },
            DataSegment { offset: u64::MAX, bytes: vec![0xAA] },
            DataSegment { offset: 1 << 40, bytes: vec![0; 300] },
        ],
        names: Names {
            funcs: vec![(0, "imp".into()), (2, "main".into()), (u32::MAX, "far".into())],
            regs: vec![(2, 0, "p".into()), (2, 65534, "q".into()), (99, 99, "".into())],
        },
        lines: Lines {
            files: vec!["a".into(), "b\u{e9}.ef".into(), "".into()],
            funcs: vec![
                FuncLines { func: 2, entries: vec![] },
                FuncLines {
                    func: 3,
                    entries: vec![
                        LineEntry { inst: 0, file: 0, line: 1, column: 2 },
                        LineEntry { inst: 2, file: 2, line: 3, column: 4 },
                    ],
                },
            ],
        },
        api: Some(ApiRef { hash: u64::MAX, namespaces: vec![("".into(), 0), ("x\u{e9}".into(), 300)] }),
        interface: Some(odd_interface()),
        events: vec![
            EventRef { namespace: "".into(), name: "".into(), payload: Some(("".into(), u32::MAX)) },
            EventRef { namespace: "x\u{e9}".into(), name: "1".into(), payload: None },
            EventRef { namespace: "x\u{e9}".into(), name: "1".into(), payload: None },
        ],
    }
}

/// Decodable but not verifiable: every `apitype` and `prim` byte, extreme LEB values.
fn odd_interface() -> Interface {
    let all_types = [
        ApiType::I32,
        ApiType::I64,
        ApiType::F32,
        ApiType::F64,
        ApiType::Bool,
        ApiType::Entity,
        ApiType::Str,
        ApiType::Bytes,
        ApiType::Struct("".into()),
        ApiType::Struct("S\u{e9}".into()),
    ];
    let mut s = StructDesc::new("S\u{e9}", u32::MAX, 1 << 31).doc("d\u{0}");
    s.owner = "engine.world".into();
    s.component = Some(u32::MAX);
    for (i, p) in Prim::ALL.into_iter().enumerate() {
        s.fields.push(FieldDesc::new(format!("f{i}"), u32::MAX - i as u32, p, 1 << 20).with_doc("x"));
    }
    Interface {
        scratch: Some((u64::MAX, u32::MAX)),
        structs: vec![s, StructDesc::new("", 0, 0)],
        provides: vec![
            Provide {
                name: "p".into(),
                func: u32::MAX,
                doc: "".into(),
                params: all_types.iter().enumerate().map(|(i, t)| Param::new(format!("a{i}"), t.clone())).collect(),
                results: all_types.iter().map(|t| Param::new("", t.clone())).collect(),
            },
            Provide::default(),
        ],
        events: vec![
            ProvidedEvent { name: "".into(), payload: Some("".into()), doc: "".into() },
            ProvidedEvent { name: "e\u{e9}".into(), payload: None, doc: "d\u{0}".into() },
        ],
    }
}

#[test]
fn every_section_round_trips() {
    for memory in [
        MemoryType { min_pages: 0, max_pages: None },
        MemoryType { min_pages: 0, max_pages: Some(0) },
        MemoryType { min_pages: 1, max_pages: None },
        MemoryType { min_pages: 1, max_pages: Some(1) },
        MemoryType { min_pages: 5, max_pages: Some(u32::MAX) },
        MemoryType { min_pages: u32::MAX, max_pages: None },
    ] {
        let m = all_sections_module(memory);
        let bytes = encode(&m);
        let ids: Vec<u8> = sections(&bytes).iter().map(|s| s.0).collect();
        let expect_mem = memory.min_pages != 0 || memory.max_pages.is_some();
        let mut expect: Vec<u8> = vec![1, 2, 3, 4];
        if expect_mem {
            expect.push(5);
        }
        expect.extend([6, 7, 8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(ids, expect, "{memory:?}");
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{memory:?}: {e}"));
        assert_eq!(back, m, "{memory:?}");
        assert_eq!(encode(&back), bytes);
        // The verifier must reject this one (it is deliberately out of range) but never panic.
        assert!(Module::decode(&bytes).is_err());
    }
}

#[test]
fn memory_none_vs_zero_distinguished() {
    // "no max" and "max 0" with min 0 must not collapse into each other.
    let a = ModuleData { memory: MemoryType { min_pages: 0, max_pages: None }, ..Default::default() };
    let b = ModuleData { memory: MemoryType { min_pages: 0, max_pages: Some(0) }, ..Default::default() };
    assert_eq!(encode(&a), HEADER);
    assert_ne!(encode(&b), HEADER);
    assert_eq!(decode(&encode(&b)).unwrap(), b);
    assert_eq!(decode(&encode(&a)).unwrap(), a);
}

// ---------------------------------------------------------------------------------------------
// 3. Format details
// ---------------------------------------------------------------------------------------------

#[test]
fn header_and_empty_module() {
    assert_eq!(HEADER, &[0x45, 0x46, 0x56, 0x4D, 0x02, 0x00, 0, 0, 0, 0]);
    assert_eq!(encode(&ModuleData::default()), HEADER);
    let m = Module::decode(HEADER).unwrap();
    assert_eq!(m.data(), &ModuleData::default());
    assert_eq!(m.encode(), HEADER);
    assert_eq!(big_bytes()[..6], HEADER[..6]);

    // Bad header variants.
    assert!(matches!(decode_err(b""), DecodeError::UnexpectedEnd | DecodeError::BadMagic));
    assert!(matches!(decode_err(b"EFV"), DecodeError::UnexpectedEnd | DecodeError::BadMagic));
    assert_eq!(decode_err(b"EFVN\x02\x00"), DecodeError::BadMagic);
    assert_eq!(decode_err(b"\x00asm\x02\x00"), DecodeError::BadMagic);
    assert_eq!(decode_err(b"EFVM\x01\x00\0\0\0\0"), DecodeError::BadVersion(1));
    assert_eq!(decode_err(b"EFVM\x03\x00"), DecodeError::BadVersion(3));
    assert_eq!(decode_err(b"EFVM\x00\x02"), DecodeError::BadVersion(512));
    assert_eq!(decode_err(b"EFVM\x00\x00"), DecodeError::BadVersion(0));
    assert!(matches!(decode_err(b"EFVM\x02"), DecodeError::UnexpectedEnd));
    assert!(matches!(decode_err(b"EFVM\x02\x00\0\0\0"), DecodeError::UnexpectedEnd));
    // An unsealed module (stale checksum) is rejected before its sections are read.
    let mut b = big_bytes();
    let last = b.len() - 1;
    b[last] ^= 0x40;
    assert!(matches!(
        Module::decode(&b),
        Err(LoadError::Decode { offset: 6, error: DecodeError::BadChecksum { .. } })
    ));
}

#[test]
fn custom_sections_are_skipped() {
    let data = big_module();
    let bytes = encode(&data);
    let secs = sections(&bytes);
    let mut boundaries: Vec<usize> = secs.iter().map(|s| s.1).collect();
    boundaries.push(bytes.len());
    let customs: [&[u8]; 5] = [
        &[200, 3, 1, 2, 3],
        &[255, 0],
        &[128, 0],
        &[200, 4, 0xFF, 0xFF, 0xFF, 0xFF],
        &[200, 5, 1, 0, 0, 0, 0, 200, 0], // payload then a second custom section
    ];
    for &at in &boundaries {
        for c in customs {
            let mut b = bytes[..at].to_vec();
            b.extend_from_slice(c);
            b.extend_from_slice(&bytes[at..]);
            let m = Module::decode(&sealed(&b)).unwrap_or_else(|e| panic!("custom {c:?} at {at}: {e}"));
            assert_eq!(m.data(), &data);
        }
    }
    // Custom section only.
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[200, 2, 9, 9]);
    assert_eq!(Module::decode(&sealed(&b)).unwrap().data(), &ModuleData::default());
    // Custom section with a size that overruns the input.
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[200, 9, 1, 2]);
    assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })));
}

#[test]
fn sections_out_of_order_rejected() {
    let bytes = big_bytes();
    let secs = sections(&bytes);
    for i in 0..secs.len() - 1 {
        let (a, b) = (secs[i], secs[i + 1]);
        let mut out = bytes[..a.1].to_vec();
        out.extend_from_slice(&bytes[b.1..b.2]);
        out.extend_from_slice(&bytes[a.1..a.2]);
        out.extend_from_slice(&bytes[b.2..]);
        assert_eq!(out.len(), bytes.len());
        match decode_err(&out) {
            DecodeError::SectionOutOfOrder(_) => {}
            e => panic!("swap {} and {}: {e:?}", a.0, b.0),
        }
    }
}

#[test]
fn duplicate_section_rejected() {
    let bytes = big_bytes();
    let secs = sections(&bytes);
    for &(id, s, e) in &secs {
        let mut out = bytes[..e].to_vec();
        out.extend_from_slice(&bytes[s..e]);
        out.extend_from_slice(&bytes[e..]);
        match decode_err(&out) {
            DecodeError::SectionOutOfOrder(_) => {}
            err => panic!("duplicate of section {id}: {err:?}"),
        }
    }
}

#[test]
fn unknown_section_ids_rejected() {
    for id in [0u8, 15, 50, 127] {
        let mut b = HEADER.to_vec();
        b.extend_from_slice(&[id, 0]);
        assert_eq!(decode_err(&b), DecodeError::UnknownSection(id), "id {id}");
        // also after valid sections
        let mut b = encode(&big_module());
        b.extend_from_slice(&[id, 0]);
        assert_eq!(decode_err(&b), DecodeError::UnknownSection(id), "id {id} after sections");
    }
}

#[test]
fn trailing_garbage_rejected() {
    let bytes = big_bytes();
    let tails: [&[u8]; 9] = [
        &[0],
        &[1],
        &[0x55],
        &[0xAA],
        &[0xAA, 0x05],
        &[0xAA, 0x80],
        &[0, 1, 2, 3],
        &[1, 0],
        &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ];
    for t in tails {
        let mut b = bytes.clone();
        b.extend_from_slice(t);
        assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })), "tail {t:?}");
        let mut b = HEADER.to_vec();
        b.extend_from_slice(t);
        assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })), "bare tail {t:?}");
    }
}

#[test]
fn overlong_leb_in_section_size_rejected() {
    // 6-byte u32 for the size of a (valid, empty-ish) types section.
    for lead in [1u8, 2, 3, 9, 200] {
        let mut b = HEADER.to_vec();
        b.extend_from_slice(&[lead, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
        assert_eq!(decode_err(&b), DecodeError::LebTooLong, "section {lead}");
    }
    // 5th byte with bits that do not fit a u32.
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[1, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F]);
    assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })));
    // Zero-padded within the max length is accepted: types section of size 1 as 5-byte LEB.
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[1, 0x81, 0x80, 0x80, 0x80, 0x00, 0x00]);
    let m = Module::decode(&sealed(&b)).unwrap();
    assert!(m.data().types.is_empty());
}

#[test]
fn huge_counts_rejected_without_allocating() {
    let huge = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]; // u32::MAX
    for id in [1u8, 2, 3, 4, 6, 7, 8, 10, 11] {
        let mut b = HEADER.to_vec();
        b.push(id);
        b.push(huge.len() as u8);
        b.extend_from_slice(&huge);
        assert_eq!(decode_err(&b), DecodeError::CountTooLarge, "section {id}");
        // some trailing bytes inside the section, still far short of the count
        let mut b = HEADER.to_vec();
        b.push(id);
        b.push(huge.len() as u8 + 4);
        b.extend_from_slice(&huge);
        b.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(decode_err(&b), DecodeError::CountTooLarge, "section {id} + bytes");
    }
    // code section: with a matching functions section the count is the failing thing.
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[1, 3, 1, 0, 0]); // types: one ()->()
    b.extend_from_slice(&[3, 3, 1, 0, 0]); // functions: 1 of type 0, nregs 0
    b.push(9);
    b.push(huge.len() as u8);
    b.extend_from_slice(&huge);
    assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })));
    // instruction count in a body
    let mut b = HEADER.to_vec();
    b.extend_from_slice(&[1, 3, 1, 0, 0]);
    b.extend_from_slice(&[3, 3, 1, 0, 0]);
    b.push(9);
    b.push(1 + huge.len() as u8);
    b.push(1);
    b.extend_from_slice(&huge);
    assert_eq!(decode_err(&b), DecodeError::CountTooLarge);
    // string length, vec(byte) length, params count
    let mut b = HEADER.to_vec();
    let mut p = vec![1u8]; // one export
    p.extend_from_slice(&huge); // name length
    b.push(7);
    b.push(p.len() as u8);
    b.extend_from_slice(&p);
    assert!(matches!(Module::decode(&sealed(&b)), Err(LoadError::Decode { .. })));
    let mut b = HEADER.to_vec();
    let mut p = vec![1u8, 1]; // one type; one param count...
    p[1] = 0xFF;
    p.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x0F]);
    b.push(1);
    b.push(p.len() as u8);
    b.extend_from_slice(&p);
    assert_eq!(decode_err(&b), DecodeError::CountTooLarge);
}

// ---------------------------------------------------------------------------------------------
// 4. Never panics
// ---------------------------------------------------------------------------------------------

/// A small module that actually executes: loops, calls, imports, memory, globals, br_table.
fn runnable_module() -> ModuleData {
    let mut b = ModuleBuilder::new();
    let t_ii = b.add_type(FuncType::new([ValType::I32], [ValType::I32]));
    let imp = b.import("env", "host", FuncType::new([ValType::I32], [ValType::I32]));
    b.memory(1, Some(2));
    let g = b.global(ValType::I32, true, 5);
    b.data(8, &[1, 2, 3, 4]);
    let mut fb = FunctionBuilder::new(8);
    let top = fb.new_label();
    let l0 = fb.new_label();
    let l1 = fb.new_label();
    let end = fb.new_label();
    fb.global_get(1, g);
    fb.call(imp, 2);
    fb.br_table(0, &[l0, l1], end);
    fb.bind(l0);
    fb.select(3, 1, 2, 0);
    fb.bind(l1);
    fb.global_set(g, 3);
    fb.bind(top);
    fb.br_if(0, top);
    fb.call_indirect(t_ii, 1, 4);
    fb.bind(end);
    fb.ret(3);
    let f = b.add_func(FuncType::new([ValType::I32], [ValType::I32]), fb);
    b.table(vec![Some(f), Some(imp), None]);
    b.export_func("main", f);
    b.export_func("imp", imp);
    b.export_global("g", g);
    b.func_name(f, "main");
    // A provided function taking `str` and a struct, so the seed carries a valid section 13.
    let mut tb = FunctionBuilder::new(4);
    tb.ret(0);
    let take = b.add_func(FuncType::new([ValType::I64; 3], [ValType::I64]), tb);
    let item = StructDesc::new("Item", 8, 4)
        .field(FieldDesc::new("count", 0, Prim::U32, 1))
        .field(FieldDesc::new("weight", 4, Prim::F32, 1).with_doc("kg"))
        .doc("An item.");
    let mut transform = StructDesc::new("Transform", 12, 4).field(FieldDesc::new("pos", 0, Prim::F32, 3));
    transform.owner = "engine".into();
    transform.component = Some(7);
    b.interface(Interface {
        scratch: Some((64, 256)),
        structs: vec![item, transform],
        provides: vec![
            Provide {
                name: "give".into(),
                func: take,
                doc: "Give an item.".into(),
                params: vec![Param::new("text", ApiType::Str), Param::new("item", ApiType::Struct("Item".into()))],
                results: vec![Param::new("n", ApiType::I64)],
            },
            Provide {
                name: "run".into(),
                func: f,
                doc: "Run main.".into(),
                params: vec![Param::new("x", ApiType::I32)],
                results: vec![Param::new("r", ApiType::I32)],
            },
        ],
        events: vec![ProvidedEvent { name: "dropped".into(), payload: Some("Item".into()), doc: "Dropped.".into() }],
    });
    // Section 14: events the module sends or reads, so the seed carries a valid one.
    assert_eq!(b.event_ref("game", "score_changed", Some(("Score", 8))), 0);
    assert_eq!(b.event_ref("engine.input", "jump", None), 1);
    b.finish().expect("runnable module builds").data().clone()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn instantiate_and_run(module: &Module) -> bool {
    let data = module.data();
    let mut reg = HostRegistry::<()>::new();
    for (ns, v) in data.api.iter().flat_map(|a| &a.namespaces) {
        let _ = reg.define_namespace(ns, *v, "test", "Test functions.");
    }
    for imp in &data.imports {
        let ty = data.types[imp.type_idx as usize].clone();
        // Invalid names cannot be registered; such a module then fails to link.
        let _ = try_host(&mut reg, &imp.namespace, &imp.name, ty, |_c, _a, _r| Ok(HostOk::Continue));
    }
    for e in &data.events {
        let _ = reg.define_namespace(&e.namespace, 1, "test", "Test events.");
        if let Some((s, size)) = &e.payload {
            let _ = reg.register_type(StructDesc::new(s.clone(), *size, 1));
        }
        let mut d = EventDesc::new(e.namespace.clone(), e.name.clone()).doc("Test event.");
        d.payload = e.payload.as_ref().map(|(s, _)| s.clone());
        let _ = reg.register_event(d);
    }
    let limits = Limits { max_memory_pages: 16, max_call_depth: 64, max_registers: 1 << 16, max_fibers: 8 };
    let Ok(mut inst) = common::new_instance(module, &reg.imports(), limits) else {
        return false;
    };
    for e in &data.exports {
        if let ExportKind::Func(i) = e.kind {
            let nparams = module.func_type(i).map(|t| t.params.len()).unwrap_or(0);
            if let Ok(f) = inst.spawn(&e.name, &vec![0u64; nparams]) {
                let _ = inst.resume(f, &mut (), 10_000);
            }
        }
    }
    true
}

fn try_decode_and_run(bytes: &[u8]) -> (bool, bool) {
    match Module::decode(bytes) {
        Ok(m) => (true, instantiate_and_run(&m)),
        Err(_) => (false, false),
    }
}

fn guarded(bytes: &[u8]) -> (bool, bool) {
    match catch_unwind(AssertUnwindSafe(|| try_decode_and_run(bytes))) {
        Ok(r) => r,
        Err(p) => {
            eprintln!("PANIC on input ({} bytes): {}", bytes.len(), hex(bytes));
            resume_unwind(p)
        }
    }
}

#[test]
fn every_truncation_is_rejected() {
    let seeds = [big_bytes(), encode(&runnable_module())];
    for bytes in &seeds {
        assert!(Module::decode(bytes).is_ok());
        let boundaries: HashSet<usize> = {
            let mut s: HashSet<usize> = sections(bytes).iter().map(|x| x.2).collect();
            s.insert(HEADER.len());
            s
        };
        for len in 0..bytes.len() {
            // Sealed, so the cut is what fails, not the checksum.
            let prefix = &sealed(&bytes[..len]);
            let r = catch_unwind(AssertUnwindSafe(|| Module::decode(prefix)));
            let r = match r {
                Ok(r) => r,
                Err(p) => {
                    eprintln!("PANIC on prefix {len}");
                    resume_unwind(p)
                }
            };
            // A prefix that ends exactly at a section boundary is a legal module with trailing
            // sections missing (missing section == empty); it may verify or not. Everything else
            // cuts a header/section short and must be rejected.
            if !boundaries.contains(&len) {
                assert!(r.is_err(), "prefix of {len}/{} bytes decoded", bytes.len());
            }
            // The unverified decoder must not panic either.
            let _ = decode(prefix);
        }
    }
    // Section-boundary prefixes of the big module must always decode unverified, except
    // between `functions` and `code` (sections 3..=8).
    let bytes = big_bytes();
    for (id, _, end) in sections(&bytes) {
        let r = decode(&sealed(&bytes[..end]));
        if (3..=8).contains(&id) {
            assert!(r.is_err(), "functions without code must be rejected (through section {id})");
        } else {
            assert!(r.is_ok(), "prefix through section {id}: {r:?}");
        }
    }
}

fn mutate(rng: &mut Rng, b: &mut Vec<u8>) {
    if b.is_empty() {
        b.push(rng.next() as u8);
        return;
    }
    let n = b.len();
    match rng.below(9) {
        0 => {
            let i = rng.below(n);
            b[i] ^= 1 << rng.below(8);
        }
        1 => {
            let i = rng.below(n);
            b[i] = rng.next() as u8;
        }
        2 => {
            let i = rng.below(n);
            b[i] = [0x00, 0x7F, 0x80, 0xFF][rng.below(4)];
        }
        3 => {
            let i = rng.below(n + 1);
            b.insert(i, rng.next() as u8);
        }
        4 => {
            let i = rng.below(n);
            b.remove(i);
        }
        5 => {
            let s = rng.below(n);
            let l = 1 + rng.below((n - s).min(16));
            let chunk = b[s..s + l].to_vec();
            let at = rng.below(n + 1);
            for (k, x) in chunk.into_iter().enumerate() {
                b.insert(at + k, x);
            }
        }
        6 => {
            let i = rng.below(n);
            for k in 0..5 {
                if i + k < b.len() {
                    b[i + k] = 0xFF;
                }
            }
        }
        7 => {
            // 5 continuation bytes then a terminator: over-long LEB
            let i = rng.below(n);
            for k in 0..5 {
                if i + k < b.len() {
                    b[i + k] = 0x80;
                }
            }
        }
        _ => {
            // truncate
            let i = rng.below(n);
            b.truncate(i);
        }
    }
}

#[test]
fn random_mutations_never_panic() {
    const ITERATIONS: usize = 50_000;
    let seeds = [big_bytes(), encode(&all_sections_module(MemoryType { min_pages: 1, max_pages: Some(2) })), encode(&runnable_module())];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut ok, mut ran, mut rejected) = (0usize, 0usize, 0usize);
    for it in 0..ITERATIONS {
        let mut b = seeds[it % seeds.len()].clone();
        // Only mutate the big seed lightly in position choice: mostly 1 mutation, sometimes more.
        let rounds = 1 + rng.below(3);
        for _ in 0..rounds {
            mutate(&mut rng, &mut b);
        }
        // Re-seal most mutants so they reach the section decoders and the verifier.
        if rng.below(4) != 0 {
            efevm::binary::seal(&mut b);
        }
        let (decoded, ran_it) = guarded(&b);
        ok += decoded as usize;
        ran += ran_it as usize;
        rejected += !decoded as usize;
    }
    eprintln!("mutations: {ITERATIONS}, decoded ok: {ok}, instantiated+ran: {ran}, rejected: {rejected}");
    assert!(rejected > 0);
    assert!(ran > 0, "no mutated module ever reached execution; seeds are too fragile");
}

#[test]
fn unmutated_seeds_run() {
    // Sanity: the execution path of the fuzz test is exercised by the unmutated runnable module.
    let (decoded, ran) = guarded(&encode(&runnable_module()));
    assert!(decoded && ran);
    let (decoded, ran) = guarded(&big_bytes());
    assert!(decoded && ran);
}

#[test]
fn table_entry_u32_max_round_trips() {
    // Table entry encoding is "function index + 1" as a u32 uleb; the largest representable
    // function index is u32::MAX - 1. Some(u32::MAX) would need index+1 = 2^32.
    let m = ModuleData { table: vec![Some(u32::MAX - 1), None], ..Default::default() };
    assert_eq!(decode(&encode(&m)).unwrap(), m);
    let m = ModuleData { table: vec![Some(u32::MAX)], ..Default::default() };
    let bytes = encode(&m);
    assert_eq!(decode(&bytes).unwrap(), m);
}
