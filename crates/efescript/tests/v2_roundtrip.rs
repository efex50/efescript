//! Round trip of every `examples/*.efa`: assemble -> disassemble -> assemble (design section 9).

use efescript::asm::assemble;
use efescript::disasm::disassemble;
use efevm::Module;

fn strip_lines(m: &Module) -> Vec<u8> {
    let mut data = m.data().clone();
    data.lines = Default::default();
    Module::new(data).expect("stripped module verifies").encode()
}

fn examples() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).expect("examples dir") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|s| s.to_str()) == Some("efa") {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let text = std::fs::read_to_string(&p).expect("read example");
            v.push((name, text));
        }
    }
    v.sort();
    v
}

#[test]
fn examples_include_the_six() {
    let names: Vec<String> = examples().into_iter().map(|(n, _)| n).collect();
    for want in ["hello.efa", "strings.efa", "jumps.efa", "recursion.efa", "sound.efa", "floats.efa"] {
        assert!(names.iter().any(|n| n == want), "missing {want} in {names:?}");
    }
}

#[test]
fn assemble_disassemble_assemble_is_identical() {
    for (name, src) in examples() {
        let m1 = assemble(&src, &name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let text = disassemble(&m1);
        let m2 = assemble(&text, "dis.efa")
            .unwrap_or_else(|e| panic!("{name}: reassembly failed: {e}\n--- text ---\n{text}"));
        assert_eq!(strip_lines(&m1), strip_lines(&m2), "{name}: bytes differ\n--- text ---\n{text}");
    }
}

#[test]
fn disassembly_is_idempotent() {
    for (name, src) in examples() {
        let m1 = assemble(&src, &name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let t1 = disassemble(&m1);
        let m2 = assemble(&t1, "dis.efa").unwrap_or_else(|e| panic!("{name}: {e}"));
        let t2 = disassemble(&m2);
        assert_eq!(t1, t2, "{name}: second disassembly differs");
    }
}

#[test]
fn decode_encode_roundtrip_and_disassembly() {
    for (name, src) in examples() {
        let m1 = assemble(&src, &name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let bytes = m1.encode();
        let m2 = Module::decode(&bytes).unwrap_or_else(|e| panic!("{name}: decode: {e}"));
        assert_eq!(m1.data(), m2.data(), "{name}: decoded data differs");
        assert_eq!(bytes, m2.encode(), "{name}: re-encode differs");
        let text = disassemble(&m2);
        assert_eq!(text, disassemble(&m1), "{name}: disassembly of decoded module differs");
        let m3 = assemble(&text, "dis.efa").unwrap_or_else(|e| panic!("{name}: {e}\n{text}"));
        assert_eq!(strip_lines(&m2), strip_lines(&m3), "{name}: reassembly differs");
    }
}
