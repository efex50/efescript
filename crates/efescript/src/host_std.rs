//! The `std` host namespaces for efescript v2 programs (design: crates/efescript/docs/e3-design.md
//! section 4): `std` (console, sound, exit), `std.time` and `std.fs`, all registered on a
//! [`HostRegistry`] so they are listed in the API index.

use std::io::{BufRead, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use efevm::api::{ApiType, HostFn, HostRegistry};
use efevm::{HostOk, Trap};

use crate::prelude::Host;

/// Host state for programs that import `std.*`: wraps an output/sound `Host`, an input
/// source for `read_line`, the exit code requested by `std.exit` and the start time of
/// `std.time.now_ms`.
pub struct StdEnv<H: Host> {
    /// Output and sound host.
    pub host: H,
    input: Box<dyn BufRead>,
    exit_code: Option<i32>,
    start: Instant,
}

impl<H: Host> StdEnv<H> {
    /// No input: `read_line` sees end of input.
    pub fn new(host: H) -> Self {
        Self::with_input(host, Vec::new())
    }

    /// `read_line` reads the process stdin.
    pub fn with_stdin(host: H) -> Self {
        Self {
            host,
            input: Box::new(std::io::BufReader::new(std::io::stdin())),
            exit_code: None,
            start: Instant::now(),
        }
    }

    /// `read_line` reads these bytes (for tests).
    pub fn with_input(host: H, input: impl Into<Vec<u8>>) -> Self {
        Self {
            host,
            input: Box::new(std::io::Cursor::new(input.into())),
            exit_code: None,
            start: Instant::now(),
        }
    }

    /// The code passed to `std.exit`, if it was called.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

/// Options of the `std` host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StdOptions {
    /// The directory `std.fs` works in. `None` (the default) denies every `std.fs` call.
    pub fs_root: Option<PathBuf>,
}

/// `std.fs` status: the call is not allowed (no root, or the path is not allowed).
const DENIED: i32 = -2;
/// `std.fs` status: an I/O error.
const IO_ERROR: i32 = -1;

/// Resolves `path` for reading: relative, no `..`, existing, and (after canonicalisation,
/// which follows symlinks) inside the canonical root.
fn resolve_existing(root: &Path, path: &str) -> Result<PathBuf, i32> {
    let rel = checked_relative(path)?;
    let root = root.canonicalize().map_err(|_| IO_ERROR)?;
    let full = root.join(rel).canonicalize().map_err(|_| IO_ERROR)?;
    if full.starts_with(&root) { Ok(full) } else { Err(DENIED) }
}

/// Resolves `path` for writing: the parent directory must exist and be inside the root
/// (canonicalised); an existing symlink at the target is followed and must stay inside too.
fn resolve_for_write(root: &Path, path: &str) -> Result<PathBuf, i32> {
    let rel = checked_relative(path)?;
    let root = root.canonicalize().map_err(|_| IO_ERROR)?;
    let joined = root.join(rel);
    let name = joined.file_name().ok_or(DENIED)?.to_owned();
    let parent = joined.parent().ok_or(DENIED)?.canonicalize().map_err(|_| IO_ERROR)?;
    if !parent.starts_with(&root) {
        return Err(DENIED);
    }
    let target = parent.join(name);
    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        let real = target.canonicalize().map_err(|_| DENIED)?;
        if !real.starts_with(&root) {
            return Err(DENIED);
        }
        return Ok(real);
    }
    Ok(target)
}

/// A non-empty relative path made of normal components only (and `.`).
fn checked_relative(path: &str) -> Result<&Path, i32> {
    let p = Path::new(path);
    if path.is_empty() || p.is_absolute() {
        return Err(DENIED);
    }
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(DENIED),
        }
    }
    Ok(p)
}

/// Builds the `std` registry: namespaces `std`, `std.time`, `std.fs` (version 1, owner
/// `std`) and their functions. `std.fs` is always registered (the index does not depend on
/// options) but every call returns -2 unless `opts.fs_root` is set. The index's game name is
/// `std`.
pub fn std_registry<H: Host + 'static>(opts: &StdOptions) -> HostRegistry<StdEnv<H>> {
    let mut reg = HostRegistry::new();
    reg.set_game("std");
    register_all(&mut reg, opts.fs_root.clone().map(Arc::new))
        .expect("the std registry is statically valid");
    reg
}

fn register_all<H: Host + 'static>(
    reg: &mut HostRegistry<StdEnv<H>>,
    fs_root: Option<Arc<PathBuf>>,
) -> Result<(), efevm::api::RegistryError> {
    reg.define_namespace("std", 1, "std", "Console output and input, sound and process control.")?;
    reg.define_namespace("std.time", 1, "std", "Clocks.")?;
    reg.define_namespace(
        "std.fs",
        1,
        "std",
        "Files below a root directory chosen by the host. Every call returns -2 when the host gave no root.",
    )?;

    reg.register(
        HostFn::new("std", "print")
            .doc("Write the bytes of `text` to standard output, without a line break.")
            .param("text", ApiType::Str),
        |caller, args, _| {
            let bytes = caller.read_bytes(args.str(0)?)?;
            caller.host().host.write_out(&bytes);
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "println")
            .doc("Write the bytes of `text` to standard output, followed by a line break.")
            .param("text", ApiType::Str),
        |caller, args, _| {
            let bytes = caller.read_bytes(args.str(0)?)?;
            let host = &mut caller.host().host;
            host.write_out(&bytes);
            host.write_out(b"\n");
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "print_i64")
            .doc("Write `v` as a signed decimal number, without a line break.")
            .param("v", ApiType::I64),
        |caller, args, _| {
            let s = args.i64(0)?.to_string();
            caller.host().host.write_out(s.as_bytes());
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "print_f64")
            .doc("Write `v` in Rust's shortest round-trip decimal form, without a line break.")
            .param("v", ApiType::F64),
        |caller, args, _| {
            let s = format!("{}", args.f64(0)?);
            caller.host().host.write_out(s.as_bytes());
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "print_char")
            .doc("Write the Unicode character with code point `c` as UTF-8; an invalid code point traps.")
            .param("c", ApiType::I32),
        |caller, args, _| {
            let Some(ch) = char::from_u32(args.i32(0)? as u32) else {
                return Err(Trap::Host("std.print_char: invalid character".into()));
            };
            let mut buf = [0u8; 4];
            caller.host().host.write_out(ch.encode_utf8(&mut buf).as_bytes());
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "read_line")
            .doc(
                "Read one line from standard input into `buf` without its line break, truncated \
                 to the buffer size; returns the bytes written, or -1 at end of input.",
            )
            .param("buf", ApiType::Bytes)
            .result("len", ApiType::I64),
        |caller, args, res| {
            let buf = args.bytes(0)?;
            if buf.ptr > i64::MAX as u64 || buf.len > i64::MAX as u64 {
                return Err(Trap::OutOfBounds);
            }
            let mut line = Vec::new();
            match caller.host().input.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => res.set_i64(0, -1)?,
                Ok(_) => {
                    if line.ends_with(b"\n") {
                        line.pop();
                        if line.ends_with(b"\r") {
                            line.pop();
                        }
                    }
                    let n = line.len().min(usize::try_from(buf.len).unwrap_or(usize::MAX));
                    caller.write_bytes(buf.ptr, &line[..n])?;
                    res.set_i64(0, n as i64)?;
                }
            }
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "play_sound")
            .doc("Play the sound file at `path` and wait until it ends; returns 0, or 1 on any failure.")
            .param("path", ApiType::Str)
            .result("status", ApiType::I32),
        |caller, args, res| {
            let bytes = caller.read_bytes(args.str(0)?)?;
            let status = match std::str::from_utf8(&bytes) {
                Err(_) => 1,
                Ok(path) => match caller.host().host.play_sound(path) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("efescript: std.play_sound failed: {e}");
                        1
                    }
                },
            };
            res.set_i32(0, status)?;
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std", "exit")
            .doc("Stop the program with exit code `code`.")
            .param("code", ApiType::I32),
        |caller, args, _| {
            caller.host().exit_code = Some(args.i32(0)?);
            Ok(HostOk::Yield)
        },
    )?;

    reg.register(
        HostFn::new("std.time", "now_ms")
            .doc("Milliseconds on a monotonic clock since the program started.")
            .result("ms", ApiType::I64),
        |caller, _, res| {
            let ms = caller.host().start.elapsed().as_millis();
            res.set_i64(0, i64::try_from(ms).unwrap_or(i64::MAX))?;
            Ok(HostOk::Continue)
        },
    )?;
    reg.register(
        HostFn::new("std.time", "unix_ms")
            .doc("Wall-clock milliseconds since 1970-01-01 UTC (0 if the clock is before that).")
            .result("ms", ApiType::I64),
        |_, _, res| {
            let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            res.set_i64(0, i64::try_from(ms).unwrap_or(i64::MAX))?;
            Ok(HostOk::Continue)
        },
    )?;

    let root = fs_root.clone();
    reg.register(
        HostFn::new("std.fs", "read_file")
            .doc(
                "Read the file `path` (relative to the host's root) into `buf`; returns the number \
                 of bytes copied (at most the buffer size, not the file size), -1 on an I/O error, \
                 -2 when denied (no root, absolute path, `..`, or outside the root).",
            )
            .param("path", ApiType::Str)
            .param("buf", ApiType::Bytes)
            .result("len", ApiType::I64),
        move |caller, args, res| {
            let Some(root) = root.as_deref() else {
                res.set_i64(0, DENIED as i64)?;
                return Ok(HostOk::Continue);
            };
            let path = caller.read_str(args.str(0)?)?;
            let buf = args.bytes(1)?;
            if buf.ptr > i64::MAX as u64 || buf.len > i64::MAX as u64 {
                return Err(Trap::OutOfBounds);
            }
            let want = buf.len.min(caller.memory().len());
            let out = resolve_existing(root, &path).and_then(|full| {
                let file = std::fs::File::open(full).map_err(|_| IO_ERROR)?;
                let mut data = Vec::new();
                file.take(want).read_to_end(&mut data).map_err(|_| IO_ERROR)?;
                Ok(data)
            });
            match out {
                Ok(data) => {
                    caller.write_bytes(buf.ptr, &data)?;
                    res.set_i64(0, data.len() as i64)?;
                }
                Err(code) => res.set_i64(0, code as i64)?,
            }
            Ok(HostOk::Continue)
        },
    )?;
    let root = fs_root;
    reg.register(
        HostFn::new("std.fs", "write_file")
            .doc(
                "Create or replace the file `path` (relative to the host's root, parent directory \
                 must exist) with `data`; returns 0, -1 on an I/O error, -2 when denied (no root, \
                 absolute path, `..`, or outside the root).",
            )
            .param("path", ApiType::Str)
            .param("data", ApiType::Bytes)
            .result("status", ApiType::I32),
        move |caller, args, res| {
            let Some(root) = root.as_deref() else {
                res.set_i32(0, DENIED)?;
                return Ok(HostOk::Continue);
            };
            let path = caller.read_str(args.str(0)?)?;
            let data = caller.read_bytes(args.bytes(1)?)?;
            let status = match resolve_for_write(root, &path) {
                Ok(full) => std::fs::write(full, data).map_or(IO_ERROR, |_| 0),
                Err(code) => code,
            };
            res.set_i32(0, status)?;
            Ok(HostOk::Continue)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::CaptureHost;
    use efevm::{Exit, FiberId, FuncType, FunctionBuilder, Instance, Limits, ModuleBuilder, ValType};

    fn reg() -> HostRegistry<StdEnv<CaptureHost>> {
        std_registry(&StdOptions::default())
    }

    /// Imports `ns.name` with the signature the registry declares for it.
    fn import(mb: &mut ModuleBuilder, ns: &str, name: &str) -> u32 {
        let f = reg().function(ns, name).expect("registered").func_type();
        mb.import(ns, name, f)
    }

    fn import_std(mb: &mut ModuleBuilder, name: &str) -> u32 {
        import(mb, "std", name)
    }

    fn run_main_with(
        opts: &StdOptions,
        data: &[(u64, &[u8])],
        nregs: u16,
        results: &[ValType],
        body: impl FnOnce(&mut ModuleBuilder, &mut FunctionBuilder),
        env: &mut StdEnv<CaptureHost>,
    ) -> (Exit, Instance<StdEnv<CaptureHost>>, FiberId) {
        let mut mb = ModuleBuilder::new();
        let mut fb = FunctionBuilder::new(nregs);
        body(&mut mb, &mut fb);
        let f = mb.add_func(FuncType::new([], results.iter().copied()), fb);
        mb.export_func("main", f);
        mb.memory(1, None);
        for (offset, bytes) in data {
            mb.data(*offset, bytes);
        }
        let module = mb.finish().unwrap();
        let imports = std_registry::<CaptureHost>(opts).imports();
        let mut inst = Instance::new(&module, &imports, Limits::default()).unwrap();
        let fiber = inst.spawn("main", &[]).unwrap();
        let exit = inst.resume(fiber, env, 1000);
        (exit, inst, fiber)
    }

    fn run_main(
        data: Option<(u64, &[u8])>,
        nregs: u16,
        results: &[ValType],
        body: impl FnOnce(&mut ModuleBuilder, &mut FunctionBuilder),
        env: &mut StdEnv<CaptureHost>,
    ) -> (Exit, Instance<StdEnv<CaptureHost>>, FiberId) {
        let data: Vec<(u64, &[u8])> = data.into_iter().collect();
        run_main_with(&StdOptions::default(), &data, nregs, results, body, env)
    }

    #[test]
    fn test_println() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(Some((1024, b"hi")), 2, &[], |mb, fb| {
            let p = import_std(mb, "println");
            fb.const_i64(0, 1024);
            fb.const_i64(1, 2);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        assert_eq!(env.host.out, b"hi\n");
    }

    #[test]
    fn test_print_i64() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(None, 1, &[], |mb, fb| {
            let p = import_std(mb, "print_i64");
            fb.const_i64(0, -42);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        assert_eq!(env.host.out, b"-42");
    }

    #[test]
    fn test_print_char() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(None, 1, &[], |mb, fb| {
            let p = import_std(mb, "print_char");
            fb.const_i32(0, 0xE9);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        assert_eq!(env.host.out, "é".as_bytes());

        let mut env2 = StdEnv::new(CaptureHost::new());
        let (exit2, _, _) = run_main(None, 1, &[], |mb, fb| {
            let p = import_std(mb, "print_char");
            fb.const_i32(0, 0xD800);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env2);
        assert_eq!(exit2, Exit::Trap(Trap::Host("std.print_char: invalid character".into())));
    }

    #[test]
    fn test_read_line() {
        let read = |input: &str, cap: i64, calls: usize| -> Vec<(u64, Vec<u8>)> {
            let mut mb = ModuleBuilder::new();
            let rl = import_std(&mut mb, "read_line");
            let mut fb = FunctionBuilder::new(2);
            fb.const_i64(0, 1024);
            fb.const_i64(1, cap);
            fb.call(rl, 0);
            fb.ret(0);
            let f = mb.add_func(FuncType::new([], [ValType::I64]), fb);
            mb.export_func("main", f);
            mb.memory(1, None);
            let module = mb.finish().unwrap();
            let mut env = StdEnv::with_input(CaptureHost::new(), input);
            let mut inst = Instance::new(&module, &reg().imports(), Limits::default()).unwrap();
            let mut out = Vec::new();
            for _ in 0..calls {
                let fib = inst.spawn("main", &[]).unwrap();
                assert_eq!(inst.resume(fib, &mut env, 1000), Exit::Done);
                let n = inst.results(fib).unwrap()[0];
                let mut buf = vec![0u8; (n as i64).max(0) as usize];
                inst.memory().read(1024, &mut buf).unwrap();
                out.push((n, buf));
            }
            out
        };
        let r = read("abc\r\nxyz", 16, 3);
        assert_eq!(r[0], (3, b"abc".to_vec()));
        assert_eq!(r[1], (3, b"xyz".to_vec()));
        assert_eq!(r[2].0, (-1i64) as u64);
        // truncation by the buffer size
        assert_eq!(read("hello\n", 2, 1)[0], (2, b"he".to_vec()));
    }

    #[test]
    fn test_play_sound() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, inst, fib) = run_main(Some((1024, b"test.wav")), 2, &[ValType::I32], |mb, fb| {
            let ps = import_std(mb, "play_sound");
            fb.const_i64(0, 1024);
            fb.const_i64(1, 8);
            fb.call(ps, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        assert_eq!(inst.results(fib).unwrap(), &[0]);
        assert_eq!(env.host.sounds, vec!["test.wav".to_string()]);
    }

    #[test]
    fn test_exit() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(None, 1, &[], |mb, fb| {
            let ex = import_std(mb, "exit");
            fb.const_i32(0, 42);
            fb.call(ex, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Yield);
        assert_eq!(env.exit_code(), Some(42));
    }

    #[test]
    fn test_print_out_of_bounds() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(None, 2, &[], |mb, fb| {
            let p = import_std(mb, "print");
            fb.const_i64(0, 65530);
            fb.const_i64(1, 10);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Trap(Trap::OutOfBounds));
    }

    #[test]
    fn test_read_line_out_of_bounds_traps() {
        // (buf ptr, buf len): a negative length and a negative pointer (bit 63 set)
        for (ptr, len) in [(1024i64, -1i64), (-1, 16)] {
            let mut env = StdEnv::with_input(CaptureHost::new(), "abc\n");
            let (exit, _, _) = run_main(None, 2, &[], |mb, fb| {
                let rl = import_std(mb, "read_line");
                fb.const_i64(0, ptr);
                fb.const_i64(1, len);
                fb.call(rl, 0);
                fb.ret(0);
            }, &mut env);
            assert_eq!(exit, Exit::Trap(Trap::OutOfBounds), "ptr {ptr} len {len}");
        }
    }

    #[test]
    fn test_print_f64_and_print() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, _, _) = run_main(Some((1024, b"ok")), 2, &[], |mb, fb| {
            let pf = import_std(mb, "print_f64");
            let p = import_std(mb, "print");
            fb.const_f64(0, 3.5);
            fb.call(pf, 0);
            fb.const_i64(0, 1024);
            fb.const_i64(1, 2);
            fb.call(p, 0);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        assert_eq!(env.host.out, b"3.5ok");

        let env_stdin = StdEnv::with_stdin(CaptureHost::new());
        assert_eq!(env_stdin.exit_code(), None);
    }

    #[test]
    fn registry_shape() {
        let r = reg();
        let ns: Vec<_> = r.namespaces().map(|n| (n.name.clone(), n.version, n.owner.clone())).collect();
        assert_eq!(
            ns,
            vec![
                ("std".to_string(), 1, "std".to_string()),
                ("std.fs".to_string(), 1, "std".to_string()),
                ("std.time".to_string(), 1, "std".to_string()),
            ]
        );
        assert_eq!(r.index().game, "std");
        assert_eq!(r.function("std", "read_line").unwrap().signature_text(), "(buf: bytes) -> (len: i64)");
        assert_eq!(r.function("std", "println").unwrap().func_type().params, [ValType::I64, ValType::I64]);
    }

    #[test]
    fn time_functions() {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, inst, fib) = run_main(None, 2, &[ValType::I64, ValType::I64], |mb, fb| {
            let now = import(mb, "std.time", "now_ms");
            let unix = import(mb, "std.time", "unix_ms");
            fb.call(now, 0);
            fb.call(unix, 1);
            fb.ret(0);
        }, &mut env);
        assert_eq!(exit, Exit::Done);
        let r = inst.results(fib).unwrap();
        assert!((0..5000).contains(&(r[0] as i64)), "now_ms {}", r[0] as i64);
        // after 2020-01-01
        assert!(r[1] as i64 > 1_577_836_800_000, "unix_ms {}", r[1] as i64);
    }

    // ---- std.fs ----

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "efescript-fs-{tag}-{}-{}-{nanos}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opts(root: &Path) -> StdOptions {
        StdOptions { fs_root: Some(root.to_path_buf()) }
    }

    /// `write_file(path, data)` through the VM; returns the status.
    fn fs_write(o: &StdOptions, path: &str, data: &[u8]) -> i32 {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, inst, fib) = run_main_with(
            o,
            &[(1024, path.as_bytes()), (4096, data)],
            4,
            &[ValType::I32],
            |mb, fb| {
                let w = import(mb, "std.fs", "write_file");
                fb.const_i64(0, 1024);
                fb.const_i64(1, path.len() as i64);
                fb.const_i64(2, 4096);
                fb.const_i64(3, data.len() as i64);
                fb.call(w, 0);
                fb.ret(0);
            },
            &mut env,
        );
        assert_eq!(exit, Exit::Done);
        inst.results(fib).unwrap()[0] as u32 as i32
    }

    /// `read_file(path, buf[cap])` through the VM; returns the result and the buffer prefix.
    fn fs_read(o: &StdOptions, path: &str, cap: i64) -> (i64, Vec<u8>) {
        let mut env = StdEnv::new(CaptureHost::new());
        let (exit, inst, fib) = run_main_with(
            o,
            &[(1024, path.as_bytes())],
            4,
            &[ValType::I64],
            |mb, fb| {
                let r = import(mb, "std.fs", "read_file");
                fb.const_i64(0, 1024);
                fb.const_i64(1, path.len() as i64);
                fb.const_i64(2, 4096);
                fb.const_i64(3, cap);
                fb.call(r, 0);
                fb.ret(0);
            },
            &mut env,
        );
        assert_eq!(exit, Exit::Done);
        let n = inst.results(fib).unwrap()[0] as i64;
        let mut buf = vec![0u8; n.max(0) as usize];
        inst.memory().read(4096, &mut buf).unwrap();
        (n, buf)
    }

    #[test]
    fn fs_denied_without_root() {
        let none = StdOptions::default();
        assert_eq!(fs_write(&none, "a.txt", b"x"), -2);
        assert_eq!(fs_read(&none, "a.txt", 16).0, -2);
    }

    #[test]
    fn fs_write_read_roundtrip() {
        let d = TempDir::new("rt");
        let o = opts(&d.0);
        assert_eq!(fs_write(&o, "a.txt", b"hello"), 0);
        assert_eq!(std::fs::read(d.0.join("a.txt")).unwrap(), b"hello");
        assert_eq!(fs_read(&o, "a.txt", 16), (5, b"hello".to_vec()));
        // copied length, truncated to the buffer
        assert_eq!(fs_read(&o, "a.txt", 3), (3, b"hel".to_vec()));
        // replaces, `./` is fine, subdirectories work when they exist
        assert_eq!(fs_write(&o, "./a.txt", b"yo"), 0);
        assert_eq!(fs_read(&o, "a.txt", 16), (2, b"yo".to_vec()));
        std::fs::create_dir(d.0.join("sub")).unwrap();
        assert_eq!(fs_write(&o, "sub/b.bin", &[0, 1, 2]), 0);
        assert_eq!(fs_read(&o, "sub/b.bin", 16), (3, vec![0, 1, 2]));
        // empty data makes an empty file
        assert_eq!(fs_write(&o, "empty", b""), 0);
        assert_eq!(fs_read(&o, "empty", 16), (0, vec![]));
    }

    #[test]
    fn fs_io_errors_are_minus_one() {
        let d = TempDir::new("io");
        let o = opts(&d.0);
        assert_eq!(fs_read(&o, "missing.txt", 16).0, -1);
        assert_eq!(fs_write(&o, "no_such_dir/a.txt", b"x"), -1);
        // a directory cannot be read as a file
        std::fs::create_dir(d.0.join("dir")).unwrap();
        assert_eq!(fs_read(&o, "dir", 16).0, -1);
        assert_eq!(fs_write(&o, "dir", b"x"), -1);
        // the root itself does not exist
        let gone = StdOptions { fs_root: Some(d.0.join("gone")) };
        assert_eq!(fs_read(&gone, "a", 16).0, -1);
    }

    #[test]
    fn read_file_buffer_out_of_bounds_traps() {
        let d = TempDir::new("bounds");
        std::fs::write(d.0.join("a.txt"), b"hello").unwrap();
        let o = opts(&d.0);
        // (buf ptr, buf len): a negative length and a negative pointer (bit 63 set)
        for (ptr, len) in [(4096i64, -1i64), (-1, 16)] {
            let mut env = StdEnv::new(CaptureHost::new());
            let (exit, _, _) = run_main_with(&o, &[(1024, b"a.txt")], 4, &[ValType::I64], |mb, fb| {
                let r = import(mb, "std.fs", "read_file");
                fb.const_i64(0, 1024);
                fb.const_i64(1, 5);
                fb.const_i64(2, ptr);
                fb.const_i64(3, len);
                fb.call(r, 0);
                fb.ret(0);
            }, &mut env);
            assert_eq!(exit, Exit::Trap(Trap::OutOfBounds), "ptr {ptr} len {len}");
        }
    }

    #[test]
    fn fs_escape_attempts_are_denied() {
        let d = TempDir::new("esc");
        let root = d.0.join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(d.0.join("secret.txt"), b"secret").unwrap();
        let o = opts(&root);
        for p in ["../secret.txt", "../x", "a/../../secret.txt", "sub/../b", "..", "/etc/passwd", "", "/"] {
            assert_eq!(fs_read(&o, p, 16).0, -2, "read {p:?}");
            assert_eq!(fs_write(&o, p, b"x"), -2, "write {p:?}");
        }
        assert!(!d.0.join("x").exists());
        assert_eq!(std::fs::read(d.0.join("secret.txt")).unwrap(), b"secret");
        let abs = d.0.join("secret.txt");
        assert_eq!(fs_read(&o, abs.to_str().unwrap(), 16).0, -2);
        assert_eq!(fs_write(&o, abs.to_str().unwrap(), b"x"), -2);
    }

    #[cfg(unix)]
    #[test]
    fn fs_symlinks_out_of_the_root_are_denied() {
        use std::os::unix::fs::symlink;
        let d = TempDir::new("sym");
        let root = d.0.join("root");
        let outside = d.0.join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
        symlink(&outside, root.join("link")).unwrap();
        symlink(outside.join("secret.txt"), root.join("file_link")).unwrap();
        symlink(outside.join("new.txt"), root.join("dangling")).unwrap();
        let o = opts(&root);
        assert_eq!(fs_read(&o, "link/secret.txt", 16).0, -2);
        assert_eq!(fs_read(&o, "file_link", 16).0, -2);
        assert_eq!(fs_write(&o, "link/new.txt", b"x"), -2);
        assert_eq!(fs_write(&o, "file_link", b"x"), -2);
        assert_eq!(fs_write(&o, "dangling", b"x"), -2);
        assert_eq!(std::fs::read(outside.join("secret.txt")).unwrap(), b"secret");
        assert!(!outside.join("new.txt").exists());
        // a symlink that stays inside the root is fine
        std::fs::write(root.join("real.txt"), b"in").unwrap();
        symlink(root.join("real.txt"), root.join("inner")).unwrap();
        assert_eq!(fs_read(&o, "inner", 16), (2, b"in".to_vec()));
    }
}
