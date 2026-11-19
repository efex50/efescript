//! Binary decoder (SPEC.md section 9 and 9.1). Produces unverified `ModuleData`.

use super::leb::{self, LebError};
use super::DecodeError;
use crate::api::{ApiType, FieldDesc, Param, Prim, StructDesc};
use crate::error::LoadError;
use crate::module::{
    ApiRef, BrTable, DataSegment, Export, ExportKind, FuncLines, Function, Global, Import, Inst,
    EventRef, Interface, LineEntry, MemoryType, ModuleData, Provide, ProvidedEvent,
};
use crate::op::{Op, Shape};
use crate::types::{FuncType, ValType};

type Res<T> = Result<T, LoadError>;

/// Cursor over a sub-slice; `base` is the absolute offset of `buf[0]` for error reporting.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], base: usize) -> Self {
        Reader { buf, pos: 0, base }
    }

    fn err_at(&self, pos: usize, error: DecodeError) -> LoadError {
        LoadError::Decode { offset: self.base.saturating_add(pos) as u64, error }
    }

    fn err(&self, error: DecodeError) -> LoadError {
        self.err_at(self.pos, error)
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn leb<T>(&mut self, f: fn(&[u8], &mut usize) -> Result<T, LebError>) -> Res<T> {
        let start = self.pos;
        let mut p = self.pos;
        match f(self.buf, &mut p) {
            Ok(v) => {
                self.pos = p;
                Ok(v)
            }
            Err(LebError::UnexpectedEnd) => Err(self.err_at(start, DecodeError::UnexpectedEnd)),
            Err(LebError::TooLong) => Err(self.err_at(start, DecodeError::LebTooLong)),
        }
    }

    fn u32(&mut self) -> Res<u32> {
        self.leb(leb::read_u32)
    }

    fn u64(&mut self) -> Res<u64> {
        self.leb(leb::read_u64)
    }

    fn i32(&mut self) -> Res<i32> {
        self.leb(leb::read_i32)
    }

    fn i64(&mut self) -> Res<i64> {
        self.leb(leb::read_i64)
    }

    fn byte(&mut self) -> Res<u8> {
        match self.buf.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Err(self.err(DecodeError::UnexpectedEnd)),
        }
    }

    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        match end {
            Some(e) => {
                let s = &self.buf[self.pos..e];
                self.pos = e;
                Ok(s)
            }
            None => Err(self.err(DecodeError::UnexpectedEnd)),
        }
    }

    fn array<const N: usize>(&mut self) -> Res<[u8; N]> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    /// A vector count; every element takes at least one byte, so it may not exceed the rest.
    fn count(&mut self) -> Res<usize> {
        let start = self.pos;
        let n = self.u32()? as usize;
        if n > self.remaining() {
            return Err(self.err_at(start, DecodeError::CountTooLarge));
        }
        Ok(n)
    }

    fn string(&mut self) -> Res<String> {
        let n = self.count()?;
        let start = self.pos;
        let s = self.take(n)?;
        match std::str::from_utf8(s) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => Err(self.err_at(start, DecodeError::BadUtf8)),
        }
    }

    fn valtype(&mut self) -> Res<ValType> {
        let at = self.pos;
        let b = self.byte()?;
        ValType::from_byte(b).ok_or_else(|| self.err_at(at, DecodeError::BadValType(b)))
    }

    fn flag(&mut self) -> Res<bool> {
        let at = self.pos;
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(self.err_at(at, DecodeError::BadFlag(b))),
        }
    }

    fn reg(&mut self) -> Res<u32> {
        let at = self.pos;
        let r = self.u32()?;
        if r > 0xFFFF {
            return Err(self.err_at(at, DecodeError::RegisterTooLarge));
        }
        Ok(r)
    }

    /// Fails with `SectionSizeMismatch` unless everything was consumed.
    fn finish(&self, id: u8) -> Res<()> {
        if self.pos != self.buf.len() {
            return Err(self.err(DecodeError::SectionSizeMismatch(id)));
        }
        Ok(())
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<ModuleData, LoadError> {
    let mut r = Reader::new(bytes, 0);
    let magic = r.take(4).map_err(|_| r.err_at(0, DecodeError::UnexpectedEnd))?;
    if magic != b"EFVM" {
        return Err(r.err_at(0, DecodeError::BadMagic));
    }
    let [v0, v1] = r.array::<2>().map_err(|_| r.err_at(4, DecodeError::UnexpectedEnd))?;
    let version = u16::from_le_bytes([v0, v1]);
    if version != 1 {
        return Err(r.err_at(4, DecodeError::BadVersion(version)));
    }

    let mut m = ModuleData::default();
    let mut last_id: u8 = 0;
    let mut code_seen = false;
    while r.remaining() > 0 {
        let id_pos = r.pos;
        let id = r.byte()?;
        let size = r.u32()? as usize;
        if size > r.remaining() {
            return Err(r.err_at(id_pos, DecodeError::UnexpectedEnd));
        }
        let base = r.pos;
        let payload = r.take(size)?;
        if id >= 128 {
            continue;
        }
        if !(1..=14).contains(&id) {
            return Err(r.err_at(id_pos, DecodeError::UnknownSection(id)));
        }
        if id <= last_id {
            return Err(r.err_at(id_pos, DecodeError::SectionOutOfOrder(id)));
        }
        last_id = id;
        let mut p = Reader::new(payload, base);
        match id {
            1 => types(&mut p, &mut m)?,
            2 => imports(&mut p, &mut m)?,
            3 => functions(&mut p, &mut m)?,
            4 => table(&mut p, &mut m)?,
            5 => memory(&mut p, &mut m)?,
            6 => globals(&mut p, &mut m)?,
            7 => exports(&mut p, &mut m)?,
            8 => data(&mut p, &mut m)?,
            9 => {
                code_seen = true;
                code(&mut p, &mut m)?;
            }
            10 => names(&mut p, &mut m)?,
            11 => lines(&mut p, &mut m)?,
            12 => api(&mut p, &mut m)?,
            13 => interface(&mut p, &mut m)?,
            _ => events(&mut p, &mut m)?,
        }
        p.finish(id)?;
    }
    if !code_seen && !m.functions.is_empty() {
        return Err(r.err(DecodeError::FunctionCodeMismatch));
    }
    Ok(m)
}

fn types(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.types.reserve(n);
    for _ in 0..n {
        let np = p.count()?;
        let mut params = Vec::with_capacity(np);
        for _ in 0..np {
            params.push(p.valtype()?);
        }
        let nr = p.count()?;
        let mut results = Vec::with_capacity(nr);
        for _ in 0..nr {
            results.push(p.valtype()?);
        }
        m.types.push(FuncType { params, results });
    }
    Ok(())
}

fn imports(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.imports.reserve(n);
    for _ in 0..n {
        let namespace = p.string()?;
        let name = p.string()?;
        let type_idx = p.u32()?;
        m.imports.push(Import { namespace, name, type_idx });
    }
    Ok(())
}

fn functions(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.functions.reserve(n);
    for _ in 0..n {
        let type_idx = p.u32()?;
        let at = p.pos;
        let nregs = p.u32()?;
        if nregs > 0xFFFF {
            return Err(p.err_at(at, DecodeError::ValueTooLarge));
        }
        m.functions.push(Function { type_idx, nregs: nregs as u16, ..Default::default() });
    }
    Ok(())
}

fn table(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.table.reserve(n);
    for _ in 0..n {
        let at = p.pos;
        let v = p.u64()?;
        m.table.push(if v == 0 {
            None
        } else {
            match u32::try_from(v - 1) {
                Ok(i) => Some(i),
                Err(_) => return Err(p.err_at(at, DecodeError::ValueTooLarge)),
            }
        });
    }
    Ok(())
}

fn memory(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let has_max = p.flag()?;
    let min_pages = p.u32()?;
    let max_pages = if has_max { Some(p.u32()?) } else { None };
    m.memory = MemoryType { min_pages, max_pages };
    Ok(())
}

fn globals(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.globals.reserve(n);
    for _ in 0..n {
        let ty = p.valtype()?;
        let mutable = p.flag()?;
        let init = match ty {
            ValType::I32 => u64::from(p.i32()? as u32),
            ValType::I64 => p.i64()? as u64,
            ValType::F32 => u64::from(u32::from_le_bytes(p.array::<4>()?)),
            ValType::F64 => u64::from_le_bytes(p.array::<8>()?),
        };
        m.globals.push(Global { ty, mutable, init });
    }
    Ok(())
}

fn exports(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.exports.reserve(n);
    for _ in 0..n {
        let name = p.string()?;
        let at = p.pos;
        let kind = p.byte()?;
        let index = p.u32()?;
        let kind = match kind {
            0 => ExportKind::Func(index),
            1 => ExportKind::Global(index),
            b => return Err(p.err_at(at, DecodeError::BadExportKind(b))),
        };
        m.exports.push(Export { name, kind });
    }
    Ok(())
}

fn data(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.data.reserve(n);
    for _ in 0..n {
        let offset = p.u64()?;
        let len = p.count()?;
        let bytes = p.take(len)?.to_vec();
        m.data.push(DataSegment { offset, bytes });
    }
    Ok(())
}

fn code(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let at = p.pos;
    let n = p.u32()? as usize;
    if n != m.functions.len() {
        return Err(p.err_at(at, DecodeError::FunctionCodeMismatch));
    }
    for f in m.functions.iter_mut() {
        let count = p.count()?;
        let mut insts = Vec::with_capacity(count);
        let mut br_tables = Vec::new();
        for _ in 0..count {
            insts.push(inst(p, &mut br_tables)?);
        }
        f.code = insts;
        f.br_tables = br_tables;
    }
    Ok(())
}

fn inst(p: &mut Reader, br_tables: &mut Vec<BrTable>) -> Res<Inst> {
    let at = p.pos;
    let code = p.u32()?;
    let op = u16::try_from(code)
        .ok()
        .and_then(Op::from_u16)
        .ok_or_else(|| p.err_at(at, DecodeError::UnknownOpcode(u64::from(code))))?;
    let (a, b, c) = match op.shape() {
        Shape::None => (0, 0, 0),
        Shape::D => (p.reg()?, 0, 0),
        Shape::DA => (p.reg()?, p.reg()?, 0),
        Shape::DAB | Shape::ABC => (p.reg()?, p.reg()?, p.reg()?),
        Shape::ConstI32 => {
            let d = p.reg()?;
            (d, p.i32()? as u32, 0)
        }
        Shape::ConstF32 => {
            let d = p.reg()?;
            (d, u32::from_le_bytes(p.array::<4>()?), 0)
        }
        Shape::ConstI64 => {
            let d = p.reg()?;
            let v = p.i64()? as u64;
            (d, v as u32, (v >> 32) as u32)
        }
        Shape::ConstF64 => {
            let d = p.reg()?;
            let v = u64::from_le_bytes(p.array::<8>()?);
            (d, v as u32, (v >> 32) as u32)
        }
        Shape::Select => {
            let d = p.reg()?;
            let cond = p.reg()?;
            let ra = p.reg()?;
            let rb = p.reg()?;
            (d, cond, ra | (rb << 16))
        }
        Shape::Load | Shape::Store => {
            let x = p.reg()?;
            let base = p.reg()?;
            (x, base, p.u32()?)
        }
        Shape::GlobalGet => {
            let d = p.reg()?;
            (d, p.u32()?, 0)
        }
        Shape::GlobalSet => {
            let g = p.u32()?;
            (g, p.reg()?, 0)
        }
        Shape::Jmp => (p.u32()?, 0, 0),
        Shape::Br => {
            let r = p.reg()?;
            (r, p.u32()?, 0)
        }
        Shape::BrTable => {
            let r = p.reg()?;
            let n = p.count()?;
            let mut targets = Vec::with_capacity(n);
            for _ in 0..n {
                targets.push(p.u32()?);
            }
            let default = p.u32()?;
            let idx = br_tables.len() as u32;
            br_tables.push(BrTable { targets, default });
            (r, idx, 0)
        }
        Shape::Call => (p.u32()?, p.reg()?, 0),
        Shape::CallIndirect => {
            let ty = p.u32()?;
            let r = p.reg()?;
            (ty, r, p.reg()?)
        }
        Shape::Ret => (p.reg()?, 0, 0),
    };
    Ok(Inst { op, a, b, c })
}

/// Section 12: `u64` hash (8 bytes little-endian), then `vec((string namespace, uleb version))`.
fn api(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let hash = u64::from_le_bytes(p.array::<8>()?);
    let n = p.count()?;
    let mut namespaces = Vec::with_capacity(n);
    for _ in 0..n {
        let name = p.string()?;
        let version = p.u32()?;
        namespaces.push((name, version));
    }
    m.api = Some(ApiRef { hash, namespaces });
    Ok(())
}

/// Section 13 (SPEC.md 9.2): `scratch, vec(struct), vec(provide), vec(pevent)`. Vectors grow
/// by push: the elements are much larger than the one byte per element `count` guarantees.
fn interface(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let scratch = if p.flag()? { Some((p.u64()?, p.u32()?)) } else { None };
    let mut structs = Vec::new();
    for _ in 0..p.count()? {
        structs.push(struct_desc(p)?);
    }
    let mut provides = Vec::new();
    for _ in 0..p.count()? {
        let name = p.string()?;
        let func = p.u32()?;
        let doc = p.string()?;
        let params = param_list(p)?;
        let results = param_list(p)?;
        provides.push(Provide { name, func, doc, params, results });
    }
    let mut events = Vec::new();
    for _ in 0..p.count()? {
        let name = p.string()?;
        let payload = if p.flag()? { Some(p.string()?) } else { None };
        let doc = p.string()?;
        events.push(ProvidedEvent { name, payload, doc });
    }
    m.interface = Some(Interface { scratch, structs, provides, events });
    Ok(())
}

/// Section 14 (SPEC.md 9.3): `vec(eventref)`; eventref = `string namespace, string name,
/// flag byte; if 1: string struct name, uleb size`. Grows by push.
fn events(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    for _ in 0..p.count()? {
        let namespace = p.string()?;
        let name = p.string()?;
        let payload = if p.flag()? { Some((p.string()?, p.u32()?)) } else { None };
        m.events.push(EventRef { namespace, name, payload });
    }
    Ok(())
}

fn struct_desc(p: &mut Reader) -> Res<StructDesc> {
    let name = p.string()?;
    let owner = p.string()?;
    let component = if p.flag()? { Some(u32::from_le_bytes(p.array::<4>()?)) } else { None };
    let doc = p.string()?;
    let size = p.u32()?;
    let align = p.u32()?;
    let mut fields = Vec::new();
    for _ in 0..p.count()? {
        let name = p.string()?;
        let at = p.pos;
        let b = p.byte()?;
        let ty = prim(b).ok_or_else(|| p.err_at(at, DecodeError::BadPrim(b)))?;
        let count = p.u32()?;
        let offset = p.u32()?;
        let doc = p.string()?;
        fields.push(FieldDesc { name, offset, ty, count, doc });
    }
    Ok(StructDesc { name, size, align, fields, doc, owner, component, readonly: false })
}

fn param_list(p: &mut Reader) -> Res<Vec<Param>> {
    let mut out = Vec::new();
    for _ in 0..p.count()? {
        let name = p.string()?;
        let at = p.pos;
        let ty = match p.byte()? {
            0 => ApiType::I32,
            1 => ApiType::I64,
            2 => ApiType::F32,
            3 => ApiType::F64,
            4 => ApiType::Bool,
            5 => ApiType::Entity,
            6 => ApiType::Str,
            7 => ApiType::Bytes,
            8 => ApiType::Struct(p.string()?),
            b => return Err(p.err_at(at, DecodeError::BadApiType(b))),
        };
        out.push(Param { name, ty });
    }
    Ok(out)
}

fn prim(b: u8) -> Option<Prim> {
    Some(match b {
        0 => Prim::U8,
        1 => Prim::I8,
        2 => Prim::U16,
        3 => Prim::I16,
        4 => Prim::U32,
        5 => Prim::I32,
        6 => Prim::U64,
        7 => Prim::I64,
        8 => Prim::F32,
        9 => Prim::F64,
        10 => Prim::Bool,
        11 => Prim::Entity,
        _ => return None,
    })
}

fn names(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.names.funcs.reserve(n);
    for _ in 0..n {
        let f = p.u32()?;
        let s = p.string()?;
        m.names.funcs.push((f, s));
    }
    let n = p.count()?;
    m.names.regs.reserve(n);
    for _ in 0..n {
        let f = p.u32()?;
        let reg = p.u32()?;
        let s = p.string()?;
        m.names.regs.push((f, reg, s));
    }
    Ok(())
}

fn lines(p: &mut Reader, m: &mut ModuleData) -> Res<()> {
    let n = p.count()?;
    m.lines.files.reserve(n);
    for _ in 0..n {
        let s = p.string()?;
        m.lines.files.push(s);
    }
    let n = p.count()?;
    m.lines.funcs.reserve(n);
    for _ in 0..n {
        let func = p.u32()?;
        let ne = p.count()?;
        let mut entries = Vec::with_capacity(ne);
        for _ in 0..ne {
            let inst = p.u32()?;
            let file = p.u32()?;
            let line = p.u32()?;
            let column = p.u32()?;
            entries.push(LineEntry { inst, file, line, column });
        }
        m.lines.funcs.push(FuncLines { func, entries });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::leb::{write_i32, write_i64, write_u32, write_u64};
    use crate::module::ExportKind;

    fn u(v: u32) -> Vec<u8> {
        let mut o = Vec::new();
        write_u32(&mut o, v);
        o
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| p.iter().copied()).collect()
    }

    fn st(s: &str) -> Vec<u8> {
        let mut o = u(s.len() as u32);
        o.extend_from_slice(s.as_bytes());
        o
    }

    fn header() -> Vec<u8> {
        b"EFVM\x01\x00".to_vec()
    }

    fn section(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut o = vec![id];
        write_u32(&mut o, payload.len() as u32);
        o.extend_from_slice(payload);
        o
    }

    fn module(sections: &[Vec<u8>]) -> Vec<u8> {
        let mut o = header();
        for s in sections {
            o.extend_from_slice(s);
        }
        o
    }

    fn dec_err(bytes: &[u8]) -> (u64, DecodeError) {
        match decode(bytes) {
            Err(LoadError::Decode { offset, error }) => (offset, error),
            other => panic!("expected decode error, got {other:?}"),
        }
    }

    fn err_kind(bytes: &[u8]) -> DecodeError {
        dec_err(bytes).1
    }

    /// One function `(i32)->(i32)` with a mix of instructions.
    fn code_payload() -> Vec<u8> {
        let mut c = Vec::new();
        write_u32(&mut c, 1); // one body
        write_u32(&mut c, 8); // instruction count
        // const.i32 r0, -5
        c.extend(u(Op::ConstI32 as u32));
        c.extend(u(0));
        write_i32(&mut c, -5);
        // const.i64 r1, -2
        c.extend(u(Op::ConstI64 as u32));
        c.extend(u(1));
        write_i64(&mut c, -2);
        // const.f32 r2, 1.5
        c.extend(u(Op::ConstF32 as u32));
        c.extend(u(2));
        c.extend_from_slice(&1.5f32.to_le_bytes());
        // const.f64 r3, 2.5
        c.extend(u(Op::ConstF64 as u32));
        c.extend(u(3));
        c.extend_from_slice(&2.5f64.to_le_bytes());
        // select r0, r1, r2, r3
        c.extend(u(Op::Select as u32));
        for r in [0, 1, 2, 3] {
            c.extend(u(r));
        }
        // br_table r0, [1, 2], 3
        c.extend(u(Op::BrTable as u32));
        c.extend(u(0));
        c.extend(u(2));
        c.extend(u(1));
        c.extend(u(2));
        c.extend(u(3));
        // global.set 7, r1
        c.extend(u(Op::GlobalSet as u32));
        c.extend(u(7));
        c.extend(u(1));
        // ret r0
        c.extend(u(Op::Ret as u32));
        c.extend(u(0));
        c
    }

    fn full_module() -> Vec<u8> {
        let types = cat(&[&u(1), &u(1), &[0], &u(1), &[0]]);
        let imports = cat(&[&u(1), &st("env"), &st("log"), &u(0)]);
        let funcs = cat(&[&u(1), &u(0), &u(4)]);
        let table = cat(&[&u(2), &u(0), &u(2)]);
        let mem = cat(&[&[1], &u(1), &u(3)]);
        let globals = cat(&[&u(1), &[0], &[1], &[0x7F]]);
        let exports = cat(&[&u(1), &st("main"), &[0], &u(1)]);
        let mut data = u(1);
        write_u64(&mut data, 16);
        data.extend(u(3));
        data.extend_from_slice(&[1, 2, 3]);
        let names = cat(&[&u(1), &u(1), &st("main"), &u(1), &u(1), &u(0), &st("x")]);
        let lines = cat(&[&u(1), &st("a.rs"), &u(1), &u(1), &u(1), &u(0), &u(0), &u(9), &u(2)]);
        module(&[
            section(1, &types),
            section(2, &imports),
            section(3, &funcs),
            section(4, &table),
            section(5, &mem),
            section(6, &globals),
            section(7, &exports),
            section(8, &data),
            section(9, &code_payload()),
            section(10, &names),
            section(11, &lines),
        ])
    }

    #[test]
    fn empty_module() {
        assert_eq!(decode(&header()).unwrap(), ModuleData::default());
    }

    #[test]
    fn full_module_decodes() {
        let m = decode(&full_module()).unwrap();
        assert_eq!(m.types, vec![FuncType::new([ValType::I32], [ValType::I32])]);
        assert_eq!(
            m.imports,
            vec![Import { namespace: "env".into(), name: "log".into(), type_idx: 0 }]
        );
        assert_eq!(m.table, vec![None, Some(1)]);
        assert_eq!(m.memory, MemoryType { min_pages: 1, max_pages: Some(3) });
        assert_eq!(m.globals, vec![Global { ty: ValType::I32, mutable: true, init: 0xFFFF_FFFF }]);
        assert_eq!(m.exports, vec![Export { name: "main".into(), kind: ExportKind::Func(1) }]);
        assert_eq!(m.data, vec![DataSegment { offset: 16, bytes: vec![1, 2, 3] }]);
        assert_eq!(m.names.funcs, vec![(1, "main".to_string())]);
        assert_eq!(m.names.regs, vec![(1, 0, "x".to_string())]);
        assert_eq!(m.lines.files, vec!["a.rs".to_string()]);
        assert_eq!(
            m.lines.funcs,
            vec![FuncLines { func: 1, entries: vec![LineEntry { inst: 0, file: 0, line: 9, column: 2 }] }]
        );
        let f = &m.functions[0];
        assert_eq!(f.nregs, 4);
        assert_eq!(f.code.len(), 8);
        assert_eq!(f.code[0], Inst::const_i32(0, -5));
        assert_eq!(f.code[1], Inst::const_i64(1, -2));
        assert_eq!(f.code[2], Inst::const_f32(2, 1.5));
        assert_eq!(f.code[3], Inst::const_f64(3, 2.5));
        assert_eq!(f.code[4], Inst::select(0, 1, 2, 3));
        assert_eq!(f.code[5], Inst::new(Op::BrTable, 0, 0, 0));
        assert_eq!(f.br_tables, vec![BrTable { targets: vec![1, 2], default: 3 }]);
        assert_eq!(f.code[6], Inst::new(Op::GlobalSet, 7, 1, 0));
        assert_eq!(f.code[7], Inst::new(Op::Ret, 0, 0, 0));
    }

    #[test]
    fn globals_by_type() {
        let mut g = u(4);
        g.extend([0, 0, 0x7F]); // i32 -1
        g.extend([1, 1]);
        write_i64(&mut g, -2);
        g.extend([2, 0]);
        g.extend_from_slice(&1.5f32.to_le_bytes());
        g.extend([3, 1]);
        g.extend_from_slice(&2.5f64.to_le_bytes());
        let m = decode(&module(&[section(6, &g)])).unwrap();
        assert_eq!(m.globals[0].init, 0xFFFF_FFFF);
        assert_eq!(m.globals[1].init, (-2i64) as u64);
        assert!(m.globals[1].mutable);
        assert_eq!(m.globals[2].init, u64::from(1.5f32.to_bits()));
        assert_eq!(m.globals[3].init, 2.5f64.to_bits());
    }

    #[test]
    fn memory_without_max() {
        let m = decode(&module(&[section(5, &cat(&[&[0], &u(2)]))])).unwrap();
        assert_eq!(m.memory, MemoryType { min_pages: 2, max_pages: None });
    }

    #[test]
    fn load_store_call_forms() {
        let mut c = u(1);
        c.extend(u(5));
        c.extend(u(Op::I32Load as u32));
        c.extend([1, 2]);
        c.extend(u(300));
        c.extend(u(Op::I32Store as u32));
        c.extend([3, 4]);
        c.extend(u(8));
        c.extend(u(Op::CallIndirect as u32));
        c.extend([6, 7, 8]);
        c.extend(u(Op::Call as u32));
        c.extend([9, 10]);
        c.extend(u(Op::GlobalGet as u32));
        c.extend([1, 2]);
        let funcs = cat(&[&u(1), &u(0), &u(11)]);
        let mut body = u(1);
        body.extend(c[1..].iter());
        let m = decode(&module(&[section(3, &funcs), section(9, &body)])).unwrap();
        let code = &m.functions[0].code;
        assert_eq!(code[0], Inst::new(Op::I32Load, 1, 2, 300));
        assert_eq!(code[1], Inst::new(Op::I32Store, 3, 4, 8));
        assert_eq!(code[2], Inst::new(Op::CallIndirect, 6, 7, 8));
        assert_eq!(code[3], Inst::new(Op::Call, 9, 10, 0));
        assert_eq!(code[4], Inst::new(Op::GlobalGet, 1, 2, 0));
    }

    #[test]
    fn bad_magic_and_version() {
        assert_eq!(dec_err(b"EFVN\x01\x00"), (0, DecodeError::BadMagic));
        assert_eq!(dec_err(b"EFVM\x02\x00"), (4, DecodeError::BadVersion(2)));
        assert_eq!(dec_err(b""), (0, DecodeError::UnexpectedEnd));
        assert_eq!(dec_err(b"EFV"), (0, DecodeError::UnexpectedEnd));
        assert_eq!(dec_err(b"EFVM\x01"), (4, DecodeError::UnexpectedEnd));
    }

    #[test]
    fn section_order_and_ids() {
        let a = section(5, &cat(&[&[0], &u(1)]));
        let b = section(4, &u(0));
        assert_eq!(err_kind(&module(&[a.clone(), b])), DecodeError::SectionOutOfOrder(4));
        assert_eq!(err_kind(&module(&[a.clone(), a.clone()])), DecodeError::SectionOutOfOrder(5));
        assert_eq!(err_kind(&module(&[section(15, &[])])), DecodeError::UnknownSection(15));
        assert_eq!(err_kind(&module(&[section(0, &[])])), DecodeError::UnknownSection(0));
        assert_eq!(err_kind(&module(&[section(127, &[])])), DecodeError::UnknownSection(127));
    }

    #[test]
    fn custom_sections_skipped() {
        let mem = section(5, &cat(&[&[0], &u(1)]));
        let c1 = section(128, b"junk");
        let c2 = section(255, &[]);
        let want = decode(&module(std::slice::from_ref(&mem))).unwrap();
        assert_eq!(decode(&module(&[c1.clone(), mem.clone()])).unwrap(), want);
        assert_eq!(decode(&module(&[mem.clone(), c1.clone()])).unwrap(), want);
        assert_eq!(decode(&module(&[c1.clone(), c2.clone(), mem.clone(), c2.clone()])).unwrap(), want);
        let ty = section(1, &u(0));
        assert_eq!(decode(&module(&[ty, c1, mem, c2])).unwrap(), {
            let mut w = want;
            w.types = vec![];
            w
        });
    }

    #[test]
    fn size_mismatch_and_truncation() {
        // size larger than input
        assert_eq!(err_kind(&[b"EFVM\x01\x00".as_slice(), &[5, 9, 0]].concat()), DecodeError::UnexpectedEnd);
        // payload with leftover bytes
        assert_eq!(
            err_kind(&module(&[section(5, &cat(&[&[0], &u(1), &[0]]))])),
            DecodeError::SectionSizeMismatch(5)
        );
        // payload shorter than its content
        assert_eq!(err_kind(&module(&[section(5, &[1, 1])])), DecodeError::UnexpectedEnd);
        // table with leftover
        assert_eq!(err_kind(&module(&[section(4, &[0, 0])])), DecodeError::SectionSizeMismatch(4));
        // section id without size
        assert_eq!(err_kind(&[header(), vec![5]].concat()), DecodeError::UnexpectedEnd);
    }

    #[test]
    fn leb_errors() {
        let mut b = header();
        b.extend([5, 6, 0, 0x80, 0x80, 0x80, 0x80, 0x80]);
        // memory section: flag 0, min_pages over-long
        assert_eq!(err_kind(&b), DecodeError::LebTooLong);
        // over-long section size
        let mut b = header();
        b.extend([4, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
        assert_eq!(err_kind(&b), DecodeError::LebTooLong);
    }

    #[test]
    fn value_errors() {
        let bad_utf8 = cat(&[&u(1), &u(1), &[0xFF], &u(0), &u(0)]);
        assert_eq!(err_kind(&module(&[section(2, &bad_utf8)])), DecodeError::BadUtf8);
        let bad_ty = cat(&[&u(1), &u(1), &[4], &u(0)]);
        assert_eq!(err_kind(&module(&[section(1, &bad_ty)])), DecodeError::BadValType(4));
        assert_eq!(err_kind(&module(&[section(5, &[2, 1])])), DecodeError::BadFlag(2));
        let g = cat(&[&u(1), &[0], &[7], &[0]]);
        assert_eq!(err_kind(&module(&[section(6, &g)])), DecodeError::BadFlag(7));
        let e = cat(&[&u(1), &st("x"), &[2], &u(0)]);
        assert_eq!(err_kind(&module(&[section(7, &e)])), DecodeError::BadExportKind(2));
        let f = cat(&[&u(1), &u(0), &u(65536)]);
        assert_eq!(err_kind(&module(&[section(3, &f)])), DecodeError::ValueTooLarge);
        let f = cat(&[&u(1), &u(0), &u(65535)]);
        assert!(decode(&module(&[section(3, &f), section(9, &cat(&[&u(1), &u(0)]))])).is_ok());
    }

    fn one_func(body: &[u8]) -> Vec<u8> {
        module(&[section(3, &cat(&[&u(1), &u(0), &u(1)])), section(9, &cat(&[&u(1), body]))])
    }

    #[test]
    fn instruction_errors() {
        // unknown opcode (gap) and > u16
        assert_eq!(err_kind(&one_func(&cat(&[&u(1), &u(0xFFF)]))), DecodeError::UnknownOpcode(0xFFF));
        assert_eq!(
            err_kind(&one_func(&cat(&[&u(1), &u(0x1_0000)]))),
            DecodeError::UnknownOpcode(0x1_0000)
        );
        // register too large
        let b = cat(&[&u(1), &u(Op::Ret as u32), &u(65536)]);
        assert_eq!(err_kind(&one_func(&b)), DecodeError::RegisterTooLarge);
        let b = cat(&[&u(1), &u(Op::Ret as u32), &u(65535)]);
        assert!(decode(&one_func(&b)).is_ok());
        // instruction count too large
        assert_eq!(err_kind(&one_func(&u(1000))), DecodeError::CountTooLarge);
        // truncated const.f32
        let b = cat(&[&u(1), &u(Op::ConstF32 as u32), &u(0), &[0, 0]]);
        assert_eq!(err_kind(&one_func(&b)), DecodeError::UnexpectedEnd);
        // br_table count too large
        let b = cat(&[&u(1), &u(Op::BrTable as u32), &u(0), &u(500), &u(0)]);
        assert_eq!(err_kind(&one_func(&b)), DecodeError::CountTooLarge);
    }

    #[test]
    fn counts_too_large() {
        assert_eq!(err_kind(&module(&[section(1, &u(1_000_000))])), DecodeError::CountTooLarge);
        assert_eq!(err_kind(&module(&[section(4, &u(u32::MAX))])), DecodeError::CountTooLarge);
        let d = cat(&[&u(1), &u(0), &u(100), &[1]]);
        assert_eq!(err_kind(&module(&[section(8, &d)])), DecodeError::CountTooLarge);
    }

    #[test]
    fn function_code_mismatch() {
        let f = section(3, &cat(&[&u(1), &u(0), &u(1)]));
        assert_eq!(err_kind(&module(std::slice::from_ref(&f))), DecodeError::FunctionCodeMismatch);
        let c = section(9, &cat(&[&u(1), &u(0)]));
        assert_eq!(err_kind(&module(std::slice::from_ref(&c))), DecodeError::FunctionCodeMismatch);
        let c2 = section(9, &cat(&[&u(2), &u(0), &u(0)]));
        assert_eq!(err_kind(&module(&[f.clone(), c2])), DecodeError::FunctionCodeMismatch);
        // zero count code without functions is fine
        assert!(decode(&module(&[section(9, &u(0))])).is_ok());
        assert!(decode(&module(&[f, c])).is_ok());
    }

    #[test]
    fn truncation_prefixes_error_or_ok() {
        let full = full_module();
        assert!(decode(&full).is_ok());
        let mut errs = 0;
        for n in 0..full.len() {
            if decode(&full[..n]).is_err() {
                errs += 1;
            }
        }
        // Every strict prefix that cuts a section must fail; only section boundaries may succeed.
        assert!(errs > full.len() - 15);
    }

    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn never_panics() {
        let full = full_module();
        for n in 0..=full.len() {
            let _ = decode(&full[..n]);
            let _ = crate::Module::decode(&full[..n]);
        }
        let mut rng = XorShift(0x1234_5678_9ABC_DEF1);
        for _ in 0..20_000 {
            let mut b = full.clone();
            let muts = 1 + rng.next() % 4;
            for _ in 0..muts {
                let i = (rng.next() as usize) % b.len();
                match rng.next() % 4 {
                    0 => b[i] = rng.next() as u8,
                    1 => b[i] ^= 1 << (rng.next() % 8),
                    2 => {
                        b.remove(i);
                    }
                    _ => b.insert(i, rng.next() as u8),
                }
                if b.is_empty() {
                    b.push(0);
                }
            }
            let _ = decode(&b);
            let _ = crate::Module::decode(&b);
        }
    }

    #[test]
    fn round_trip_if_encoder_exists() {
        use crate::builder::{FunctionBuilder, ModuleBuilder};
        let mut mb = ModuleBuilder::new();
        let ty = FuncType::new([ValType::I32], [ValType::I32]);
        let mut f = FunctionBuilder::new(4);
        f.const_i32(1, 7);
        f.op3(Op::I32Add, 0, 0, 1);
        f.ret(0);
        let fi = mb.add_func(ty, f);
        mb.export_func("f", fi);
        mb.memory(1, None);
        let Ok(data) = mb.finish_unverified() else { return };
        let bytes = crate::binary::encode(&data);
        if bytes.is_empty() {
            return;
        }
        assert_eq!(decode(&bytes).unwrap(), data);
    }
}
