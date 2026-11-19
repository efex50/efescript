//! Runner for efescript v2 programs: load `.efa`/`.efvm`, run export `main`, report traps.

use std::fmt;

use efevm::{Exit, Instance, Limits, Module, Trap, ValType};

use crate::asm::{self, Diagnostics};
use crate::host_std::{std_registry, StdEnv, StdOptions};
use crate::prelude::Host;

/// Why a program could not be loaded.
#[derive(Debug)]
pub enum LoadFailure {
    Io(String),
    Assemble(Diagnostics),
    Load(String),
}

impl fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadFailure::Io(m) | LoadFailure::Load(m) => f.write_str(m),
            LoadFailure::Assemble(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for LoadFailure {}

/// A loaded module, with the source text when it is known (file name, text).
pub struct Program {
    pub module: efevm::Module,
    pub source: Option<(String, String)>,
}

fn decode(path: &str, bytes: &[u8]) -> Result<Program, LoadFailure> {
    match Module::decode(bytes) {
        Ok(module) => Ok(Program { module, source: None }),
        Err(e) => Err(LoadFailure::Load(format!("{path}: {e}"))),
    }
}

/// Loads a program from a file: `.efa` is assembled, `.efvm` decoded; other extensions are
/// decoded when they start with the `EFVM` magic.
pub fn load_path(path: &str) -> Result<Program, LoadFailure> {
    load_path_with(path, &asm::AsmOptions::default())
}

/// Like [`load_path`], with assembler options (an API index for `--api`).
pub fn load_path_with(path: &str, opts: &asm::AsmOptions) -> Result<Program, LoadFailure> {
    let bytes = std::fs::read(path).map_err(|e| LoadFailure::Io(format!("{path}: {e}")))?;
    if path.ends_with(".efa") {
        let text = String::from_utf8(bytes)
            .map_err(|_| LoadFailure::Io(format!("{path}: not valid UTF-8")))?;
        return load_program_with(&text, path, opts);
    }
    if path.ends_with(".efvm") || bytes.starts_with(b"EFVM") {
        return decode(path, &bytes);
    }
    Err(LoadFailure::Load(format!(
        "{path}: unknown file type (expected .efa or .efvm)"
    )))
}

/// Decodes a module from bytes (for example an embedded bundle payload).
pub fn load_bytes(bytes: &[u8], name: &str) -> Result<Program, LoadFailure> {
    decode(name, bytes)
}

/// If the running executable has an embedded module (see `bundle`), runs it like
/// `efescript run` and returns the exit code (2 for a load error or a corrupt trailer,
/// 3 for a trap), printing reports to stderr. `None` when nothing is embedded.
pub fn run_embedded() -> Option<i32> {
    let bytes = match crate::bundle::embedded_module_of_current_exe() {
        Ok(Some(b)) => b,
        Ok(None) => return None,
        Err(e) => {
            eprintln!("{e}");
            return Some(2);
        }
    };
    let prog = match load_bytes(&bytes, "<embedded module>") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Some(2);
        }
    };
    let mut env = StdEnv::with_stdin(crate::prelude::StdHost);
    let outcome = run(&prog, &mut env, None);
    {
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    if let Some(e) = outcome.error {
        eprintln!("{e}");
    }
    Some(outcome.exit_code)
}

/// Assembles `.efa` text; `file` is used in diagnostics and debug info.
pub fn load_source(text: &str, file: &str) -> Result<Program, LoadFailure> {
    load_source_with(text, file, &asm::AsmOptions::default())
}

/// Like [`load_source`], with assembler options.
pub fn load_source_with(text: &str, file: &str, opts: &asm::AsmOptions) -> Result<Program, LoadFailure> {
    match asm::assemble_with(text, file, opts) {
        Ok(module) => Ok(Program {
            module,
            source: Some((file.to_string(), text.to_string())),
        }),
        Err(d) => Err(LoadFailure::Assemble(d)),
    }
}

/// The root file's text is served as given (so a root that is a symbolic link keeps
/// working); every other file goes through the confined [`asm::FsSource`].
struct RootSource<'a> {
    root: &'a str,
    text: &'a str,
    fs: asm::FsSource,
}

impl asm::FileSource for RootSource<'_> {
    fn read(&self, path: &str) -> Result<String, String> {
        if path == self.root {
            Ok(self.text.to_string())
        } else {
            self.fs.read(path)
        }
    }
    fn display_name(&self, path: &str) -> String {
        self.fs.display_name(path)
    }
}

/// Assembles the program whose root file is `file` with the given `text`, following `use`
/// through the file system (efa-syntax 16): the other files are found relative to the
/// root's directory, and named in debug info with that directory as typed in front.
pub fn load_program_with(text: &str, file: &str, opts: &asm::AsmOptions) -> Result<Program, LoadFailure> {
    let (fs, rel) = asm::FsSource::for_file(file);
    let src = RootSource { root: &rel, text, fs };
    match asm::assemble_program(&rel, &src, opts) {
        Ok(module) => Ok(Program { module, source: Some((file.to_string(), text.to_string())) }),
        Err(d) => Err(LoadFailure::Assemble(d)),
    }
}

/// The result of running a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub exit_code: i32,
    /// Text for stderr (already prefixed with `error:`), if the run failed.
    pub error: Option<String>,
}

fn fail(code: i32, msg: String) -> Outcome {
    Outcome { exit_code: code, error: Some(msg) }
}

/// Runs export `main` (no parameters; results `()` or `(i32)`).
pub fn run<H: Host + 'static>(prog: &Program, env: &mut StdEnv<H>, budget: Option<u64>) -> Outcome {
    run_with(prog, env, budget, &StdOptions::default())
}

/// Like [`run`], with `std` options (the `std.fs` root).
pub fn run_with<H: Host + 'static>(
    prog: &Program,
    env: &mut StdEnv<H>,
    budget: Option<u64>,
    opts: &StdOptions,
) -> Outcome {
    let Some(main_idx) = prog.module.export_func("main") else {
        return fail(2, "error: the module has no exported function `main`".into());
    };
    let ok_sig = prog
        .module
        .func_type(main_idx)
        .map(|t| t.params.is_empty() && (t.results.is_empty() || t.results == [ValType::I32]))
        .unwrap_or(false);
    if !ok_sig {
        return fail(
            2,
            "error: `main` must take no parameters and return nothing or one i32".into(),
        );
    }
    let imports = std_registry::<H>(opts).imports();
    let problems = imports.check_module(&prog.module);
    if !problems.is_empty() {
        return fail(2, link_report(&problems));
    }
    let mut inst = match Instance::new(&prog.module, &imports, Limits::default()) {
        Ok(i) => i,
        Err(e) => return fail(2, format!("error: {e}")),
    };
    let fiber = match inst.spawn("main", &[]) {
        Ok(f) => f,
        Err(e) => return fail(2, format!("error: cannot start `main`: {e}")),
    };
    let mut remaining = budget.unwrap_or(u64::MAX);
    loop {
        let (exit, used) = inst.resume_units(fiber, env, remaining);
        if budget.is_some() {
            remaining = remaining.saturating_sub(used);
        }
        match exit {
            Exit::Yield => {
                if let Some(c) = env.exit_code() {
                    return Outcome { exit_code: c, error: None };
                }
            }
            Exit::Done => {
                let code = env.exit_code().unwrap_or_else(|| {
                    inst.results(fiber)
                        .and_then(|r| r.first())
                        .map(|v| *v as u32 as i32)
                        .unwrap_or(0)
                });
                return Outcome { exit_code: code, error: None };
            }
            Exit::OutOfBudget => {
                if let Some(c) = env.exit_code() {
                    return Outcome { exit_code: c, error: None };
                }
                let n = budget.map(|b| b.to_string()).unwrap_or_else(|| "unlimited".into());
                return fail(3, format!("error: instruction budget exhausted ({n})"));
            }
            Exit::Trap(t) => {
                let bt = inst.backtrace(fiber).map(<[(u32, u32)]>::to_vec).unwrap_or_default();
                return fail(3, trap_report(prog, &t, &bt));
            }
        }
    }
}

/// Every link problem on its own line, by name.
fn link_report(problems: &[efevm::LinkError]) -> String {
    let mut out = String::from("error: cannot link the module:");
    for p in problems {
        out.push_str(&format!("\n  {p}"));
    }
    out
}

fn func_name(prog: &Program, func: u32) -> String {
    prog.module
        .data()
        .names
        .funcs
        .iter()
        .find(|(i, _)| *i == func)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| format!("f{func}"))
}

/// `(file, line, column)` of the lines entry with the greatest instruction index <= `inst`.
fn lookup_line(prog: &Program, func: u32, inst: u32) -> Option<(String, u32, u32)> {
    let lines = &prog.module.data().lines;
    let fl = lines.funcs.iter().find(|f| f.func == func)?;
    let e = fl.entries.iter().filter(|e| e.inst <= inst).max_by_key(|e| e.inst)?;
    let file = lines.files.get(e.file as usize)?.clone();
    Some((file, e.line, e.column))
}

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const HEAD_FRAMES: usize = 16;
const TAIL_FRAMES: usize = 4;

/// Reads a source file for a report: regular files up to 16 MiB only.
fn read_source_file(file: &str) -> Option<String> {
    let md = std::fs::metadata(file).ok()?;
    if !md.is_file() || md.len() > MAX_SOURCE_BYTES {
        return None;
    }
    std::fs::read_to_string(file).ok()
}

fn frame_text(
    prog: &Program,
    cache: &mut std::collections::HashMap<String, Option<String>>,
    func: u32,
    inst: u32,
) -> String {
    let name = func_name(prog, func);
    let Some((file, line, col)) = lookup_line(prog, func, inst) else {
        return format!("\n  at {name} (inst {inst})");
    };
    let mut out = format!("\n  at {name} ({file}:{line}:{col})");
    let text = cache.entry(file.clone()).or_insert_with(|| match &prog.source {
        Some((name, t)) if *name == file => Some(t.clone()),
        _ => read_source_file(&file),
    });
    let src = text
        .as_deref()
        .and_then(|t| t.lines().nth((line as usize).checked_sub(1)?))
        .map(str::trim_end);
    if let Some(src) = src {
        out.push_str(&format!("\n{line:>7} | {src}"));
    }
    out
}

/// Formats a trap and its backtrace (innermost frame first) for stderr. Long backtraces show
/// the 16 innermost and 4 outermost frames.
pub fn trap_report(prog: &Program, trap: &Trap, backtrace: &[(u32, u32)]) -> String {
    let mut out = format!("error: trap: {trap}");
    let mut cache = std::collections::HashMap::new();
    let n = backtrace.len();
    for (i, &(func, inst)) in backtrace.iter().enumerate() {
        if n > HEAD_FRAMES + TAIL_FRAMES && i >= HEAD_FRAMES && i < n - TAIL_FRAMES {
            if i == HEAD_FRAMES {
                out.push_str(&format!("\n  ... {} more frames", n - HEAD_FRAMES - TAIL_FRAMES));
            }
            continue;
        }
        out.push_str(&frame_text(prog, &mut cache, func, inst));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::CaptureHost;

    fn go(src: &str, budget: Option<u64>) -> (Outcome, String) {
        let cap = CaptureHost::shared();
        let prog = match load_source(src, "t.efa") {
            Ok(p) => p,
            Err(e) => return (fail(2, e.to_string()), String::new()),
        };
        let mut env = StdEnv::new(cap.clone());
        let o = run(&prog, &mut env, budget);
        let out = cap.borrow().out_string();
        (o, out)
    }

    #[test]
    fn main_returns_i32() {
        let (o, _) = go("export fn main() -> i32 {\n    reg x\n    const.i32 x, 7\n    ret x\n}\n", None);
        assert_eq!(o, Outcome { exit_code: 7, error: None });
    }

    #[test]
    fn main_returns_nothing() {
        let (o, _) = go("export fn main() {\n    ret\n}\n", None);
        assert_eq!(o, Outcome { exit_code: 0, error: None });
    }

    #[test]
    fn exit_wins() {
        let src = "import std.exit(i32)\nexport fn main() -> i32 {\n    reg x\n    call std.exit(5)\n    const.i32 x, 7\n    ret x\n}\n";
        let (o, _) = go(src, None);
        assert_eq!(o.exit_code, 5);
    }

    #[test]
    fn trap_report_has_location() {
        let src = "fn div(a: i64, b: i64) -> i64 {\n    i64.div_s a, a, b\n    ret a\n}\n\nexport fn main() {\n    reg r\n    call div(1, 0) -> r\n    ret\n}\n";
        let (o, _) = go(src, None);
        assert_eq!(o.exit_code, 3);
        let e = o.error.unwrap_or_default();
        assert!(e.starts_with("error: trap: integer divide by zero"), "{e}");
        assert!(e.contains("at div (t.efa:2:"), "{e}");
        assert!(e.contains("at main (t.efa:"), "{e}");
        assert!(e.contains("2 |     i64.div_s a, a, b"), "{e}");
    }

    #[test]
    fn budget_exhausted() {
        let (o, _) = go("export fn main() {\nl:\n    jmp l\n}\n", Some(100));
        assert_eq!(o.exit_code, 3);
        assert_eq!(o.error.as_deref(), Some("error: instruction budget exhausted (100)"));
    }

    #[test]
    fn bad_main_signature() {
        let (o, _) = go("export fn main(a: i64) {\n    ret\n}\n", None);
        assert_eq!(o.exit_code, 2);
    }

    #[test]
    fn missing_import() {
        let src = "import std.nothing(i64)\nexport fn main() {\n    call std.nothing(1)\n    ret\n}\n";
        let (o, _) = go(src, None);
        assert_eq!(o.exit_code, 2);
    }

    #[test]
    fn budget_counts_bulk_units_across_yields() {
        let src = "memory 1\nexport fn main() {\nl:\n    memory.fill 0, 0, 60000\n    yield\n    jmp l\n}\n";
        let (o, _) = go(src, Some(100_000));
        assert_eq!(o.exit_code, 3);
        assert_eq!(o.error.as_deref(), Some("error: instruction budget exhausted (100000)"));
    }

    #[test]
    fn deep_backtrace_is_capped() {
        let src = "fn rec() {\n    call rec()\n    ret\n}\nexport fn main() {\n    call rec()\n    ret\n}\n";
        let (o, _) = go(src, None);
        assert_eq!(o.exit_code, 3, "{:?}", o.error);
        let e = o.error.unwrap_or_default();
        assert!(e.lines().count() <= 1 + 2 * 20 + 1, "{}", e.lines().count());
        assert!(e.contains("more frames"), "{e}");
    }
}
