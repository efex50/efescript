//! Deterministic mutation test: mangled mod folders must make `build_mod` return (Ok or Err),
//! never panic or hang. Own xorshift PRNG, no extra dependencies.

use efa::asm::MemSource;
use efa::mod_folder::{build_mod, BuildLimits};

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

const TOML: &str = "[mod]\nid = \"fz\"\nversion = \"1.2.3\"\nname = \"Fuzz\"\n\n[dependencies]\nother = \"^1\"\n\n[permissions]\nnamespaces = [\"engine.log\"]\nbudget = 1000\n\n[build]\nentry = \"src/main.efa\"\nassets = \"assets\"\n\n[meta]\nk = \"v\"\nn = [1, 2.5, true]\n[meta.t]\nx = 1\n";
const MAIN: &str = "use \"lib.efa\" as l\nexport struct Item, \"One stack.\" {\n    id: u32\n    count: i32\n}\nname db \"sword\"\nbuf resb 16\nglobal mut g: i32 = 0\nprovide give(n: i64) -> (r: i64) = give, \"Give.\"\nfn give(n: i64) -> i64 {\n    reg r\n    i64.add r, n, l.LIMIT\n    ret r\n}\nexport fn init() {\n    reg a, b\n    const.i64 a, 21\n    call give(a) -> b\n    ret\n}\n";
const LIB: &str = "pub const LIMIT = 100\n";

fn seed_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("mod.toml", TOML.as_bytes().to_vec()),
        ("src/main.efa", MAIN.as_bytes().to_vec()),
        ("src/lib.efa", LIB.as_bytes().to_vec()),
        ("assets/a.bin", vec![1, 2, 3, 4]),
    ]
}

const TOKENS: &[&str] = &[
    "(", ")", "{", "}", "[", "]", "\"", "\\", ",", ";", "\n", "use \"lib.efa\"\n", "use \"../x.efa\"\n", "use \"main.efa\"\n",
    "resb 4294967295\n", "resb 99999999999\n", "const.i64 a, 99999999999999999999\n", "-9223372036854775808", "1e999",
    "export fn f() {\n", "provide x() = x, \"\"\n", "import engine.log.info\n", "[build]\nentry = \"../x\"\n",
    "name db \"", "global mut z: i64 = 0\n", "\u{0}", "\u{feff}", "é", "0x", "0b2", "'", "''",
];

fn mutate(rng: &mut Rng, data: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(4) {
        match rng.below(8) {
            0 if !data.is_empty() => {
                let i = rng.below(data.len());
                data[i] ^= 1 << rng.below(8);
            }
            1 if !data.is_empty() => {
                let i = rng.below(data.len());
                data[i] = rng.next() as u8;
            }
            2 if !data.is_empty() => data.truncate(rng.below(data.len())),
            3 if !data.is_empty() => {
                let (a, b) = (rng.below(data.len()), rng.below(data.len()));
                let (a, b) = (a.min(b), a.max(b));
                data.drain(a..b);
            }
            4 => {
                let t = TOKENS[rng.below(TOKENS.len())].as_bytes();
                let i = rng.below(data.len() + 1);
                data.splice(i..i, t.iter().copied());
            }
            5 if !data.is_empty() => {
                let (a, b) = (rng.below(data.len()), rng.below(data.len()));
                let (a, b) = (a.min(b), a.max(b));
                let chunk = data[a..b].to_vec();
                let i = rng.below(data.len() + 1);
                for _ in 0..1 + rng.below(8) {
                    data.splice(i..i, chunk.iter().copied());
                }
            }
            6 => {
                // deep nesting
                let open = ["(", "{", "[", "\n{\n"][rng.below(4)];
                let n = 1 + rng.below(600);
                let i = rng.below(data.len() + 1);
                data.splice(i..i, open.repeat(n).into_bytes());
            }
            _ => {
                // a long run of digits (huge counts)
                let n = 1 + rng.below(40);
                let i = rng.below(data.len() + 1);
                data.splice(i..i, "9".repeat(n).into_bytes());
            }
        }
    }
}

fn run(files: &[(String, Vec<u8>)], assets: &[String]) {
    let mut src = MemSource::new();
    for (p, b) in files {
        src = src.with_bytes(p, b);
    }
    let _ = build_mod(&src, None, assets, &BuildLimits::default());
}

#[test]
fn the_seed_folder_builds() {
    let files: Vec<(String, Vec<u8>)> = seed_files().into_iter().map(|(p, b)| (p.to_string(), b)).collect();
    let mut src = MemSource::new();
    for (p, b) in &files {
        src = src.with_bytes(p, b);
    }
    build_mod(&src, None, &["a.bin".to_string()], &BuildLimits::default()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn mutated_folders_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        let mut files: Vec<(String, Vec<u8>)> = seed_files().into_iter().map(|(p, b)| (p.to_string(), b)).collect();
        for _ in 0..1 + rng.below(3) {
            let i = rng.below(files.len());
            mutate(&mut rng, &mut files[i].1);
        }
        run(&files, &["a.bin".to_string()]);
    }
}

#[test]
fn long_use_chains_and_nesting_never_panic() {
    for depth in [1usize, 10, 63, 64, 65, 200, 1000] {
        let mut files: Vec<(String, Vec<u8>)> = vec![
            ("mod.toml".into(), TOML.as_bytes().to_vec()),
            ("src/main.efa".into(), b"use \"f0.efa\"\nexport fn init() {\n    ret\n}\n".to_vec()),
        ];
        for i in 0..depth {
            let body = if i + 1 < depth { format!("use \"f{}.efa\"\n", i + 1) } else { "pub const X = 1\n".to_string() };
            files.push((format!("src/f{i}.efa"), body.into_bytes()));
        }
        run(&files, &[]);
    }
    for n in [1usize, 127, 128, 129, 1000, 20000] {
        for open in ["(", "{", "[", "\n{\n"] {
            let main = format!("export fn init() {{\n{}\n", open.repeat(n));
            let files: Vec<(String, Vec<u8>)> =
                vec![("mod.toml".into(), TOML.as_bytes().to_vec()), ("src/main.efa".into(), main.into_bytes())];
            run(&files, &[]);
        }
    }
}

#[test]
fn hostile_mod_toml_and_assets_never_panic() {
    let tomls = [
        String::new(),
        "[mod]".to_string(),
        "[mod]\nid = \"\"\nversion = \"\"\n".to_string(),
        format!("[mod]\nid = \"a\"\nversion = \"{}\"\n", "9".repeat(5000)),
        format!("[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[meta]\nx = {}\n", "[".repeat(3000) + &"]".repeat(3000)),
        "[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[permissions]\nbudget = 99999999999999999999\nmemory_pages = -1\n".to_string(),
        "[mod]\nid = \"a\"\nversion = \"1.0.0\"\n[build]\nassets = \"/etc\"\nentry = \"\"\napi = [\"..\", \"a/../../b\"]\n".to_string(),
        "\u{0}\u{1}\u{2}".to_string(),
    ];
    for t in tomls {
        let files = vec![("mod.toml".to_string(), t.into_bytes()), ("src/main.efa".to_string(), MAIN.as_bytes().to_vec())];
        run(&files, &["a.bin".to_string(), "../x".to_string(), "".to_string(), "a//b".to_string(), "a\\b".to_string()]);
    }
}
