//! The 7z container profile (MOD_PACKAGE.md sections 1, 2, 6, 8). Needs the `write`
//! feature (`[[test]] required-features`).

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use efemod::container::{self, Entry};
use efemod::{PackageError, ReadLimits};

fn lim() -> ReadLimits {
    ReadLimits::default()
}

fn ok(bytes: &[u8]) -> Vec<Entry> {
    container::read(bytes, &lim()).unwrap_or_else(|e| panic!("expected Ok, got {e}"))
}

fn container_err(bytes: &[u8]) -> String {
    match container::read(bytes, &lim()) {
        Err(PackageError::Container(m)) => m,
        other => panic!("expected a container error, got {other:?}"),
    }
}

fn too_large(bytes: &[u8], l: &ReadLimits) -> (String, u64, u64) {
    match container::read(bytes, l) {
        Err(PackageError::TooLarge { what, size, limit }) => (what, size, limit),
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

/// Compressible text (LZMA2) and incompressible noise (COPY).
fn text(n: usize) -> Vec<u8> {
    b"the quick brown fox jumps over the lazy dog. ".iter().copied().cycle().take(n).collect()
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    (0..n).map(|_| r.next() as u8).collect()
}

fn sample() -> Vec<(String, Vec<u8>)> {
    vec![
        ("manifest.json".into(), b"{\"format\":1}".to_vec()),
        ("module.efvm".into(), text(3000)),
        ("assets/a.bin".into(), noise(500, 7)),
        ("assets/empty.txt".into(), Vec::new()),
        ("assets/z/\u{e7}i\u{11f}.txt".into(), text(40)),
    ]
}

fn write(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let v: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    container::write(&v)
}

// ---- archive surgery --------------------------------------------------------------

fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

/// Start of the header (the end of the packed streams).
fn header_start(b: &[u8]) -> usize {
    32 + u64_at(b, 12) as usize
}

/// Recompute the header CRC, size and the start header CRC after editing the header.
fn fix_crcs(b: &mut [u8]) {
    let hs = header_start(b);
    let size = (b.len() - hs) as u64;
    b[20..28].copy_from_slice(&size.to_le_bytes());
    let hcrc = crc32fast::hash(&b[hs..]);
    b[28..32].copy_from_slice(&hcrc.to_le_bytes());
    let scrc = crc32fast::hash(&b[12..32]);
    b[8..12].copy_from_slice(&scrc.to_le_bytes());
}

/// Position of `pat` inside the header.
fn find_in_header(b: &[u8], pat: &[u8]) -> usize {
    let hs = header_start(b);
    hs + b[hs..].windows(pat.len()).position(|w| w == pat).expect("pattern in header")
}

/// A one-entry archive with LZMA2 (text) and its LZMA2 property byte position.
fn lzma2_archive() -> (Vec<u8>, usize) {
    let b = container::write(&[("a.txt", &text(5000))]);
    // coder: flags 0x21 (1-byte id, has properties), id 0x21, 1 property byte.
    let p = find_in_header(&b, &[0x01, 0x21, 0x21, 0x01]) + 4;
    (b, p)
}

/// A one-entry archive with COPY (noise) and the position of its unpack size byte.
fn copy_archive() -> (Vec<u8>, usize) {
    let b = container::write(&[("a.bin", &noise(100, 3))]);
    // folder: 1 coder, flags 0x01, id 0x00; then kCodersUnpackSize, size (100 < 128: 1 byte).
    let p = find_in_header(&b, &[0x01, 0x01, 0x00, 0x0C]) + 4;
    assert_eq!(b[p], 100);
    (b, p)
}

// ---- round trip and the canonical writer -------------------------------------------

#[test]
fn round_trip_and_determinism() {
    let s = sample();
    let a = write(&s);
    assert_eq!(a, write(&s), "same inputs, same bytes");
    let got = ok(&a);
    assert_eq!(got.len(), s.len());
    for (e, (n, d)) in got.iter().zip(&s) {
        assert_eq!(&e.name, n);
        assert_eq!(&e.data, d);
    }
    // Different content gives different bytes.
    let mut s2 = s.clone();
    s2[1].1[0] ^= 1;
    assert_ne!(a, write(&s2));
}

#[test]
fn empty_archive_and_empty_files() {
    assert!(ok(&container::write(&[])).is_empty());
    let b = container::write(&[("a", b""), ("b", b"")]);
    let e = ok(&b);
    assert_eq!(e, vec![Entry { name: "a".into(), data: vec![] }, Entry { name: "b".into(), data: vec![] }]);
}

#[test]
fn writer_uses_lzma2_or_copy_one_folder_each() {
    let b = write(&sample());
    let mut src = Cursor::new(&b[..]);
    let ar = sevenz_rust2::Archive::read(&mut src, &sevenz_rust2::Password::empty()).unwrap();
    assert!(!ar.is_solid);
    // Four non-empty entries, four folders.
    assert_eq!(ar.blocks.len(), 4);
    let ids: Vec<Vec<u8>> = ar.blocks.iter().map(|bl| bl.coders[0].encoder_method_id().to_vec()).collect();
    // Short entries do not compress: COPY; the 3000-byte text: LZMA2.
    assert_eq!(ids, vec![vec![0x00], vec![0x21], vec![0x00], vec![0x00]]);
    for f in &ar.files {
        assert!(!f.has_creation_date && !f.has_last_modified_date && !f.has_access_date);
        assert!(!f.has_windows_attributes && !f.is_directory && !f.is_anti_item);
    }
    // The header is plain (not encoded) and ends the file.
    assert_eq!(b[header_start(&b)], 0x01);
}

#[test]
fn plain_sevenz_reader_reads_writer_output() {
    // Without our pre-checks: the writer agrees with sevenz-rust2, not just with itself.
    let s = sample();
    let b = write(&s);
    let mut r = sevenz_rust2::ArchiveReader::new(Cursor::new(&b[..]), sevenz_rust2::Password::empty()).unwrap();
    for (n, d) in &s {
        assert_eq!(&r.read_file(n).unwrap(), d, "{n}");
    }
}

#[test]
fn dictionary_fits_the_entry() {
    let (b, p) = lzma2_archive();
    // 5000 bytes: dictionary 6 KiB (property 1).
    assert_eq!(b[p], 1);
    let n = (8 << 20) + 1;
    let big = container::write(&[("a", &vec![0u8; n])]);
    let p = find_in_header(&big, &[0x01, 0x21, 0x21, 0x01]) + 4;
    assert_eq!(big[p], 22, "capped at 8 MiB");
    assert_eq!(ok(&big)[0].data.len(), n);
}

// ---- signature header, header bounds, encoded headers ------------------------------

#[test]
fn reject_short_and_bad_signature() {
    assert!(container_err(b"").contains("too short"));
    assert!(container_err(&[0u8; 31]).contains("too short"));
    let mut b = write(&sample());
    b[0] = b'8';
    assert!(container_err(&b).contains("signature"));
    let mut b = write(&sample());
    b[6] = 1;
    fix_crcs(&mut b);
    assert!(container_err(&b).contains("version"));
}

#[test]
fn zeroed_start_header_is_rejected_before_the_crate_scans() {
    // sevenz-rust2 would treat CRC 0 + zero start header as damaged and scan backwards
    // for a header (reader.rs try_to_locale_end_header); the CRC gate stops that.
    let mut b = write(&sample());
    for x in &mut b[8..32] {
        *x = 0;
    }
    assert!(container_err(&b).contains("start header CRC"));
    let mut b = write(&sample());
    b[9] ^= 1;
    assert!(container_err(&b).contains("start header CRC"));
}

#[test]
fn reject_header_crc_and_bounds() {
    let mut b = write(&sample());
    let last = b.len() - 1;
    b[last] ^= 0x40;
    assert!(container_err(&b).contains("header CRC"));
    // Trailing data after the header.
    let mut b = write(&sample());
    b.push(0);
    assert!(container_err(&b).contains("end of the file"));
    // Header offset past the end.
    let mut b = write(&sample());
    b[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    let c = crc32fast::hash(&b[12..32]);
    b[8..12].copy_from_slice(&c.to_le_bytes());
    assert!(container_err(&b).contains("end of the file"));
}

#[test]
fn header_size_limit() {
    let b = write(&sample());
    let size = (b.len() - header_start(&b)) as u64;
    let l = ReadLimits { header_size: size - 1, ..lim() };
    assert_eq!(too_large(&b, &l), ("7z header".into(), size, size - 1));
    let l = ReadLimits { header_size: size, ..lim() };
    assert!(container::read(&b, &l).is_ok());
}

#[test]
fn reject_encoded_header() {
    let mut b = write(&sample());
    let hs = header_start(&b);
    b[hs] = 0x17;
    fix_crcs(&mut b);
    assert!(container_err(&b).contains("encoded"));
}

#[test]
fn reject_archive_properties() {
    let mut b = write(&sample());
    let hs = header_start(&b);
    b.insert(hs + 1, 0x02);
    b.insert(hs + 2, 0x00);
    fix_crcs(&mut b);
    assert!(container_err(&b).contains("archive properties"));
}

// ---- coders and dictionary ----------------------------------------------------------

#[test]
fn lzma2_dictionary_property_boundaries() {
    let (b, p) = lzma2_archive();
    let with = |prop: u8| {
        let mut c = b.clone();
        c[p] = prop;
        fix_crcs(&mut c);
        c
    };
    // 28 = 64 MiB, the hard cap: accepted (the window grows with the output only).
    assert_eq!(ok(&with(28))[0].data, text(5000));
    // 29 = 96 MiB, 40 = 4 GiB - 1: too large.
    assert_eq!(too_large(&with(29), &lim()), ("LZMA2 dictionary".into(), 96 << 20, 64 << 20));
    assert_eq!(too_large(&with(40), &lim()).1, 0xFFFF_FFFF);
    assert!(container_err(&with(41)).contains("dictionary property"));
    assert!(container_err(&with(0xFF)).contains("dictionary property"));
    // A lowered limit applies to what the writer chose.
    let l = ReadLimits { dictionary_size: 4096, ..lim() };
    assert_eq!(too_large(&b, &l), ("LZMA2 dictionary".into(), 6144, 4096));
}

#[test]
fn other_coders_are_rejected() {
    let (b, p) = lzma2_archive();
    let id = p - 2;
    // One-byte ids: Delta (03), ARM64 (0A), RISC-V (0B), unknown (7F).
    for new in [0x03u8, 0x0A, 0x0B, 0x7F] {
        let mut c = b.clone();
        c[id] = new;
        fix_crcs(&mut c);
        assert!(container_err(&c).contains("not supported"), "{new:#x}");
    }
    // COPY with properties.
    let mut c = b.clone();
    c[id] = 0x00;
    fix_crcs(&mut c);
    assert!(container_err(&c).contains("COPY coder with properties"));
    // Complex coder flag, alternative methods flag.
    for flags in [0x31u8, 0xA1, 0x61] {
        let mut c = b.clone();
        c[id - 1] = flags;
        fix_crcs(&mut c);
        container_err(&c);
    }
    // Two coders in the folder.
    let mut c = b.clone();
    c[id - 2] = 2;
    fix_crcs(&mut c);
    assert!(container_err(&c).contains("exactly one coder"));
}

// ---- data integrity -----------------------------------------------------------------

#[test]
fn reject_crc_mismatch() {
    let (mut b, _) = copy_archive();
    b[40] ^= 1; // inside the stored data
    assert_eq!(container::read(&b, &lim()), Err(PackageError::Crc { name: "a.bin".into() }));
    // A stored CRC that does not match.
    let (mut b, _) = copy_archive();
    let p = find_in_header(&b, &[0x08, 0x0A, 0x01]) + 3;
    b[p] ^= 1;
    fix_crcs(&mut b);
    assert_eq!(container::read(&b, &lim()), Err(PackageError::Crc { name: "a.bin".into() }));
}

#[test]
fn corrupt_lzma2_data_is_an_error() {
    let (b, _) = lzma2_archive();
    let hs = header_start(&b);
    for i in 32..hs {
        let mut c = b.clone();
        c[i] ^= 0x55;
        assert!(container::read(&c, &lim()).is_err(), "flip at {i}");
    }
}

#[test]
fn reject_declared_size_lies() {
    // The unpack size says more or less than the stored data holds.
    for size in [99u8, 101, 0, 127] {
        let (mut b, p) = copy_archive();
        b[p] = size;
        fix_crcs(&mut b);
        assert!(container::read(&b, &lim()).is_err(), "size {size}");
    }
    // A huge declared size is caught by the limit before decoding.
    let (mut b, p) = copy_archive();
    b.splice(p..p + 1, [0xF0, 0, 0, 0, 0x40]); // 4-extra-byte NUMBER: 0x40_0000_0000
    fix_crcs(&mut b);
    let (what, size, limit) = too_large(&b, &lim());
    assert_eq!((what.as_str(), limit), ("a.bin", 32 << 20));
    assert!(size > limit);
    // Same for LZMA2.
    let (b, p) = lzma2_archive();
    let q = find_in_header(&b, &[0x0C]) + 1;
    assert!(q > p);
    let mut c = b.clone();
    c.splice(q..q + 2, [0xF0, 0, 0, 0, 0x40]);
    fix_crcs(&mut c);
    too_large(&c, &lim());
    let mut c = b.clone();
    c[q] ^= 1; // 5000 -> 5001 or 4999: the LZMA2 stream ends elsewhere
    fix_crcs(&mut c);
    assert!(container::read(&c, &lim()).is_err());
}

#[test]
fn reject_packed_streams_not_filling_the_space() {
    let (mut b, _) = copy_archive();
    // Pack size 100 -> 99: one byte between the streams and the header.
    let p = find_in_header(&b, &[0x06, 0x00, 0x01, 0x09]) + 4;
    b[p] = 99;
    fix_crcs(&mut b);
    container_err(&b);
    // Pack position not 0.
    let (mut b, _) = copy_archive();
    let p = find_in_header(&b, &[0x06, 0x00]) + 1;
    b[p] = 1;
    fix_crcs(&mut b);
    assert!(container_err(&b).contains("signature header"));
}

// ---- entries -------------------------------------------------------------------------

#[test]
fn reject_bad_names() {
    for n in ["../x", "a/../b", "/abs", "C:x", "a\\b", "a//b", "a/", "", "a\u{1}"] {
        let b = container::write(&[(n, b"x")]);
        assert!(matches!(container::read(&b, &lim()), Err(PackageError::BadEntryName { .. })), "{n:?}");
    }
    let long = "a".repeat(241);
    let b = container::write(&[(long.as_str(), b"x")]);
    assert!(matches!(container::read(&b, &lim()), Err(PackageError::BadEntryName { .. })));
    let deep = vec!["a"; 17].join("/");
    let b = container::write(&[(deep.as_str(), b"x")]);
    assert!(matches!(container::read(&b, &lim()), Err(PackageError::BadEntryName { .. })));
    // Boundaries are accepted.
    let ok_long = "a".repeat(240);
    let ok_deep = vec!["a"; 16].join("/");
    ok(&container::write(&[(ok_long.as_str(), b"x"), (ok_deep.as_str(), b"y")]));
}

#[test]
fn reject_duplicates_and_case_duplicates() {
    for (a, b) in [("x.txt", "x.txt"), ("Assets/A.png", "assets/a.png")] {
        let ar = container::write(&[(a, b"1"), (b, b"2")]);
        match container::read(&ar, &lim()) {
            Err(PackageError::BadEntryName { reason, .. }) => assert!(reason.contains("duplicate")),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn reject_unpaired_surrogate_name() {
    let mut b = container::write(&[("ab", b"x")]);
    let p = find_in_header(&b, &[b'a', 0, b'b', 0]);
    b[p..p + 2].copy_from_slice(&0xD800u16.to_le_bytes());
    fix_crcs(&mut b);
    container_err(&b);
}

#[test]
fn reject_anti_items() {
    // Insert kAnti (one bit for the one empty-stream entry) after kEmptyFile.
    let mut b = container::write(&[("a", b"")]);
    let p = find_in_header(&b, &[0x0F, 0x01, 0x80]) + 3;
    b.splice(p..p, [0x10, 0x01, 0x80]);
    fix_crcs(&mut b);
    assert!(container_err(&b).contains("anti-item"));
}

#[test]
fn directory_entries_are_ignored_but_validated() {
    // kEmptyFile bit cleared: the empty entry becomes a directory.
    let mk = |dir: &str| {
        let mut b = container::write(&[(dir, b""), ("assets/a", b"x")]);
        let p = find_in_header(&b, &[0x0F, 0x01, 0x80]) + 2;
        b[p] = 0;
        fix_crcs(&mut b);
        b
    };
    assert_eq!(ok(&mk("assets")), vec![Entry { name: "assets/a".into(), data: b"x".to_vec() }]);
    assert!(matches!(container::read(&mk("../d"), &lim()), Err(PackageError::BadEntryName { .. })));
    assert!(matches!(container::read(&mk("Assets/A"), &lim()), Err(PackageError::BadEntryName { .. })));
}

// ---- limits --------------------------------------------------------------------------

#[test]
fn limits() {
    let b = write(&sample());
    let n = b.len() as u64;
    assert_eq!(too_large(&b, &ReadLimits { file_size: n - 1, ..lim() }), ("package".into(), n, n - 1));
    assert_eq!(too_large(&b, &ReadLimits { entries: 4, ..lim() }).0, "entry count");
    assert!(container::read(&b, &ReadLimits { entries: 5, ..lim() }).is_ok());
    assert_eq!(too_large(&b, &ReadLimits { manifest_size: 5, ..lim() }).0, "manifest.json");
    assert_eq!(too_large(&b, &ReadLimits { module_size: 2999, ..lim() }).0, "module.efvm");
    assert_eq!(too_large(&b, &ReadLimits { asset_size: 499, ..lim() }).0, "assets/a.bin");
    assert_eq!(too_large(&b, &ReadLimits { total_size: 3000, ..lim() }).0, "all entries");
    let total = sample().iter().map(|(_, d)| d.len() as u64).sum::<u64>();
    assert!(container::read(&b, &ReadLimits { total_size: total, ..lim() }).is_ok());
    // Directory entries count too: 4096 + 1 entries.
    let many: Vec<(String, Vec<u8>)> = (0..4097).map(|i| (format!("e{i}"), Vec::new())).collect();
    assert_eq!(too_large(&write(&many), &lim()).0, "entry count");
}

#[test]
fn limits_are_clamped_to_hard_caps() {
    let big = ReadLimits {
        file_size: u64::MAX,
        entries: u32::MAX,
        header_size: u64::MAX,
        dictionary_size: u64::MAX,
        name_bytes: usize::MAX,
        name_segments: usize::MAX,
        manifest_size: u64::MAX,
        module_size: u64::MAX,
        asset_size: u64::MAX,
        total_size: u64::MAX,
    };
    assert_eq!(big.clamped(), ReadLimits::HARD);
    assert_eq!(ReadLimits::HARD.header_size, 4 << 20);
    assert_eq!(ReadLimits::HARD.dictionary_size, 64 << 20);
    // A 96 MiB dictionary stays rejected with raised limits.
    let (b, p) = lzma2_archive();
    let mut c = b.clone();
    c[p] = 29;
    fix_crcs(&mut c);
    assert_eq!(too_large(&c, &big).2, 64 << 20);
}

#[test]
fn huge_counts_are_rejected_before_allocation() {
    // A tiny header declaring 2^40 files, folders or pack streams.
    let huge = [0xF0, 0, 0, 0, 0x01];
    let mut b = container::write(&[("a", b"")]);
    let p = find_in_header(&b, &[0x05, 0x01]) + 1;
    b.splice(p..p + 1, huge);
    fix_crcs(&mut b);
    assert_eq!(too_large(&b, &lim()).0, "entry count");
    let (mut b, _) = copy_archive();
    let p = find_in_header(&b, &[0x06, 0x00, 0x01]) + 2;
    b.splice(p..p + 1, huge);
    fix_crcs(&mut b);
    assert_eq!(too_large(&b, &lim()).0, "pack stream count");
}

// ---- the 7z command-line tool -----------------------------------------------------------

fn seven_zip() -> Option<&'static str> {
    ["7z", "7zz", "7za"].into_iter().find(|t| Command::new(t).arg("i").output().is_ok_and(|o| o.status.success()))
}

fn unique_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("efemod-7z-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `7z a <switches> out.7z` over a small tree; returns the archive bytes.
fn cli_archive(tool: &str, tag: &str, switches: &[&str]) -> Vec<u8> {
    let d = unique_dir(tag);
    let src = d.join("src");
    std::fs::create_dir_all(src.join("assets/sub")).unwrap();
    std::fs::write(src.join("manifest.json"), b"{}").unwrap();
    std::fs::write(src.join("module.efvm"), text(4000)).unwrap();
    std::fs::write(src.join("assets/sub/n.bin"), noise(300, 1)).unwrap();
    std::fs::write(src.join("assets/e.txt"), b"").unwrap();
    let out = d.join("out.7z");
    let st = Command::new(tool)
        .current_dir(&src)
        .args(["a", "-t7z", "-bd", "-y"])
        .args(switches)
        .arg(&out)
        .args(["manifest.json", "module.efvm", "assets"])
        .output()
        .unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let b = std::fs::read(&out).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    b
}

#[test]
fn seven_zip_tests_writer_output() {
    let Some(tool) = seven_zip() else {
        eprintln!("7z not installed; skipped");
        return;
    };
    let d = unique_dir("t");
    let p = d.join("w.7z");
    std::fs::write(&p, write(&sample())).unwrap();
    let st = Command::new(tool).args(["t", "-bd"]).arg(&p).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stdout));
    let listing = Command::new(tool).args(["l", "-slt"]).arg(&p).output().unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(listing.contains("Solid = -"), "{listing}");
    assert!(listing.contains("Path = assets/z/\u{e7}i\u{11f}.txt"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn seven_zip_non_solid_plain_header_archives_read() {
    let Some(tool) = seven_zip() else {
        eprintln!("7z not installed; skipped");
        return;
    };
    for m in ["-m0=lzma2", "-m0=copy"] {
        let b = cli_archive(tool, "ok", &["-ms=off", "-mhc=off", m]);
        let e = ok(&b);
        let names: Vec<&str> = e.iter().map(|e| e.name.as_str()).collect();
        // Directory entries (assets, assets/sub) are left out; times and attributes ignored.
        for n in ["manifest.json", "module.efvm", "assets/sub/n.bin", "assets/e.txt"] {
            assert!(names.contains(&n), "{m}: {names:?}");
        }
        assert_eq!(names.len(), 4, "{m}: {names:?}");
        assert_eq!(e.iter().find(|e| e.name == "module.efvm").unwrap().data, text(4000));
    }
}

#[test]
fn seven_zip_other_archives_are_rejected() {
    let Some(tool) = seven_zip() else {
        eprintln!("7z not installed; skipped");
        return;
    };
    let cases: &[(&str, &[&str])] = &[
        ("default (solid, compressed header)", &[]),
        ("compressed header", &["-ms=off"]),
        ("solid", &["-ms=on", "-mhc=off"]),
        ("ppmd", &["-ms=off", "-mhc=off", "-m0=ppmd"]),
        ("bzip2", &["-ms=off", "-mhc=off", "-m0=bzip2"]),
        ("deflate", &["-ms=off", "-mhc=off", "-m0=deflate"]),
        ("lzma", &["-ms=off", "-mhc=off", "-m0=lzma"]),
        ("delta + lzma2", &["-ms=off", "-mhc=off", "-m0=delta:4", "-m1=lzma2"]),
        ("bcj + lzma2", &["-ms=off", "-mhc=off", "-mf=BCJ", "-m0=lzma2"]),
        ("aes", &["-ms=off", "-mhc=off", "-psecret", "-mhe=off"]),
        ("aes header", &["-ms=off", "-psecret", "-mhe=on"]),
    ];
    for (what, sw) in cases {
        let b = cli_archive(tool, "bad", sw);
        match container::read(&b, &lim()) {
            Err(PackageError::Container(_)) => {}
            other => panic!("{what}: expected a container error, got {other:?}"),
        }
    }
}

// ---- never panics ---------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate_range(r: &mut Rng, b: &mut Vec<u8>, lo: usize, hi: usize) {
    if hi <= lo {
        return;
    }
    for _ in 0..1 + r.below(4) {
        let hi = hi.min(b.len());
        if hi <= lo {
            return;
        }
        let i = lo + r.below(hi - lo);
        match r.below(6) {
            0 => b[i] ^= 1 << r.below(8),
            1 => b[i] = r.next() as u8,
            2 => b[i] = [0x00, 0x01, 0x7F, 0x80, 0xFF, 0x17, 0x21][r.below(7)],
            3 => b.insert(i, r.next() as u8),
            4 if b.len() > 33 => {
                b.remove(i);
            }
            _ => b[i] = b[i].wrapping_add(1),
        }
    }
}

#[derive(Default, Debug)]
struct Stages {
    inputs: u32,
    past_precheck: u32,
    crc_errors: u32,
    ok: u32,
}

fn run(stages: &mut Stages, b: &[u8]) {
    stages.inputs += 1;
    if container::precheck(b, &lim()).is_ok() {
        stages.past_precheck += 1;
    }
    match container::read(b, &lim()) {
        Ok(_) => stages.ok += 1,
        Err(PackageError::Crc { .. }) => stages.crc_errors += 1,
        Err(_) => {}
    }
}

#[test]
fn never_panics() {
    let small = container::write(&[("m", b"{}"), ("a", &text(300)), ("e", b""), ("n", &noise(40, 9))]);
    let seeds = [small.clone(), write(&sample()), lzma2_archive().0, copy_archive().0];
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut s = Stages::default();

    // Truncation at every length.
    for n in 0..small.len() {
        run(&mut s, &small[..n]);
    }
    for seed in &seeds {
        let hs = header_start(seed);
        for _ in 0..8000 {
            let mut b = seed.clone();
            match r.below(4) {
                // Anywhere, no CRC fixup (mostly stops at the CRC gates).
                0 => {
                    let len = b.len();
                    mutate_range(&mut r, &mut b, 0, len);
                }
                // Header only, CRCs fixed: reaches the header walk and sevenz-rust2.
                1 | 2 => {
                    let len = b.len();
                    mutate_range(&mut r, &mut b, hs, len);
                    fix_crcs(&mut b);
                }
                // Packed data only, header intact: reaches the decoders.
                _ => {
                    let mut data = b[32..hs].to_vec();
                    let len = data.len();
                    if len > 0 {
                        for _ in 0..1 + r.below(3) {
                            let i = r.below(len);
                            data[i] ^= 1 << r.below(8);
                        }
                    }
                    b.splice(32..hs, data);
                }
            }
            run(&mut s, &b);
        }
    }
    // Random bytes behind a valid signature header pointing at a random header.
    for _ in 0..5000 {
        let mut b = vec![0u8; 32];
        let body = r.below(64);
        let hlen = 1 + r.below(64);
        b.extend((0..body).map(|_| r.next() as u8));
        b.push(0x01);
        b.extend((1..hlen).map(|_| r.next() as u8));
        b[0..6].copy_from_slice(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]);
        b[7] = 4;
        b[12..20].copy_from_slice(&(body as u64).to_le_bytes());
        fix_crcs(&mut b);
        run(&mut s, &b);
        // And plain random bytes.
        let n = r.below(200);
        let junk: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        run(&mut s, &junk);
    }
    eprintln!("{s:?}");
    // The fuzzing reaches every stage, not just the CRC gates.
    assert!(s.past_precheck > 5000, "{s:?}");
    assert!(s.crc_errors > 200, "{s:?}");
    assert!(s.ok > 200, "{s:?}");
}
