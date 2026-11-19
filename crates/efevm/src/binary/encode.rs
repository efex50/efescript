//! Binary encoder for `.efvm` bytecode modules (SPEC.md section 9).
//!
//! File layout:
//! - 4-byte magic: `b"EFVM"`
//! - 2-byte version: `2u16` in little-endian (`[0x02, 0x00]`)
//! - 4-byte checksum: CRC-32 (IEEE) of every byte after the header, little-endian
//! - Non-empty sections in ascending ID order:
//!   - 1: Types (function signatures)
//!   - 2: Imports (host function declarations)
//!   - 3: Functions (type indices and register counts)
//!   - 4: Table (indirect call targets)
//!   - 5: Memory (linear memory limits)
//!   - 6: Globals (type, mutability, initial value)
//!   - 7: Exports (names and indices of exported functions/globals)
//!   - 8: Data (linear memory data segments)
//!   - 9: Code (function bodies / instruction streams)
//!   - 10: Names (debug names for functions and registers)
//!   - 11: Lines (source file and line mappings)
//!   - 12: Api (index hash and namespace versions; written when `ModuleData::api` is `Some`)
//!   - 13: Interface (scratch, structs, provides, events; written when `ModuleData::interface` is `Some`)
//!   - 14: Events (script event references; written when `ModuleData::events` is non-empty)
//!
//! Each section consists of:
//! - 1-byte section ID
//! - `uleb` u32 payload size
//! - Raw payload bytes

use super::leb;
use crate::api::{ApiType, Prim};
use crate::module::*;
use crate::op::Shape;
use crate::types::ValType;

/// Encodes an unverified [`ModuleData`] structure into `.efvm` binary format.
pub(crate) fn encode(m: &ModuleData) -> Vec<u8> {
    let mut out = Vec::new();

    // Header: b"EFVM", the version as u16 little-endian, then a checksum placeholder that
    // `seal` fills in once every section is written.
    out.extend_from_slice(&super::MAGIC);
    out.extend_from_slice(&super::VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 4]);

    // Section 1: Types
    if !m.types.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.types.len());
        for ft in &m.types {
            write_len(&mut payload, ft.params.len());
            for &ty in &ft.params {
                payload.push(ty as u8);
            }
            write_len(&mut payload, ft.results.len());
            for &ty in &ft.results {
                payload.push(ty as u8);
            }
        }
        emit_section(&mut out, 1, &payload);
    }

    // Section 2: Imports
    if !m.imports.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.imports.len());
        for imp in &m.imports {
            write_str(&mut payload, &imp.namespace);
            write_str(&mut payload, &imp.name);
            leb::write_u32(&mut payload, imp.type_idx);
        }
        emit_section(&mut out, 2, &payload);
    }

    // Section 3: Functions
    if !m.functions.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.functions.len());
        for func in &m.functions {
            leb::write_u32(&mut payload, func.type_idx);
            leb::write_u32(&mut payload, u32::from(func.nregs));
        }
        emit_section(&mut out, 3, &payload);
    }

    // Section 4: Table
    if !m.table.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.table.len());
        for &entry in &m.table {
            let v = match entry {
                None => 0u64,
                Some(f) => (f as u64) + 1,
            };
            leb::write_u64(&mut payload, v);
        }
        emit_section(&mut out, 4, &payload);
    }

    // Section 5: Memory
    if m.memory.min_pages != 0 || m.memory.max_pages.is_some() {
        let mut payload = Vec::new();
        let has_max = m.memory.max_pages.is_some();
        payload.push(if has_max { 1 } else { 0 });
        leb::write_u32(&mut payload, m.memory.min_pages);
        if let Some(max) = m.memory.max_pages {
            leb::write_u32(&mut payload, max);
        }
        emit_section(&mut out, 5, &payload);
    }

    // Section 6: Globals
    if !m.globals.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.globals.len());
        for g in &m.globals {
            payload.push(g.ty as u8);
            payload.push(if g.mutable { 1 } else { 0 });
            match g.ty {
                ValType::I32 => leb::write_i32(&mut payload, g.init as u32 as i32),
                ValType::I64 => leb::write_i64(&mut payload, g.init as i64),
                ValType::F32 => payload.extend_from_slice(&(g.init as u32).to_le_bytes()),
                ValType::F64 => payload.extend_from_slice(&g.init.to_le_bytes()),
            }
        }
        emit_section(&mut out, 6, &payload);
    }

    // Section 7: Exports
    if !m.exports.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.exports.len());
        for exp in &m.exports {
            write_str(&mut payload, &exp.name);
            match exp.kind {
                ExportKind::Func(idx) => {
                    payload.push(0);
                    leb::write_u32(&mut payload, idx);
                }
                ExportKind::Global(idx) => {
                    payload.push(1);
                    leb::write_u32(&mut payload, idx);
                }
            }
        }
        emit_section(&mut out, 7, &payload);
    }

    // Section 8: Data
    if !m.data.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.data.len());
        for seg in &m.data {
            leb::write_u64(&mut payload, seg.offset);
            write_len(&mut payload, seg.bytes.len());
            payload.extend_from_slice(&seg.bytes);
        }
        emit_section(&mut out, 8, &payload);
    }

    // Section 9: Code
    if !m.functions.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.functions.len());
        for func in &m.functions {
            write_len(&mut payload, func.code.len());
            for inst in &func.code {
                leb::write_u32(&mut payload, inst.op.code() as u32);
                match inst.op.shape() {
                    Shape::None => {}
                    Shape::D => {
                        leb::write_u32(&mut payload, inst.a);
                    }
                    Shape::DA => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                    }
                    Shape::DAB => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c);
                    }
                    Shape::ABC => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c);
                    }
                    Shape::ConstI32 => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_i32(&mut payload, inst.b as i32);
                    }
                    Shape::ConstI64 => {
                        leb::write_u32(&mut payload, inst.a);
                        let v = ((inst.c as u64) << 32 | inst.b as u64) as i64;
                        leb::write_i64(&mut payload, v);
                    }
                    Shape::ConstF32 => {
                        leb::write_u32(&mut payload, inst.a);
                        payload.extend_from_slice(&inst.b.to_le_bytes());
                    }
                    Shape::ConstF64 => {
                        leb::write_u32(&mut payload, inst.a);
                        let v = (inst.c as u64) << 32 | inst.b as u64;
                        payload.extend_from_slice(&v.to_le_bytes());
                    }
                    Shape::Select => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c & 0xFFFF);
                        leb::write_u32(&mut payload, inst.c >> 16);
                    }
                    Shape::Load => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c);
                    }
                    Shape::Store => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c);
                    }
                    Shape::GlobalGet => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                    }
                    Shape::GlobalSet => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                    }
                    Shape::Jmp => {
                        leb::write_u32(&mut payload, inst.a);
                    }
                    Shape::Br => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                    }
                    Shape::BrTable => {
                        leb::write_u32(&mut payload, inst.a);
                        if let Some(bt) = func.br_tables.get(inst.b as usize) {
                            write_len(&mut payload, bt.targets.len());
                            for &target in &bt.targets {
                                leb::write_u32(&mut payload, target);
                            }
                            leb::write_u32(&mut payload, bt.default);
                        } else {
                            leb::write_u32(&mut payload, 0);
                            leb::write_u32(&mut payload, 0);
                        }
                    }
                    Shape::Call => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                    }
                    Shape::CallIndirect => {
                        leb::write_u32(&mut payload, inst.a);
                        leb::write_u32(&mut payload, inst.b);
                        leb::write_u32(&mut payload, inst.c);
                    }
                    Shape::Ret => {
                        leb::write_u32(&mut payload, inst.a);
                    }
                }
            }
        }
        emit_section(&mut out, 9, &payload);
    }

    // Section 10: Names
    if !m.names.funcs.is_empty() || !m.names.regs.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.names.funcs.len());
        for (func_idx, name) in &m.names.funcs {
            leb::write_u32(&mut payload, *func_idx);
            write_str(&mut payload, name);
        }
        write_len(&mut payload, m.names.regs.len());
        for (func_idx, reg_idx, name) in &m.names.regs {
            leb::write_u32(&mut payload, *func_idx);
            leb::write_u32(&mut payload, *reg_idx);
            write_str(&mut payload, name);
        }
        emit_section(&mut out, 10, &payload);
    }

    // Section 11: Lines
    if !m.lines.files.is_empty() || !m.lines.funcs.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.lines.files.len());
        for file in &m.lines.files {
            write_str(&mut payload, file);
        }
        write_len(&mut payload, m.lines.funcs.len());
        for fl in &m.lines.funcs {
            leb::write_u32(&mut payload, fl.func);
            write_len(&mut payload, fl.entries.len());
            for entry in &fl.entries {
                leb::write_u32(&mut payload, entry.inst);
                leb::write_u32(&mut payload, entry.file);
                leb::write_u32(&mut payload, entry.line);
                leb::write_u32(&mut payload, entry.column);
            }
        }
        emit_section(&mut out, 11, &payload);
    }

    // Section 12: API index reference (written whenever present, even with no namespaces)
    if let Some(api) = &m.api {
        let mut payload = Vec::new();
        payload.extend_from_slice(&api.hash.to_le_bytes());
        write_len(&mut payload, api.namespaces.len());
        for (name, version) in &api.namespaces {
            write_str(&mut payload, name);
            leb::write_u32(&mut payload, *version);
        }
        emit_section(&mut out, 12, &payload);
    }

    // Section 13: interface (written whenever present, even when empty)
    if let Some(iface) = &m.interface {
        let mut payload = Vec::new();
        interface(&mut payload, iface);
        emit_section(&mut out, 13, &payload);
    }

    // Section 14: event references (written when non-empty)
    if !m.events.is_empty() {
        let mut payload = Vec::new();
        write_len(&mut payload, m.events.len());
        for e in &m.events {
            write_str(&mut payload, &e.namespace);
            write_str(&mut payload, &e.name);
            match &e.payload {
                None => payload.push(0),
                Some((s, size)) => {
                    payload.push(1);
                    write_str(&mut payload, s);
                    leb::write_u32(&mut payload, *size);
                }
            }
        }
        emit_section(&mut out, 14, &payload);
    }

    super::seal(&mut out);
    out
}

/// SPEC.md 9.2: `scratch, vec(struct), vec(provide), vec(pevent)`.
fn interface(out: &mut Vec<u8>, iface: &Interface) {
    match iface.scratch {
        None => out.push(0),
        Some((offset, size)) => {
            out.push(1);
            leb::write_u64(out, offset);
            leb::write_u32(out, size);
        }
    }
    write_len(out, iface.structs.len());
    for s in &iface.structs {
        write_str(out, &s.name);
        write_str(out, &s.owner);
        match s.component {
            None => out.push(0),
            Some(id) => {
                out.push(1);
                out.extend_from_slice(&id.to_le_bytes());
            }
        }
        write_str(out, &s.doc);
        leb::write_u32(out, s.size);
        leb::write_u32(out, s.align);
        write_len(out, s.fields.len());
        for f in &s.fields {
            write_str(out, &f.name);
            out.push(prim_byte(f.ty));
            leb::write_u32(out, f.count);
            leb::write_u32(out, f.offset);
            write_str(out, &f.doc);
        }
    }
    write_len(out, iface.provides.len());
    for p in &iface.provides {
        write_str(out, &p.name);
        leb::write_u32(out, p.func);
        write_str(out, &p.doc);
        for list in [&p.params, &p.results] {
            write_len(out, list.len());
            for param in list {
                write_str(out, &param.name);
                api_type(out, &param.ty);
            }
        }
    }
    write_len(out, iface.events.len());
    for e in &iface.events {
        write_str(out, &e.name);
        match &e.payload {
            None => out.push(0),
            Some(s) => {
                out.push(1);
                write_str(out, s);
            }
        }
        write_str(out, &e.doc);
    }
}

/// `ptr` has no encoding (SPEC.md 9.2) and the verifier rejects it in a provide; the encoder
/// cannot fail, so it writes `0xFF`, which the decoder rejects as `BadApiType(0xFF)`.
const PTR_PLACEHOLDER: u8 = 0xFF;

fn api_type(out: &mut Vec<u8>, t: &ApiType) {
    match t {
        ApiType::I32 => out.push(0),
        ApiType::I64 => out.push(1),
        ApiType::F32 => out.push(2),
        ApiType::F64 => out.push(3),
        ApiType::Bool => out.push(4),
        ApiType::Entity => out.push(5),
        ApiType::Str => out.push(6),
        ApiType::Bytes => out.push(7),
        ApiType::Struct(name) => {
            out.push(8);
            write_str(out, name);
        }
        ApiType::Ptr => out.push(PTR_PLACEHOLDER),
    }
}

fn prim_byte(p: Prim) -> u8 {
    match p {
        Prim::U8 => 0,
        Prim::I8 => 1,
        Prim::U16 => 2,
        Prim::I16 => 3,
        Prim::U32 => 4,
        Prim::I32 => 5,
        Prim::U64 => 6,
        Prim::I64 => 7,
        Prim::F32 => 8,
        Prim::F64 => 9,
        Prim::Bool => 10,
        Prim::Entity => 11,
    }
}

/// Writes a length as a u32 uleb. Lengths above `u32::MAX` saturate silently: modules that
/// large are unsupported.
#[inline]
fn write_len(out: &mut Vec<u8>, len: usize) {
    leb::write_u32(out, u32::try_from(len).unwrap_or(u32::MAX));
}

#[inline]
fn write_str(out: &mut Vec<u8>, s: &str) {
    write_len(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

#[inline]
fn emit_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    write_len(out, payload.len());
    out.extend_from_slice(payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Op;

    #[test]
    fn default_module_header_only() {
        let m = ModuleData::default();
        let bytes = encode(&m);
        assert_eq!(bytes, b"EFVM\x02\x00\0\0\0\0");
    }

    #[test]
    fn types_section_encoding() {
        let m = ModuleData {
            types: vec![crate::types::FuncType::new(
                [ValType::I32, ValType::I64],
                [ValType::F64],
            )],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        // Section 1: id=1, size=6, count=1, params_count=2, i32=0, i64=1, results_count=1, f64=3
        assert_eq!(&bytes[10..], &[1, 6, 1, 2, 0, 1, 1, 3]);
    }

    #[test]
    fn memory_section_encoding() {
        let m = ModuleData {
            memory: MemoryType {
                min_pages: 1,
                max_pages: Some(2),
            },
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(&bytes[10..], &[5, 3, 1, 1, 2]);
    }

    #[test]
    fn code_section_encoding() {
        let func = Function {
            type_idx: 0,
            nregs: 1,
            code: vec![
                Inst::new(Op::ConstI32, 0, (-1i32) as u32, 0),
                Inst::new(Op::Ret, 0, 0, 0),
            ],
            br_tables: Vec::new(),
        };
        let m = ModuleData {
            functions: vec![func],
            ..Default::default()
        };
        let bytes = encode(&m);
        // Header (10 bytes): b"EFVM\x02\x00" + checksum
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        // Functions section (Section 3):
        // id: 3, size: 3, count: 1, type_idx: 0, nregs: 1
        assert_eq!(&bytes[10..15], &[3, 3, 1, 0, 1]);
        // Code section (Section 9):
        // id: 9, size: 7, func_count: 1, inst_count: 2,
        // Op::ConstI32, r0=0, imm=-1 (sleb 0x7F),
        // Op::Ret, base=0
        let expected_code_section = [
            9,
            7,
            1,
            2,
            Op::ConstI32.code() as u8,
            0,
            0x7F,
            Op::Ret.code() as u8,
            0,
        ];
        assert_eq!(&bytes[15..], &expected_code_section);
    }

    #[test]
    fn table_section_encoding() {
        let m = ModuleData {
            table: vec![None, Some(0), Some(u32::MAX)],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        // Section 4 (table): id=4, payload size 8:
        // count: 3
        // None -> 0
        // Some(0) -> 1
        // Some(u32::MAX) -> u32::MAX + 1 = 4294967296 -> [0x80, 0x80, 0x80, 0x80, 0x10]
        assert_eq!(&bytes[10..], &[4, 8, 3, 0, 1, 0x80, 0x80, 0x80, 0x80, 0x10]);
    }

    #[test]
    fn globals_section_encoding() {
        let m = ModuleData {
            globals: vec![
                Global {
                    ty: ValType::I32,
                    mutable: true,
                    init: (-5i32 as u32) as u64,
                },
                Global {
                    ty: ValType::I64,
                    mutable: false,
                    init: (-10i64) as u64,
                },
                Global {
                    ty: ValType::F32,
                    mutable: true,
                    init: 1.5f32.to_bits() as u64,
                },
                Global {
                    ty: ValType::F64,
                    mutable: false,
                    init: 2.5f64.to_bits(),
                },
            ],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(bytes[10], 6); // section 6
    }

    #[test]
    fn exports_section_encoding() {
        let m = ModuleData {
            exports: vec![
                Export {
                    name: "foo".into(),
                    kind: ExportKind::Func(42),
                },
                Export {
                    name: "bar".into(),
                    kind: ExportKind::Global(7),
                },
            ],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(bytes[10], 7); // section 7
    }

    #[test]
    fn data_section_encoding() {
        let m = ModuleData {
            data: vec![DataSegment {
                offset: 1024,
                bytes: vec![1, 2, 3, 4],
            }],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(bytes[10], 8); // section 8
    }

    #[test]
    fn names_section_encoding() {
        let m = ModuleData {
            names: Names {
                funcs: vec![(0, "main".into())],
                regs: vec![(0, 1, "r1".into())],
            },
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(bytes[10], 10); // section 10
    }

    #[test]
    fn lines_section_encoding() {
        let m = ModuleData {
            lines: Lines {
                files: vec!["test.efe".into()],
                funcs: vec![FuncLines {
                    func: 0,
                    entries: vec![LineEntry {
                        inst: 0,
                        file: 0,
                        line: 1,
                        column: 5,
                    }],
                }],
            },
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
        assert_eq!(bytes[10], 11); // section 11
    }

    #[test]
    fn br_table_encoding() {
        let func = Function {
            type_idx: 0,
            nregs: 2,
            code: vec![
                Inst::new(Op::BrTable, 1, 0, 0),  // valid b=0
                Inst::new(Op::BrTable, 1, 99, 0), // out-of-range b=99 (unverified data)
                Inst::new(Op::Ret, 0, 0, 0),
            ],
            br_tables: vec![BrTable {
                targets: vec![10, 20],
                default: 30,
            }],
        };
        let m = ModuleData {
            functions: vec![func],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
    }

    #[test]
    fn all_shapes_encoding() {
        let func = Function {
            type_idx: 0,
            nregs: 10,
            code: vec![
                Inst::new(Op::Nop, 0, 0, 0),          // Shape::None
                Inst::new(Op::MemorySize, 0, 0, 0),   // Shape::D
                Inst::new(Op::Mov, 1, 2, 0),          // Shape::DA
                Inst::new(Op::I32Add, 1, 2, 3),       // Shape::DAB
                Inst::new(Op::MemoryCopy, 1, 2, 3),   // Shape::ABC
                Inst::const_i32(1, -42),              // Shape::ConstI32
                Inst::const_i64(2, -100),             // Shape::ConstI64
                Inst::const_f32(3, 1.25),             // Shape::ConstF32
                Inst::const_f64(4, 0.375),            // Shape::ConstF64
                Inst::select(5, 1, 2, 3),             // Shape::Select
                Inst::new(Op::I32Load, 1, 2, 16),     // Shape::Load
                Inst::new(Op::I32Store, 1, 2, 16),    // Shape::Store
                Inst::new(Op::GlobalGet, 1, 0, 0),    // Shape::GlobalGet
                Inst::new(Op::GlobalSet, 0, 1, 0),    // Shape::GlobalSet
                Inst::new(Op::Jmp, 5, 0, 0),          // Shape::Jmp
                Inst::new(Op::BrIf, 1, 5, 0),         // Shape::Br
                Inst::new(Op::Call, 0, 1, 0),         // Shape::Call
                Inst::new(Op::CallIndirect, 0, 1, 2), // Shape::CallIndirect
                Inst::new(Op::Ret, 0, 0, 0),          // Shape::Ret
            ],
            br_tables: vec![],
        };
        let m = ModuleData {
            functions: vec![func],
            ..Default::default()
        };
        let bytes = encode(&m);
        assert_eq!(&bytes[..6], b"EFVM\x02\x00");
        assert_eq!(&bytes[6..10], &crate::binary::checksum(&bytes).to_le_bytes());
    }
}
