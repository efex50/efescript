//! `--api` repeated on the command line: the indices are merged with `ApiIndex::merge`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use efevm::api::{ApiIndex, ApiType, HostFn, HostRegistry};
use efevm::{HostOk, Module};

const BIN: &str = env!("CARGO_BIN_EXE_efescript");

fn efescript(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().expect("spawn efescript")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A per-test scratch directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("efescript-merge-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("write temp file");
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("UTF-8 temp path")
}

/// An index of one game: the `engine.log` namespace (at `version`) with the functions `names`.
/// Every function has the same content wherever it appears, so games share them.
fn log_index(game: &str, names: &[&str], version: u32) -> ApiIndex {
    let mut r: HostRegistry<()> = HostRegistry::new();
    r.set_game(game);
    r.define_namespace("engine.log", version, "engine", "Logging.").unwrap();
    for name in names {
        r.register(
            HostFn::new("engine.log", *name).doc("Log a line.").param("text", ApiType::Str),
            |_, _, _| Ok(HostOk::Continue),
        )
        .unwrap();
    }
    r.index()
}

/// Imports `debug` (only gameA has it) and `warn` (only gameB has it).
const SCRIPT: &str = "\
import engine.log.debug
import engine.log.warn

msg db \"hi\"

export fn main() -> i32 {
    reg n
    call engine.log.debug(msg, msg.len)
    call engine.log.warn(msg, msg.len)
    const.i32 n, 7
    ret n
}
";

#[test]
fn build_with_repeated_api_merges_the_indices() {
    let dir = Scratch::new("two");
    let a = log_index("gameA", &["info", "debug"], 1);
    let b = log_index("gameB", &["info", "warn"], 1);
    let a_api = dir.write("a.efapi", &a.to_json());
    let b_api = dir.write("b.efapi", &b.to_json());
    let script = dir.write("game.efa", SCRIPT);
    let out = dir.0.join("game.efvm");

    // Each index alone lacks one of the two imports: the namespace is listed, the function is not.
    let o = efescript(&["build", s(&script), "--api", s(&a_api), "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("engine.log.warn"), "{}", stderr(&o));
    let o = efescript(&["build", s(&script), "--api", s(&b_api), "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("engine.log.debug"), "{}", stderr(&o));

    // Both together: the module records the hash of the merged index.
    let o = efescript(&["build", s(&script), "--api", s(&a_api), "--api", s(&b_api), "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let merged = ApiIndex::merge(&[&a, &b]).expect("merge");
    assert_eq!(merged.functions.len(), 3, "info is kept once");
    let m = Module::decode(&std::fs::read(&out).unwrap()).unwrap();
    let api = m.data().api.clone().expect("section 12");
    assert_eq!(api.hash, merged.hash());
    assert_eq!(api.namespaces, vec![("engine.log".to_string(), 1)]);

    // The order of the flags does not change the result.
    let out2 = dir.0.join("game2.efvm");
    let o = efescript(&["build", s(&script), "--api", s(&b_api), "--api", s(&a_api), "-o", s(&out2)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let m2 = Module::decode(&std::fs::read(&out2).unwrap()).unwrap();
    assert_eq!(m2.data().api.as_ref().map(|a| a.hash), Some(merged.hash()));

    // The same name with different content is an error that names both games.
    let c = log_index("gameC", &["info"], 2);
    let c_api = dir.write("c.efapi", &c.to_json());
    let o = efescript(&["build", s(&script), "--api", s(&a_api), "--api", s(&c_api), "-o", s(&out)]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("namespace `engine.log` differs between `gameA` and `gameC`"), "{}", stderr(&o));
}
