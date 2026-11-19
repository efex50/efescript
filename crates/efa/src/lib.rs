//! The lean `.efa` compiler: assembler, linker and mod-folder builder. No file system or
//! package writer is needed (the native helpers are `cfg`-gated), so it builds for wasm.

pub mod asm;
pub mod link;
pub mod mod_folder;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
