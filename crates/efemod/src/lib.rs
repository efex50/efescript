//! Mod packages (`.efmod`) for efeengine, specified in `crates/efevm/MOD_PACKAGE.md`.
//!
//! A package is a 7z archive in a restricted profile (section 1): one folder per entry,
//! LZMA2 or COPY, a plain header, no encryption. The crate does no I/O and builds for
//! `wasm32-unknown-unknown`; it reads with `sevenz-rust2` (no AES, bzip2 or PPMd) after
//! checking the archive's structure itself. It provides:
//!
//! - [`Package`]: [`Package::read`] checks bytes in the order of section 7 and gives the
//!   manifest, the decoded module, the assets and the ignored entries; [`Package::write`]
//!   writes the canonical archive; [`Package::index`] is the mod's own API index.
//! - [`Manifest`] (`manifest.json`, section 3) with [`Permissions`] and [`check_module`],
//!   the manifest-against-module rules.
//! - [`Version`] and [`VersionReq`] (SemVer and ranges, section 5).
//! - [`resolve()`]: dependency resolution and load order over [`Candidate`]s.
//! - [`container`] (the 7z profile, sections 1, 2), [`ReadLimits`] (section 6) and
//!   [`PackageError`].
//!
//! # Features
//!
//! - `write` (off by default): the canonical writer, [`Package::write`] and
//!   `container::write`, with the LZMA2 encoder. The packer enables it
//!   (`efemod = { path = "../efemod", features = ["write"] }`); the engine does not, so its
//!   build has the decoder only.

#![forbid(unsafe_code)]

pub mod components;
pub mod container;
pub mod error;
pub mod limits;
pub mod manifest;
pub mod package;
pub mod resolve;
pub mod version;

pub use version::{Version, VersionReq, VersionError};
pub use error::PackageError;
pub use limits::ReadLimits;
pub use resolve::{resolve, Candidate, Dependency, Resolution, ResolveProblem};
pub use manifest::{Manifest, Permissions, MAX_META_BYTES, MAX_META_DEPTH};
pub use package::{check_module, Package};
pub use components::api_fragment;
