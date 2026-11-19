//! A loaded `.efmod` package (MOD_PACKAGE.md sections 2, 7).

use std::collections::{BTreeMap, HashSet};

use crate::error::PackageError;
use crate::limits::ReadLimits;
use crate::manifest::Manifest;
use crate::container;

const ASSETS: &str = "assets/";

/// A read package: the manifest, the module (decoded and verified) and the assets.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub module: efevm::Module,
    /// The bytes of `module.efvm` as stored.
    pub module_bytes: Vec<u8>,
    /// Asset path (without the `assets/` prefix) to content.
    pub assets: BTreeMap<String, Vec<u8>>,
    /// Entries that are neither the manifest, the module nor an asset.
    pub ignored: Vec<String>,
}

/// `ns` equals `prefix` or continues it with a dot.
fn covers(prefix: &str, ns: &str) -> bool {
    ns == prefix || (ns.len() > prefix.len() && ns.starts_with(prefix) && ns.as_bytes()[prefix.len()] == b'.')
}

/// The manifest-against-module rules (section 3): no import from the mod's own namespace,
/// and every imported namespace is a requested prefix or a dependency's namespace.
pub fn check_module(manifest: &Manifest, module: &efevm::Module) -> Result<(), PackageError> {
    let mut seen = HashSet::new();
    let mut undeclared: Vec<String> = Vec::new();
    for imp in &module.data().imports {
        let ns = imp.namespace.as_str();
        if !seen.insert(ns) {
            continue;
        }
        if covers(&manifest.id, ns) {
            return Err(PackageError::Manifest(format!(
                "the module imports `{ns}.{}` from its own namespace `{}`",
                imp.name, manifest.id
            )));
        }
        let declared = manifest.permissions.namespaces.iter().any(|p| covers(p, ns))
            || manifest.dependencies.iter().any(|d| covers(&d.id, ns));
        if !declared {
            undeclared.push(ns.to_string());
        }
    }
    if undeclared.is_empty() {
        return Ok(());
    }
    undeclared.sort();
    undeclared.dedup();
    Err(PackageError::UndeclaredImport { namespaces: undeclared })
}

impl Package {
    /// Reads a package, checking everything in the order of section 7.
    pub fn read(bytes: &[u8], limits: &ReadLimits) -> Result<Package, PackageError> {
        let entries = container::read(bytes, limits)?;
        let mut manifest_data: Option<Vec<u8>> = None;
        let mut module_data: Option<Vec<u8>> = None;
        let mut assets = BTreeMap::new();
        let mut ignored = Vec::new();
        for e in entries {
            match e.name.as_str() {
                "manifest.json" => manifest_data = Some(e.data),
                "module.efvm" => module_data = Some(e.data),
                n if n.starts_with(ASSETS) && n.len() > ASSETS.len() => {
                    assets.insert(n[ASSETS.len()..].to_string(), e.data);
                }
                _ => ignored.push(e.name),
            }
        }
        let manifest_data = manifest_data.ok_or(PackageError::Missing("manifest.json"))?;
        let module_bytes = module_data.ok_or(PackageError::Missing("module.efvm"))?;
        let text = std::str::from_utf8(&manifest_data)
            .map_err(|_| PackageError::Manifest("manifest.json is not valid UTF-8".into()))?;
        let manifest = Manifest::from_json(text)?;
        let module = efevm::Module::decode(&module_bytes).map_err(PackageError::Module)?;
        check_module(&manifest, &module)?;
        Ok(Package { manifest, module, module_bytes, assets, ignored })
    }

    /// Writes the canonical archive. Validates the manifest and the asset names; the caller
    /// checks the module against the manifest with [`check_module`]. Needs the `write`
    /// feature.
    #[cfg(feature = "write")]
    pub fn write(
        manifest: &Manifest,
        module_bytes: &[u8],
        assets: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, PackageError> {
        manifest.validate()?;
        let limits = ReadLimits::default();
        let json = manifest.to_json();
        let mut names: Vec<(String, &Vec<u8>)> = Vec::new();
        let mut lower = HashSet::new();
        lower.insert("manifest.json".to_string());
        lower.insert("module.efvm".to_string());
        // BTreeMap iterates in byte order of the asset path, which is the order of the full
        // names since the prefix is shared.
        for (path, data) in assets {
            let full = format!("{ASSETS}{path}");
            let bad = |reason: String| PackageError::BadEntryName { name: full.clone(), reason };
            if path.is_empty() {
                return Err(bad("empty asset path".into()));
            }
            container::validate_entry_name(&full, &limits).map_err(bad)?;
            if path.ends_with('/') {
                return Err(bad("ends with `/`".into()));
            }
            if !lower.insert(full.to_ascii_lowercase()) {
                return Err(bad("duplicate name ignoring case".into()));
            }
            names.push((full, data));
        }
        let mut list: Vec<(&str, &[u8])> = vec![("manifest.json", json.as_bytes()), ("module.efvm", module_bytes)];
        for (n, d) in &names {
            list.push((n.as_str(), d.as_slice()));
        }
        Ok(container::write(&list))
    }

    /// The mod's own API index: the module's interface with the manifest's id,
    /// `api_version` and namespace doc (just the namespace when there is no interface).
    pub fn index(&self) -> efevm::ApiIndex {
        let doc = self.manifest.namespace_doc();
        let id = &self.manifest.id;
        let v = self.manifest.api_version;
        match &self.module.data().interface {
            Some(i) => i.index(id, v, &doc),
            None => efevm::Interface::default().index(id, v, &doc),
        }
    }

    /// An asset by path (without the `assets/` prefix).
    pub fn asset(&self, path: &str) -> Option<&[u8]> {
        self.assets.get(path).map(|v| v.as_slice())
    }
}
