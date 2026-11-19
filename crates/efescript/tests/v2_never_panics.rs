//! Deterministic mutation fuzz: no input may panic the assembler, decoder or disassembler.

use std::panic::{catch_unwind, AssertUnwindSafe};

use efescript::asm::assemble;
use efescript::disasm::disassemble;
use efevm::Module;

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
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const SPECIAL: &[u8] = b"{}()[],:;\"'-.\n0r";

fn mutate(src: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = src.to_vec();
    let rounds = 1 + rng.below(3);
    for _ in 0..rounds {
        let n = v.len();
        match rng.below(9) {
            0 if n > 0 => {
                v.remove(rng.below(n));
            }
            1 => {
                let at = rng.below(n + 1);
                v.insert(at, SPECIAL[rng.below(SPECIAL.len())]);
            }
            2 if n > 0 => {
                let i = rng.below(n);
                let c = v[i];
                v.insert(i, c);
            }
            3 if n > 1 => {
                let (a, b) = (rng.below(n), rng.below(n));
                v.swap(a, b);
            }
            4 => {
                v.truncate(rng.below(n + 1));
            }
            5 if n > 0 => {
                let i = rng.below(n);
                v[i] = SPECIAL[rng.below(SPECIAL.len())];
            }
            6 if n > 0 => {
                let i = rng.below(n);
                v[i] = rng.next() as u8;
            }
            7 => {
                // duplicate a line
                let mut lines: Vec<Vec<u8>> = v.split(|&b| b == b'\n').map(|l| l.to_vec()).collect();
                let i = rng.below(lines.len());
                let l = lines[i].clone();
                lines.insert(i, l);
                v = lines.join(&b'\n');
            }
            _ => {
                // swap two lines
                let mut lines: Vec<Vec<u8>> = v.split(|&b| b == b'\n').map(|l| l.to_vec()).collect();
                let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
                lines.swap(a, b);
                v = lines.join(&b'\n');
            }
        }
    }
    v
}

fn section15() -> String {
    let doc = include_str!("../docs/efa-syntax.md");
    let at = doc.find("## 15.").expect("section 15");
    let rest = &doc[at..];
    let start = rest.find("```\n").expect("fence") + 4;
    let end = rest[start..].find("\n```").expect("closing fence");
    rest[start..start + end].to_string() + "\n"
}

fn sources() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).expect("examples dir") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|s| s.to_str()) == Some("efa") {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            v.push((name, std::fs::read_to_string(&p).expect("read")));
        }
    }
    v.sort();
    v.push(("section15".to_string(), section15()));
    v
}

#[test]
fn section15_example_assembles() {
    assemble(&section15(), "tour.efa").unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn mutated_sources_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut count = 0usize;
    for (name, src) in sources() {
        for i in 0..300 {
            let input = if i == 0 { src.clone().into_bytes() } else { mutate(src.as_bytes(), &mut rng) };
            let text = String::from_utf8_lossy(&input).into_owned();
            let r = catch_unwind(AssertUnwindSafe(|| {
                // Assembled modules (rarely, when the mutation is harmless) must also disassemble.
                if let Ok(m) = assemble(&text, "fuzz.efa") {
                    let _ = disassemble(&m);
                }
            }));
            count += 1;
            assert!(r.is_ok(), "panic on mutation {i} of {name}; input:\n{text:?}");
        }
    }
    assert!(count >= 2100);
}

#[test]
fn odd_inputs_never_panic() {
    let odd: &[&str] = &[
        "", "\n", "\r\n\r\n", "{", "}", "fn", "fn (", "fn f(", "fn f() {", "export", "import", "import std.",
        "memory", "memory 99999999999999999999", "const", "const X =", "global", "table [", "table [f,",
        "x db", "x db \"", "x db '", "x db '\\", "x resb -1", "x resb 99999999999", "\"", "'", ";", "\t\t\t",
        "fn f() -> {\n}", "fn f(a: i64 -> i64 {\nret\n}", "fn f() {\nl:\n}", "fn f() {\n:\n}",
        "fn f() {\nbr_table r0, [", "fn f() {\ncall_indirect (", "fn f() {\ncall f(", "fn f() {\nret (",
        "\u{feff}fn f() {\nret\n}", "fn f\u{0}() {\nret\n}", "fn é() {\nret\n}", "x db \"\u{1F600}\"",
    ];
    for s in odd {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let _ = assemble(s, "odd.efa");
        }));
        assert!(r.is_ok(), "panic on {s:?}");
    }
}

#[test]
fn random_and_mutated_binaries_never_panic() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let mut decoded = 0usize;
    // Random bytes, some with a valid magic.
    for i in 0..2000 {
        let len = rng.below(200);
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if i % 2 == 0 && bytes.len() >= 4 {
            bytes[..4].copy_from_slice(b"EFVM");
        }
        decode_and_dis(&bytes, &mut decoded);
    }
    // Mutations of real encoded modules reach much deeper into the decoder.
    for (name, src) in sources() {
        let Ok(m) = assemble(&src, &name) else { continue };
        let good = m.encode();
        for _ in 0..300 {
            let mut bytes = good.clone();
            for _ in 0..1 + rng.below(4) {
                let n = bytes.len();
                if n == 0 {
                    break;
                }
                match rng.below(3) {
                    0 => {
                        let i = rng.below(n);
                        bytes[i] = rng.next() as u8;
                    }
                    1 => {
                        let i = rng.below(n);
                        bytes[i] ^= 1 << rng.below(8);
                    }
                    _ => bytes.truncate(rng.below(n)),
                }
            }
            decode_and_dis(&bytes, &mut decoded);
        }
    }
    eprintln!("{decoded} mutated binaries decoded");
}

fn decode_and_dis(bytes: &[u8], decoded: &mut usize) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(m) = Module::decode(bytes) {
            let _ = disassemble(&m);
            true
        } else {
            false
        }
    }));
    match r {
        Ok(d) => *decoded += usize::from(d),
        Err(_) => panic!("panic on binary input {bytes:?}"),
    }
}
