#![no_main]
//! Feeds `build_mod` and `assemble_program` arbitrary mod folders. The input is split at 0xFF
//! bytes into `mod.toml`, `src/main.efa`, a file the entry may `use`, and an asset.

use efa::asm::{assemble_program, AsmOptions, MemSource};
use efa::mod_folder::{asm_options, build_mod, BuildLimits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut parts = data.splitn(4, |b| *b == 0xFF);
    let mut next = || parts.next().unwrap_or(&[]).to_vec();
    let (toml, main, used, asset) = (next(), next(), next(), next());
    let files = MemSource::new()
        .with_bytes("mod.toml", &toml)
        .with_bytes("src/main.efa", &main)
        .with_bytes("src/lib.efa", &used)
        .with_bytes("assets/a.bin", &asset);
    let limits = BuildLimits::default();
    let _ = build_mod(&files, None, &["a.bin".to_string()], &limits);

    // the same entry with a header that pulls the used file in, under tight nesting / use limits
    let with_use = [b"use \"lib.efa\"\n".as_slice(), &main].concat();
    let files = files.with_bytes("src/main.efa", &with_use);
    let tight = BuildLimits { max_use_depth: 4, max_nesting: 8, ..limits };
    let _ = assemble_program("src/main.efa", &files, &asm_options(None, &tight));
    let _ = assemble_program("src/main.efa", &files, &AsmOptions::default());
});
