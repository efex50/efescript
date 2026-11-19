//! Host interface: output and sound for the std host namespace.
//!
//! Reading captured data back in tests: `CaptureHost` is shared through an
//! `Rc<RefCell<CaptureHost>>`, which itself implements `Host`:
//!
//! ```
//! use efescript::prelude::*;
//! let cap = CaptureHost::shared();
//! let mut h = cap.clone();
//! h.write_out(b"hi");
//! assert_eq!(cap.borrow().out_string(), "hi");
//! ```

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

pub trait Host {
    /// Print / println output.
    fn write_out(&mut self, bytes: &[u8]);
    /// PlaySound. `Err` carries the error text.
    fn play_sound(&mut self, path: &str) -> Result<(), String>;
}

/// Writes to the process stdout. Sound needs the `audio` cargo feature.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdHost;

impl Host for StdHost {
    fn write_out(&mut self, bytes: &[u8]) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }

    #[cfg(feature = "audio")]
    fn play_sound(&mut self, path: &str) -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let (_stream, handle) = rodio::OutputStream::try_default().map_err(|e| e.to_string())?;
        let sink = rodio::Sink::try_new(&handle).map_err(|e| e.to_string())?;
        let source = rodio::Decoder::new(std::io::BufReader::new(file))
            .map_err(|e| format!("{path}: {e}"))?;
        sink.append(source);
        sink.sleep_until_end();
        Ok(())
    }

    #[cfg(not(feature = "audio"))]
    fn play_sound(&mut self, _path: &str) -> Result<(), String> {
        Err("efescript was built without the `audio` feature".to_string())
    }
}

/// Records everything instead of touching the outside world (for tests).
#[derive(Debug, Default, Clone)]
pub struct CaptureHost {
    pub out: Vec<u8>,
    pub sounds: Vec<String>,
}

impl CaptureHost {
    pub fn new() -> Self {
        Self::default()
    }
    /// A shared handle: pass `Box::new(handle.clone())` to `ProgramRuntime::with_host`
    /// and read `handle.borrow()` after the run.
    pub fn shared() -> Rc<RefCell<CaptureHost>> {
        Rc::new(RefCell::new(CaptureHost::default()))
    }
    /// Captured output as text (lossy).
    pub fn out_string(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }
}

impl Host for CaptureHost {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }
    fn play_sound(&mut self, path: &str) -> Result<(), String> {
        self.sounds.push(path.to_string());
        Ok(())
    }
}

impl<H: Host> Host for Rc<RefCell<H>> {
    fn write_out(&mut self, bytes: &[u8]) {
        self.borrow_mut().write_out(bytes);
    }
    fn play_sound(&mut self, path: &str) -> Result<(), String> {
        self.borrow_mut().play_sound(path)
    }
}
