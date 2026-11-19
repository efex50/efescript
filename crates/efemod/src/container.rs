//! The 7z container profile (MOD_PACKAGE.md sections 1, 2, 6, 7).
//!
//! [`read`] accepts only the profile described there and never panics: it checks the
//! signature header and walks the (plain, never encoded) header itself before handing the
//! bytes to `sevenz-rust2`, so every count the crate allocates by is already bounded by
//! [`ReadLimits::entries`] and every coder is COPY or LZMA2 with a bounded dictionary.
//! With the `write` feature, [`write`] is the canonical writer (same inputs, same bytes).

use std::collections::HashSet;
use std::io::{Cursor, Read};

use sevenz_rust2::{Archive, BlockDecoder, Password};

use crate::error::PackageError;
use crate::limits::ReadLimits;

/// One file of an archive (directory entries are not returned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

const SIGNATURE: [u8; 6] = [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];
const SIGNATURE_HEADER_SIZE: u64 = 32;

// Property ids (7-Zip `7zFormat.txt`).
const K_END: u8 = 0x00;
const K_HEADER: u8 = 0x01;
const K_ARCHIVE_PROPERTIES: u8 = 0x02;
const K_ADDITIONAL_STREAMS_INFO: u8 = 0x03;
const K_MAIN_STREAMS_INFO: u8 = 0x04;
const K_FILES_INFO: u8 = 0x05;
const K_PACK_INFO: u8 = 0x06;
const K_UNPACK_INFO: u8 = 0x07;
const K_SUB_STREAMS_INFO: u8 = 0x08;
const K_SIZE: u8 = 0x09;
const K_CRC: u8 = 0x0A;
const K_FOLDER: u8 = 0x0B;
const K_CODERS_UNPACK_SIZE: u8 = 0x0C;
const K_NUM_UNPACK_STREAM: u8 = 0x0D;
const K_ENCODED_HEADER: u8 = 0x17;
#[cfg(feature = "write")]
const K_EMPTY_STREAM: u8 = 0x0E;
#[cfg(feature = "write")]
const K_EMPTY_FILE: u8 = 0x0F;
#[cfg(feature = "write")]
const K_NAME: u8 = 0x11;

/// Coder ids.
const ID_COPY: &[u8] = &[0x00];
const ID_LZMA2: &[u8] = &[0x21];
const ID_AES: &[u8] = &[0x06, 0xF1, 0x07, 0x01];

/// The largest LZMA2 dictionary property (40 means 4 GiB - 1).
const LZMA2_PROP_MAX: u8 = 40;

/// `sevenz-rust2` counts an LZMA2 decoder as its dictionary plus this many KiB
/// (`lzma_rust2::lzma2_get_memory_usage`: 40 + the 64 KiB range-decoder buffer).
const LZMA2_OVERHEAD_KIB: u64 = 40 + 64;

fn container<T>(m: impl Into<String>) -> Result<T, PackageError> {
    Err(PackageError::Container(m.into()))
}

/// The dictionary size an LZMA2 property byte declares (`None` above 40).
fn lzma2_dict_size(prop: u8) -> Option<u64> {
    match prop {
        p if p > LZMA2_PROP_MAX => None,
        LZMA2_PROP_MAX => Some(0xFFFF_FFFF),
        p => Some((2 | (p as u64 & 1)) << (p / 2 + 11)),
    }
}

/// Check one coder against the profile: COPY without properties, or LZMA2 with one
/// property byte whose dictionary fits the limit.
fn check_coder(id: &[u8], props: &[u8], limits: &ReadLimits) -> Result<(), PackageError> {
    match id {
        ID_COPY if props.is_empty() => Ok(()),
        ID_COPY => container("COPY coder with properties"),
        ID_LZMA2 => {
            let [prop] = props else {
                return container(format!("LZMA2 coder with {} property bytes (expected 1)", props.len()));
            };
            let Some(dict) = lzma2_dict_size(*prop) else {
                return container(format!("bad LZMA2 dictionary property {prop}"));
            };
            if dict > limits.dictionary_size {
                return Err(PackageError::TooLarge {
                    what: "LZMA2 dictionary".into(),
                    size: dict,
                    limit: limits.dictionary_size,
                });
            }
            Ok(())
        }
        ID_AES => container("encrypted entries are not supported"),
        _ => {
            let hex: Vec<String> = id.iter().map(|b| format!("{b:02X}")).collect();
            container(format!("coder {} is not supported (only COPY and LZMA2)", hex.join(" ")))
        }
    }
}

/// Check an entry name against the rules of section 2 (without the case-duplicate rule,
/// which needs the whole set). Returns the reason on failure.
pub fn validate_entry_name(name: &str, limits: &ReadLimits) -> Result<(), String> {
    let limits = limits.clamped();
    if name.is_empty() {
        return Err("empty name".into());
    }
    if name.len() > limits.name_bytes {
        return Err(format!("longer than {} bytes", limits.name_bytes));
    }
    if name.starts_with('/') {
        return Err("absolute path".into());
    }
    for c in name.chars() {
        match c {
            '\\' => return Err("contains `\\`".into()),
            ':' => return Err("contains `:`".into()),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                return Err("contains a control character".into());
            }
            _ => {}
        }
    }
    let mut n = 0usize;
    for seg in name.split('/') {
        n += 1;
        match seg {
            "" => return Err("empty path segment".into()),
            "." | ".." => return Err(format!("`{seg}` path segment")),
            _ => {}
        }
    }
    if n > limits.name_segments {
        return Err(format!("more than {} path segments", limits.name_segments));
    }
    Ok(())
}

/// A cursor over the plain header for the pre-check walk.
struct Walk<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Walk<'a> {
    fn u8(&mut self) -> Result<u8, PackageError> {
        let Some(&v) = self.b.get(self.pos) else {
            return container("header ends early");
        };
        self.pos += 1;
        Ok(v)
    }

    fn bytes(&mut self, n: u64) -> Result<&'a [u8], PackageError> {
        let end = usize::try_from(n).ok().and_then(|n| self.pos.checked_add(n));
        let Some(s) = end.and_then(|end| self.b.get(self.pos..end)) else {
            return container("header ends early");
        };
        self.pos += s.len();
        Ok(s)
    }

    /// A 7z NUMBER (the first byte's leading one bits count the extra bytes).
    fn num(&mut self) -> Result<u64, PackageError> {
        let first = self.u8()? as u64;
        let mut mask = 0x80u64;
        let mut value = 0u64;
        for i in 0..8 {
            if first & mask == 0 {
                return Ok(value | ((first & (mask - 1)) << (8 * i)));
            }
            value |= (self.u8()? as u64) << (8 * i);
            mask >>= 1;
        }
        Ok(value)
    }

    /// A count that must not exceed `max` (checked before anything is sized by it).
    fn count(&mut self, max: u64, what: &str) -> Result<u64, PackageError> {
        let n = self.num()?;
        if n > max {
            return Err(PackageError::TooLarge { what: what.into(), size: n, limit: max });
        }
        Ok(n)
    }

    fn expect(&mut self, id: u8, what: &str) -> Result<(), PackageError> {
        let got = self.u8()?;
        if got != id {
            return container(format!("expected {what} (0x{id:02X}) in the header, found 0x{got:02X}"));
        }
        Ok(())
    }

    /// Digests for `n` streams; returns which are defined.
    fn digests(&mut self, n: u64) -> Result<Vec<bool>, PackageError> {
        let all = self.u8()? != 0;
        let defined: Vec<bool> = if all {
            vec![true; n as usize]
        } else {
            let bits = self.bytes(n.div_ceil(8))?;
            (0..n as usize).map(|i| bits[i / 8] & (0x80 >> (i % 8)) != 0).collect()
        };
        let k = defined.iter().filter(|d| **d).count() as u64;
        self.bytes(k * 4)?;
        Ok(defined)
    }
}

/// Walk the plain header up to the file count, checking the profile on the way, before
/// `sevenz-rust2` parses it. Returns nothing: its job is to reject.
fn precheck_header(header: &[u8], limits: &ReadLimits) -> Result<(), PackageError> {
    let mut w = Walk { b: header, pos: 0 };
    match w.u8()? {
        K_HEADER => {}
        K_ENCODED_HEADER => return container("encoded (compressed or encrypted) header is not supported"),
        x => return container(format!("expected a header (0x01), found 0x{x:02X}")),
    }
    let max = limits.entries as u64;
    let mut nid = w.u8()?;
    match nid {
        K_ARCHIVE_PROPERTIES => return container("archive properties are not supported"),
        K_ADDITIONAL_STREAMS_INFO => return container("additional streams are not supported"),
        _ => {}
    }
    if nid == K_MAIN_STREAMS_INFO {
        // Pack info: the packed streams start right after the signature header.
        w.expect(K_PACK_INFO, "pack info")?;
        if w.num()? != 0 {
            return container("packed streams do not start after the signature header");
        }
        let n_pack = w.count(max, "pack stream count")?;
        w.expect(K_SIZE, "pack sizes")?;
        for _ in 0..n_pack {
            w.num()?;
        }
        nid = w.u8()?;
        if nid == K_CRC {
            w.digests(n_pack)?;
            nid = w.u8()?;
        }
        if nid != K_END {
            return container("bad pack info");
        }
        // Unpack info: one folder per packed stream, one simple coder per folder.
        w.expect(K_UNPACK_INFO, "unpack info")?;
        w.expect(K_FOLDER, "folders")?;
        let n_folders = w.count(max, "folder count")?;
        if n_folders != n_pack {
            return container("folder count differs from pack stream count");
        }
        if w.u8()? != 0 {
            return container("external folders are not supported");
        }
        for _ in 0..n_folders {
            if w.num()? != 1 {
                return container("a folder must have exactly one coder");
            }
            let flags = w.u8()?;
            if flags & 0x10 != 0 {
                return container("complex coders are not supported");
            }
            if flags & 0xC0 != 0 {
                return container("alternative coder methods are not supported");
            }
            let id = w.bytes((flags & 0x0F) as u64)?;
            let props = if flags & 0x20 != 0 {
                let n = w.count(16, "coder property size")?;
                w.bytes(n)?
            } else {
                &[]
            };
            check_coder(id, props, limits)?;
        }
        w.expect(K_CODERS_UNPACK_SIZE, "unpack sizes")?;
        for _ in 0..n_folders {
            w.num()?;
        }
        nid = w.u8()?;
        let mut folder_crc = vec![false; n_folders as usize];
        if nid == K_CRC {
            folder_crc = w.digests(n_folders)?;
            nid = w.u8()?;
        }
        if nid != K_END {
            return container("bad unpack info");
        }
        // Substreams: exactly one per folder (non-solid).
        nid = w.u8()?;
        if nid == K_SUB_STREAMS_INFO {
            nid = w.u8()?;
            if nid == K_NUM_UNPACK_STREAM {
                for _ in 0..n_folders {
                    if w.num()? != 1 {
                        return container("a folder must hold exactly one entry (solid archives are not supported)");
                    }
                }
                nid = w.u8()?;
            }
            if nid == K_SIZE {
                return container("substream sizes are not supported (solid archive)");
            }
            if nid == K_CRC {
                let missing = folder_crc.iter().filter(|d| !**d).count() as u64;
                w.digests(missing)?;
                nid = w.u8()?;
            }
            if nid != K_END {
                return container("bad substreams info");
            }
            nid = w.u8()?;
        }
        if nid != K_END {
            return container("bad streams info");
        }
        nid = w.u8()?;
    }
    if nid != K_FILES_INFO {
        return container("header has no files info");
    }
    w.count(max, "entry count")?;
    Ok(())
}

fn too_large(what: &str, size: u64, limit: u64) -> PackageError {
    PackageError::TooLarge { what: what.into(), size, limit }
}

/// Whether a `sevenz-rust2` error is (or wraps) a CRC mismatch.
fn is_crc_error(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if let Some(sevenz_rust2::Error::ChecksumVerificationFailed) = err.downcast_ref::<sevenz_rust2::Error>() {
            return true;
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>()
            && let Some(inner) = io.get_ref()
            && is_crc_error(inner)
        {
            return true;
        }
        cur = match err.downcast_ref::<sevenz_rust2::Error>() {
            Some(sevenz_rust2::Error::Io(io, _)) | Some(sevenz_rust2::Error::MaybeBadPassword(io)) => {
                Some(io as &(dyn std::error::Error + 'static))
            }
            _ => err.source(),
        };
    }
    false
}

/// The checks [`read`] makes before `sevenz-rust2` sees the bytes (file size, signature
/// header, header bounds and CRCs, the header walk). Exposed for the never-panics test,
/// which counts how many inputs get past this stage.
#[doc(hidden)]
pub fn precheck(bytes: &[u8], limits: &ReadLimits) -> Result<(), PackageError> {
    signature_and_header(bytes, &limits.clamped()).map(|_| ())
}

/// Returns `next_header_offset` (the size of the packed streams).
fn signature_and_header(bytes: &[u8], limits: &ReadLimits) -> Result<u64, PackageError> {
    let flen = bytes.len() as u64;
    if flen > limits.file_size {
        return Err(too_large("package", flen, limits.file_size));
    }

    // Signature header. Checking its CRC here keeps sevenz-rust2 off its "damaged start
    // header" path (a backwards scan for a header).
    let Some(sig) = bytes.get(..SIGNATURE_HEADER_SIZE as usize) else {
        return container("file is too short for a 7z archive");
    };
    if sig[..6] != SIGNATURE {
        return container("not a 7z archive (bad signature)");
    }
    if sig[6] != 0 {
        return container(format!("unsupported 7z version {}.{}", sig[6], sig[7]));
    }
    let u32_at = |i: usize| u32::from_le_bytes([sig[i], sig[i + 1], sig[i + 2], sig[i + 3]]);
    let u64_at = |i: usize| u64::from_le_bytes(sig[i..i + 8].try_into().unwrap_or([0; 8]));
    if crc32fast::hash(&sig[12..32]) != u32_at(8) {
        return container("start header CRC mismatch");
    }
    let (next_off, next_size, next_crc) = (u64_at(12), u64_at(20), u32_at(28));
    if next_size > limits.header_size {
        return Err(too_large("7z header", next_size, limits.header_size));
    }
    // The header must end exactly at the end of the file (no trailing data).
    let header_start = SIGNATURE_HEADER_SIZE.checked_add(next_off);
    if header_start.and_then(|s| s.checked_add(next_size)) != Some(flen) || next_size == 0 {
        return container("header does not end at the end of the file");
    }
    let header_start = (SIGNATURE_HEADER_SIZE + next_off) as usize;
    let header = &bytes[header_start..];
    if crc32fast::hash(header) != next_crc {
        return container("header CRC mismatch");
    }
    precheck_header(header, limits)?;
    Ok(next_off)
}

/// Read a 7z archive in the profile of section 1. Entries come back in header order,
/// including ones the package format ignores; directory entries are skipped. Every name
/// is validated, and every entry is decoded and CRC-checked.
pub fn read(bytes: &[u8], limits: &ReadLimits) -> Result<Vec<Entry>, PackageError> {
    let limits = limits.clamped();
    let next_off = signature_and_header(bytes, &limits)?;

    // Parse with sevenz-rust2. The memory limit (KiB) covers the dictionary check it makes
    // itself; the header is already bounded above.
    let mem_kb = (limits.dictionary_size.max(limits.header_size) / 1024 + LZMA2_OVERHEAD_KIB) as usize;
    let password = Password::empty();
    let mut source = Cursor::new(bytes);
    let archive = Archive::read_with_memory_limit(&mut source, &password, mem_kb)
        .map_err(|e| PackageError::Container(e.to_string()))?;

    // The parsed metadata is what the decoder uses: check the profile again on it.
    if archive.is_solid {
        return container("solid archives are not supported");
    }
    if archive.pack_pos() != 0 {
        return container("packed streams do not start after the signature header");
    }
    if archive.pack_sizes().len() != archive.blocks.len() {
        return container("folder count differs from pack stream count");
    }
    let mut packed: u64 = 0;
    for &s in archive.pack_sizes() {
        packed = packed.saturating_add(s);
    }
    if packed != next_off {
        return container("packed streams do not fill the space before the header");
    }
    for block in &archive.blocks {
        let [coder] = block.coders.as_slice() else {
            return container("a folder must have exactly one coder");
        };
        check_coder(coder.encoder_method_id(), coder.properties(), &limits)?;
    }
    if archive.files.len() as u64 > limits.entries as u64 {
        return Err(too_large("entry count", archive.files.len() as u64, limits.entries as u64));
    }
    let mut used_blocks = vec![false; archive.blocks.len()];
    let mut seen: HashSet<String> = HashSet::new();
    let mut total: u64 = 0;
    for (i, f) in archive.files.iter().enumerate() {
        let name = f.name.as_str();
        let bad = |reason: String| PackageError::BadEntryName { name: name.to_string(), reason };
        let checked = name.strip_suffix('/').filter(|_| f.is_directory).unwrap_or(name);
        validate_entry_name(checked, &limits).map_err(bad)?;
        if !seen.insert(checked.to_ascii_lowercase()) {
            return Err(bad("duplicate name (ignoring ASCII case)".into()));
        }
        if f.is_anti_item {
            return container(format!("`{name}` is an anti-item"));
        }
        let block = archive.stream_map.file_block_index.get(i).copied().flatten();
        if !f.has_stream {
            if block.is_some() {
                return container(format!("`{name}` has no data but belongs to a folder"));
            }
            continue;
        }
        match block {
            Some(b) if b < used_blocks.len() && !used_blocks[b] => used_blocks[b] = true,
            _ => return container(format!("`{name}` does not have a folder of its own")),
        }
        if !f.has_crc {
            return container(format!("`{name}` has no CRC"));
        }
        let limit = match name {
            "manifest.json" => limits.manifest_size,
            "module.efvm" => limits.module_size,
            _ => limits.asset_size,
        };
        if f.size > limit {
            return Err(too_large(name, f.size, limit));
        }
        total = total.saturating_add(f.size);
        if total > limits.total_size {
            return Err(too_large("all entries", total, limits.total_size));
        }
    }
    if used_blocks.iter().any(|u| !u) {
        return container("a folder holds no entry");
    }

    // Decode each entry from its own folder, never reading more than its declared size
    // (already within its limit) plus one byte.
    let mut out = Vec::new();
    for (i, f) in archive.files.iter().enumerate() {
        if f.is_directory {
            continue;
        }
        let Some(block) = archive.stream_map.file_block_index[i] else {
            out.push(Entry { name: f.name.clone(), data: Vec::new() });
            continue;
        };
        let mut result: Option<Result<Vec<u8>, PackageError>> = None;
        let decoder = BlockDecoder::new_with_memory_limit(1, mem_kb, block, &archive, &password, &mut source);
        let status = decoder.for_each_entries(&mut |entry, reader| {
            let name = entry.name.as_str();
            let mut data = Vec::new();
            if data.try_reserve_exact(entry.size as usize).is_err() {
                result = Some(container(format!("`{name}`: cannot allocate {} bytes", entry.size)));
                return Err(std::io::Error::other("allocation failed").into());
            }
            match reader.take(entry.size.saturating_add(1)).read_to_end(&mut data) {
                Ok(_) if data.len() as u64 == entry.size => {
                    result = Some(Ok(data));
                    Ok(true)
                }
                Ok(_) => {
                    result = Some(container(format!(
                        "`{name}`: decoded size differs from declared {}",
                        entry.size
                    )));
                    Err(std::io::Error::other("size mismatch").into())
                }
                Err(e) => {
                    result = Some(if is_crc_error(&e) {
                        Err(PackageError::Crc { name: name.to_string() })
                    } else {
                        container(format!("`{name}`: {e}"))
                    });
                    Err(e.into())
                }
            }
        });
        let name = &f.name;
        match (status, result) {
            (_, Some(Err(e))) => return Err(e),
            (Ok(true), Some(Ok(data))) => out.push(Entry { name: name.clone(), data }),
            (Err(e), _) if is_crc_error(&e) => return Err(PackageError::Crc { name: name.clone() }),
            (Err(e), _) => return container(format!("`{name}`: {e}")),
            _ => return container(format!("`{name}`: its folder did not decode")),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Writer

#[cfg(feature = "write")]
fn put_num(v: &mut Vec<u8>, mut x: u64) {
    // 7z NUMBER: the count of leading one bits in the first byte gives the extra bytes.
    let mut first = 0u8;
    let mut mask = 0x80u8;
    let mut n = 0;
    while n < 8 {
        if x < (1u64 << (7 * (n + 1))) {
            first |= (x >> (8 * n)) as u8;
            break;
        }
        first |= mask;
        mask >>= 1;
        n += 1;
    }
    v.push(first);
    for _ in 0..n {
        v.push(x as u8);
        x >>= 8;
    }
}

#[cfg(feature = "write")]
fn put_bits(v: &mut Vec<u8>, bits: &[bool]) {
    for chunk in bits.chunks(8) {
        let mut b = 0u8;
        for (i, &set) in chunk.iter().enumerate() {
            if set {
                b |= 0x80 >> i;
            }
        }
        v.push(b);
    }
}

/// The smallest LZMA2 dictionary property whose size holds `len` bytes, at most 22
/// (8 MiB, the dictionary of preset 6).
#[cfg(feature = "write")]
fn lzma2_prop_for(len: usize) -> u8 {
    (0..=22u8).find(|&p| lzma2_dict_size(p).is_some_and(|d| d >= len as u64)).unwrap_or(22)
}

/// LZMA2 at preset 6 with the dictionary fitted to the data. `None` when not smaller.
#[cfg(feature = "write")]
fn lzma2(data: &[u8]) -> Option<(u8, Vec<u8>)> {
    use lzma_rust2::{Lzma2Options, Lzma2Writer};
    use std::io::Write;
    let prop = lzma2_prop_for(data.len());
    let mut options = Lzma2Options::with_preset(6);
    options.lzma_options.dict_size = lzma2_dict_size(prop)? as u32;
    let mut w = Lzma2Writer::new(Vec::new(), options);
    w.write_all(data).ok()?;
    let packed = w.finish().ok()?;
    (packed.len() < data.len()).then_some((prop, packed))
}

/// Canonical writer: entries are stored in the given order (the caller passes them in
/// canonical order). One folder per non-empty entry with one coder, LZMA2 (preset 6,
/// dictionary fitted to the entry, at most 8 MiB) unless COPY is not larger; empty entries
/// are empty files without a stream; a plain header with sizes, CRCs and names only (no
/// times, no attributes, no directory entries).
///
/// Callers keep names valid and within the limits (the reader's limits are far smaller
/// than anything the format cannot express).
#[cfg(feature = "write")]
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![0u8; SIGNATURE_HEADER_SIZE as usize];
    // (coder id, properties, packed size, unpack size, crc) per non-empty entry.
    let mut folders: Vec<(&[u8], Vec<u8>, u64, u64, u32)> = Vec::new();
    for &(_, data) in entries {
        if data.is_empty() {
            continue;
        }
        let crc = crc32fast::hash(data);
        match lzma2(data) {
            Some((prop, packed)) => {
                out.extend_from_slice(&packed);
                folders.push((ID_LZMA2, vec![prop], packed.len() as u64, data.len() as u64, crc));
            }
            None => {
                out.extend_from_slice(data);
                folders.push((ID_COPY, Vec::new(), data.len() as u64, data.len() as u64, crc));
            }
        }
    }
    let next_off = out.len() as u64 - SIGNATURE_HEADER_SIZE;

    let mut h = vec![K_HEADER];
    if !folders.is_empty() {
        h.push(K_MAIN_STREAMS_INFO);
        h.push(K_PACK_INFO);
        put_num(&mut h, 0);
        put_num(&mut h, folders.len() as u64);
        h.push(K_SIZE);
        for f in &folders {
            put_num(&mut h, f.2);
        }
        h.push(K_END);
        h.push(K_UNPACK_INFO);
        h.push(K_FOLDER);
        put_num(&mut h, folders.len() as u64);
        h.push(0); // not external
        for (id, props, ..) in &folders {
            put_num(&mut h, 1); // one coder
            let attrs = if props.is_empty() { 0 } else { 0x20 };
            h.push(id.len() as u8 | attrs);
            h.extend_from_slice(id);
            if !props.is_empty() {
                put_num(&mut h, props.len() as u64);
                h.extend_from_slice(props);
            }
        }
        h.push(K_CODERS_UNPACK_SIZE);
        for f in &folders {
            put_num(&mut h, f.3);
        }
        h.push(K_END);
        h.push(K_SUB_STREAMS_INFO);
        h.push(K_CRC);
        h.push(1); // all defined
        for f in &folders {
            h.extend_from_slice(&f.4.to_le_bytes());
        }
        h.push(K_END);
        h.push(K_END);
    }
    h.push(K_FILES_INFO);
    put_num(&mut h, entries.len() as u64);
    let empty: Vec<bool> = entries.iter().map(|(_, d)| d.is_empty()).collect();
    let n_empty = empty.iter().filter(|e| **e).count();
    if n_empty > 0 {
        let mut bits = Vec::new();
        put_bits(&mut bits, &empty);
        h.push(K_EMPTY_STREAM);
        put_num(&mut h, bits.len() as u64);
        h.extend_from_slice(&bits);
        // Every empty entry is a file, not a directory.
        let mut bits = Vec::new();
        put_bits(&mut bits, &vec![true; n_empty]);
        h.push(K_EMPTY_FILE);
        put_num(&mut h, bits.len() as u64);
        h.extend_from_slice(&bits);
    }
    let mut names = vec![0u8]; // not external
    for (name, _) in entries {
        for u in name.encode_utf16() {
            names.extend_from_slice(&u.to_le_bytes());
        }
        names.extend_from_slice(&[0, 0]);
    }
    h.push(K_NAME);
    put_num(&mut h, names.len() as u64);
    h.extend_from_slice(&names);
    h.push(K_END);
    h.push(K_END);

    out.extend_from_slice(&h);
    let mut start = [0u8; 20];
    start[0..8].copy_from_slice(&next_off.to_le_bytes());
    start[8..16].copy_from_slice(&(h.len() as u64).to_le_bytes());
    start[16..20].copy_from_slice(&crc32fast::hash(&h).to_le_bytes());
    out[0..6].copy_from_slice(&SIGNATURE);
    out[6] = 0;
    out[7] = 4;
    out[8..12].copy_from_slice(&crc32fast::hash(&start).to_le_bytes());
    out[12..32].copy_from_slice(&start);
    out
}
