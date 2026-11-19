//! Section 13 `interface` as API data (SPEC.md 9.2, crates/efescript/docs/e8-design.md 5):
//! the host function descriptions an embedder registers for a mod's provides, and the mod's
//! own API index. Pure data, no I/O.

use crate::api::index::ApiIndex;
use crate::api::types::{ApiType, EventDesc, HostFn, NamespaceInfo, StructDesc};
use crate::module::{Interface, Provide, ProvidedEvent};
use crate::types::FuncType;

impl Provide {
    /// The lowered VM signature: `ApiType::lower` of the parameters, then of the results.
    /// A verified module's provided function has exactly this type (SPEC.md 9.2 rule 4).
    pub fn func_type(&self) -> FuncType {
        FuncType::new(
            self.params.iter().flat_map(|p| p.ty.lower().iter().copied()),
            self.results.iter().flat_map(|p| p.ty.lower().iter().copied()),
        )
    }

    /// `true` if a call needs the scratch region: some parameter is `str`, `bytes` or a struct.
    pub fn needs_scratch(&self) -> bool {
        self.params.iter().any(|p| !p.ty.is_scalar())
    }

    /// The host function description of this provide in `namespace`. `owner` is left empty:
    /// `HostRegistry::register` fills it from the namespace.
    pub fn host_fn(&self, namespace: &str) -> HostFn {
        HostFn {
            namespace: namespace.to_string(),
            name: self.name.clone(),
            doc: self.doc.clone(),
            params: self.params.clone(),
            results: self.results.clone(),
            owner: String::new(),
        }
    }
}

impl ProvidedEvent {
    /// The event description of this event in `namespace`. `owner` is left empty:
    /// `HostRegistry::register_event` fills it from the namespace.
    pub fn event_desc(&self, namespace: &str) -> EventDesc {
        EventDesc {
            namespace: namespace.to_string(),
            name: self.name.clone(),
            payload: self.payload.clone(),
            doc: self.doc.clone(),
            owner: String::new(),
        }
    }
}

impl Interface {
    /// The provided event called `name`.
    pub fn event(&self, name: &str) -> Option<&ProvidedEvent> {
        self.events.iter().find(|e| e.name == name)
    }

    /// One event description per provided event, in section order, all in `namespace`, with
    /// an empty `owner` (filled by `HostRegistry::register_event`). The embedder registers
    /// them when the mod loads, after the mod's own structs.
    pub fn event_descs(&self, namespace: &str) -> Vec<EventDesc> {
        self.events.iter().map(|e| e.event_desc(namespace)).collect()
    }

    /// The provide called `name`.
    pub fn provide(&self, name: &str) -> Option<&Provide> {
        self.provides.iter().find(|p| p.name == name)
    }

    /// The struct layout called `name` (own or copied).
    pub fn struct_desc(&self, name: &str) -> Option<&StructDesc> {
        self.structs.iter().find(|s| s.name == name)
    }

    /// `true` if any provide has a `str`, `bytes` or struct parameter (then a verified module
    /// has a scratch region of at least one byte).
    pub fn needs_scratch(&self) -> bool {
        self.provides.iter().any(Provide::needs_scratch)
    }

    /// The structs this module defines itself (empty `owner`).
    pub fn own_structs(&self) -> impl Iterator<Item = &StructDesc> {
        self.structs.iter().filter(|s| s.owner.is_empty())
    }

    /// The copies of structs defined elsewhere (non-empty `owner`), which the embedder checks
    /// against its registry at load.
    pub fn copied_structs(&self) -> impl Iterator<Item = &StructDesc> {
        self.structs.iter().filter(|s| !s.owner.is_empty())
    }

    /// One host function description per provide, in section order, all in `namespace`, with
    /// an empty `owner` (filled by `HostRegistry::register`). These are what the embedder
    /// registers its trampolines with when the mod loads.
    pub fn host_fns(&self, namespace: &str) -> Vec<HostFn> {
        self.provides.iter().map(|p| p.host_fn(namespace)).collect()
    }

    /// The mod's own API index: one namespace `id` (version `version`, owner `id`, doc `doc`),
    /// the provides as functions of `id` owned by `id`, the provided events as events of `id`
    /// owned by `id`, own structs owned by `id` and copied structs with their recorded owner.
    /// `game` is `id`. Sorted like a registry index, so for
    /// a verified module with a valid `id` it passes `ApiIndex::validate` and
    /// `ApiIndex::from_json(&ix.to_json()) == Ok(ix)`.
    pub fn index(&self, id: &str, version: u32, doc: &str) -> ApiIndex {
        let types = self
            .structs
            .iter()
            .map(|s| {
                let mut s = s.clone();
                if s.owner.is_empty() {
                    s.owner = id.to_string();
                }
                s
            })
            .collect();
        let functions = self
            .provides
            .iter()
            .map(|p| {
                let mut f = p.host_fn(id);
                f.owner = id.to_string();
                f
            })
            .collect();
        let events = self
            .events
            .iter()
            .map(|e| {
                let mut d = e.event_desc(id);
                d.owner = id.to_string();
                d
            })
            .collect();
        let mut ix = ApiIndex {
            game: id.to_string(),
            namespaces: vec![NamespaceInfo {
                name: id.to_string(),
                version,
                owner: id.to_string(),
                doc: doc.to_string(),
            }],
            types,
            functions,
            events,
        };
        ix.sort();
        ix
    }
}

/// `true` if `t` may cross a module boundary: everything but `ptr`, which means nothing in
/// another module's memory (SPEC.md 9.2).
pub(crate) fn crosses_modules(t: &ApiType) -> bool {
    !matches!(t, ApiType::Ptr)
}
