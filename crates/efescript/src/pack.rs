//! `efescript pack`: a mod folder (`mod.toml`, sources, assets) to a `.efmod` package
//! (MOD_PACKAGE.md section 4, e8-design.md sections 8 and 10).
//!
//! The functions here are the library side of the `pack`, `inspect` and `api-export`
//! commands, so tests and build tools can pack without spawning the binary.

use std::path::Path;

use efa::asm::FsSource;
use efa::mod_folder::{build_mod, list_assets, BuildLimits};
use efemod::{Manifest, Package, ReadLimits};
pub use efevm::api::ApiIndex;

pub use efa::mod_folder::{mod_index, read_mod_toml, BuildToml, ModToml};

/// The result of [`pack_dir`].
#[derive(Debug)]
pub struct Packed {
    pub manifest: Manifest,
    pub module: efevm::Module,
    /// The `.efmod` file.
    pub bytes: Vec<u8>,
    /// `<id>-<version>.efmod`.
    pub file_name: String,
    /// The mod's own API index fragment (`<id>.efapi`): its namespace, provides, events, structs
    /// and `[[meta.component]]` components, built from the declaration (see [`efemod::api_fragment`]).
    pub api: ApiIndex,
}

/// Packs the mod folder `dir`: reads `dir/mod.toml`, assembles `[build] entry` against the
/// merge of `extra_apis` and the `[build] api` files, checks the manifest against the module,
/// collects the assets and writes the canonical package.
pub fn pack_dir(dir: &Path, extra_apis: &[&ApiIndex]) -> Result<Packed, String> {
    let toml_path = dir.join("mod.toml");
    let text = std::fs::read_to_string(&toml_path).map_err(|e| format!("{}: {e}", toml_path.display()))?;
    let mt = read_mod_toml(&text)?;
    mt.manifest.validate().map_err(|e| format!("mod.toml: {e}"))?;

    // Inputs: the given indices, then [build] api (relative to mod.toml).
    let mut owned: Vec<ApiIndex> = Vec::new();
    for a in &mt.build.api {
        efa::mod_folder::inside(a, "api")?;
        owned.push(read_api_input(&dir.join(a))?);
    }
    let mut refs: Vec<&ApiIndex> = extra_apis.to_vec();
    refs.extend(owned.iter());
    let merged = if refs.is_empty() { None } else { Some(ApiIndex::merge(&refs).map_err(|e| e.to_string())?) };

    let assets = list_assets(dir)?;
    let src = FsSource::with_prefix(dir, "");
    let built = build_mod(&src, merged.as_ref(), &assets, &BuildLimits::default())?;

    let module_bytes = built.module.encode();
    let bytes = Package::write(&built.manifest, &module_bytes, &built.assets).map_err(|e| e.to_string())?;
    let file_name = format!("{}-{}.efmod", built.manifest.id, built.manifest.version);
    let api = efemod::api_fragment(&built.manifest, &built.module)
        .map_err(|e| format!("mod `{}`: component `{}`: {}", built.manifest.id, e.component, e.reason))?;
    Ok(Packed { manifest: built.manifest, module: built.module, bytes, file_name, api })
}

/// Reads an API input: `.efapi` (JSON index) or `.efmod` (the package's own index). Other
/// extensions are told apart by content (the 7z signature, or JSON).
pub fn read_api_input(path: &Path) -> Result<ApiIndex, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let is_pkg = match ext {
        "efmod" => true,
        "efapi" => false,
        _ if bytes.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) => true,
        _ if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') => false,
        _ => return Err(format!("{}: not an .efapi or .efmod file", path.display())),
    };
    if is_pkg {
        let pkg = Package::read(&bytes, &ReadLimits::default()).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(pkg.index())
    } else {
        let text = std::str::from_utf8(&bytes).map_err(|_| format!("{}: not valid UTF-8", path.display()))?;
        ApiIndex::from_json(text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

