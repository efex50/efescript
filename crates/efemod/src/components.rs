//! Mod-defined component declarations (`[[meta.component]]`): parsing and `repr(C)` layout.
//! Shared by the engine (registration) and the build tools (the mod's `.efapi` fragment).

use efevm::api::json::Value;
use efevm::{FieldDesc, Prim, StructDesc};

/// Most components one mod may declare, whatever the grant says.
pub const MAX_COMPONENTS_PER_MOD: usize = 256;
/// Most fields one component may have.
pub const MAX_FIELDS: usize = 64;
/// Largest size of one component, whatever the grant says.
pub const MAX_COMPONENT_BYTES: u32 = 64 * 1024;

/// One component a mod declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModComponent {
    /// The name as declared (`Health`).
    pub name: String,
    /// Whether change ticks are recorded (`tracked`, default true).
    pub tracked: bool,
    /// The layout: name `mod:<mod>/<Name>`, owner `mod:<mod>`, `repr(C)` offsets.
    pub desc: StructDesc,
}

impl ModComponent {
    /// Size in bytes.
    pub fn size(&self) -> u32 {
        self.desc.size
    }
    /// Alignment in bytes.
    pub fn align(&self) -> u32 {
        self.desc.align
    }
    /// The id scripts and Rust use: `fnv1a32` of the full name.
    pub fn id(&self) -> u32 {
        efevm::api::fnv1a32(self.desc.name.as_bytes())
    }
}

/// `mod:<mod>/<component>`.
pub fn component_name(mod_id: &str, component: &str) -> String {
    format!("mod:{mod_id}/{component}")
}

/// Why a `[[meta.component]]` declaration is refused. `component` is empty when the whole
/// `meta.component` entry is at fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentError {
    pub component: String,
    pub reason: String,
}

fn bad<T>(_id: &str, component: &str, reason: impl Into<String>) -> Result<T, ComponentError> {
    Err(ComponentError { component: component.to_string(), reason: reason.into() })
}

/// Parse and lay out the components of `meta.component`. Nothing is registered.
pub fn parse_components_raw(mod_id: &str, meta: Option<&Value>) -> Result<Vec<ModComponent>, ComponentError> {
    let Some(list) = meta.and_then(|m| m.get("component")) else { return Ok(Vec::new()) };
    let Some(list) = list.as_array() else {
        return bad(mod_id, "", "`meta.component` must be an array of tables (`[[meta.component]]`)");
    };
    if list.len() > MAX_COMPONENTS_PER_MOD {
        return bad(mod_id, "", format!("{} components declared, at most {MAX_COMPONENTS_PER_MOD}", list.len()));
    }
    let mut out: Vec<ModComponent> = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let Some(entries) = c.entries() else { return bad(mod_id, &format!("#{i}"), "a component must be a table") };
        let name = match c.get("name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            None => return bad(mod_id, &format!("#{i}"), "`name` is missing or not a string"),
        };
        if !efevm::api::is_ident(&name) {
            return bad(mod_id, &name, "the name is not an identifier");
        }
        if out.iter().any(|o| o.name == name) {
            return bad(mod_id, &name, "declared twice");
        }
        for (k, _) in entries {
            if !["name", "doc", "fields", "tracked"].contains(&k.as_str()) {
                return bad(mod_id, &name, format!("unknown key `{k}` (known: name, doc, fields, tracked)"));
            }
        }
        let doc = match c.get("doc") {
            None => String::new(),
            Some(v) => match v.as_str() {
                Some(s) => s.to_string(),
                None => return bad(mod_id, &name, "`doc` must be a string"),
            },
        };
        let tracked = match c.get("tracked") {
            None => true,
            Some(v) => match v.as_bool() {
                Some(b) => b,
                None => return bad(mod_id, &name, "`tracked` must be true or false"),
            },
        };
        let Some(fields) = c.get("fields").and_then(Value::as_array) else {
            return bad(mod_id, &name, "`fields` is missing or not an array");
        };
        if fields.is_empty() {
            return bad(mod_id, &name, "a component needs at least one field");
        }
        if fields.len() > MAX_FIELDS {
            return bad(mod_id, &name, format!("{} fields, at most {MAX_FIELDS}", fields.len()));
        }
        let full = component_name(mod_id, &name);
        let mut desc = StructDesc::new(full, 0, 1).doc(doc);
        desc.owner = format!("mod:{mod_id}");
        let (mut offset, mut align) = (0u64, 1u32);
        for (j, f) in fields.iter().enumerate() {
            let Some(fe) = f.entries() else { return bad(mod_id, &name, format!("field #{j} must be a table")) };
            let Some(fname) = f.get("name").and_then(Value::as_str) else {
                return bad(mod_id, &name, format!("field #{j}: `name` is missing or not a string"));
            };
            for (k, _) in fe {
                if !["name", "type", "count", "doc"].contains(&k.as_str()) {
                    return bad(mod_id, &name, format!("field `{fname}`: unknown key `{k}` (known: name, type, count, doc)"));
                }
            }
            if !efevm::api::is_ident(fname) {
                return bad(mod_id, &name, format!("field name `{fname}` is not an identifier"));
            }
            if desc.fields.iter().any(|g| g.name == fname) {
                return bad(mod_id, &name, format!("duplicate field `{fname}`"));
            }
            let Some(tname) = f.get("type").and_then(Value::as_str) else {
                return bad(mod_id, &name, format!("field `{fname}`: `type` is missing or not a string"));
            };
            let Some(ty) = Prim::parse(tname) else {
                let known: Vec<&str> = Prim::ALL.iter().map(|p| p.name()).collect();
                return bad(mod_id, &name, format!("field `{fname}`: unknown type `{tname}` (known: {})", known.join(", ")));
            };
            let count = match f.get("count") {
                None => 1,
                Some(v) => match v.as_u64() {
                    Some(n) if (1..=u32::MAX as u64).contains(&n) => n as u32,
                    _ => return bad(mod_id, &name, format!("field `{fname}`: `count` must be an integer of at least 1")),
                },
            };
            let fdoc = match f.get("doc") {
                None => String::new(),
                Some(v) => match v.as_str() {
                    Some(s) => s.to_string(),
                    None => return bad(mod_id, &name, format!("field `{fname}`: `doc` must be a string")),
                },
            };
            let tsize = ty.size() as u64;
            offset = offset.div_ceil(tsize) * tsize;
            let end = offset + tsize * count as u64;
            if end > MAX_COMPONENT_BYTES as u64 {
                return bad(mod_id, &name, format!("larger than {MAX_COMPONENT_BYTES} bytes"));
            }
            desc.fields.push(FieldDesc::new(fname, offset as u32, ty, count).with_doc(fdoc));
            offset = end;
            align = align.max(tsize as u32);
        }
        let size = (offset as u32).div_ceil(align) * align;
        desc.size = size;
        desc.align = align;
        if let Err(e) = desc.validate() {
            return bad(mod_id, &name, e);
        }
        out.push(ModComponent { name, tracked, desc });
    }
    Ok(out)
}


/// The API index fragment of a mod, built from its declaration without running anything: the
/// namespace `<id>` (version `api_version`, the manifest's namespace doc) with the module's
/// provides, events and own structs (what the engine registers when the mod loads), plus every
/// `[[meta.component]]` as a struct `mod:<id>/<Name>` owned by `mod:<id>` carrying its component
/// id. `game` is the mod id. Merge it with the game's index (`ApiIndex::merge`) to check scripts
/// that use the mod.
pub fn api_fragment(manifest: &crate::Manifest, module: &efevm::Module) -> Result<efevm::ApiIndex, ComponentError> {
    let comps = parse_components_raw(&manifest.id, manifest.meta.as_ref())?;
    let doc = manifest.namespace_doc();
    let mut ix = match &module.data().interface {
        Some(i) => i.index(&manifest.id, manifest.api_version, &doc),
        None => efevm::Interface::default().index(&manifest.id, manifest.api_version, &doc),
    };
    for c in comps {
        let mut d = c.desc.clone();
        d.component = Some(c.id());
        ix.types.push(d);
    }
    ix.sort();
    Ok(ix)
}
