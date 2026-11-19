//! `HostRegistry`: the only way to make a host function callable (crates/efescript/docs/e3-design.md
//! section 2). Registering a function records its description next to the function itself, so
//! [`HostRegistry::imports`] (what modules link against) and [`HostRegistry::index`] (the API
//! index file) are two views of one table.

use crate::api::index::ApiIndex;
use crate::api::types::{ApiType, EventDesc, HostFn, NamespaceInfo, StructDesc, is_ident, is_namespace};
use crate::imports::{Caller, HostFunc, HostOk, Imports};
use crate::trap::Trap;
use crate::types::FuncType;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// A view of guest memory passed as a `str` or `bytes` parameter: `(ptr, len)` as the two raw
/// `i64` slots (bit patterns). Read it with [`Caller::read_bytes`] / [`Caller::read_str`];
/// negative values (bit 63 set) are always out of bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct GuestSlice {
    pub ptr: u64,
    pub len: u64,
}

impl GuestSlice {
    pub fn new(ptr: u64, len: u64) -> Self {
        GuestSlice { ptr, len }
    }
}

/// A registered function's description plus the slot of each logical parameter.
pub(crate) struct Sig {
    desc: HostFn,
    slots: Vec<usize>,
}

impl Sig {
    fn new(desc: HostFn) -> Sig {
        let slots = desc.param_slots();
        Sig { desc, slots }
    }

    fn bad(&self, what: &str, i: usize, want: &str) -> Trap {
        Trap::Host(format!("{}: {what} {i} is not {want}", self.desc.qualified()))
    }
}

/// The arguments of one host call, read by *logical* parameter index (a `str` parameter is
/// one index, two VM slots). Every accessor checks the index and the declared type and returns
/// `Err(Trap::Host)` on a mismatch; nothing here panics.
pub struct Args<'a> {
    sig: &'a Sig,
    raw: &'a [u64],
}

impl<'a> Args<'a> {
    /// The declared function.
    pub fn desc(&self) -> &HostFn {
        &self.sig.desc
    }

    /// Number of logical parameters.
    pub fn len(&self) -> usize {
        self.sig.desc.params.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sig.desc.params.is_empty()
    }

    /// The raw lowered argument slots (`func_type().params` order).
    pub fn raw(&self) -> &[u64] {
        self.raw
    }

    fn slot(&self, i: usize, ok: fn(&ApiType) -> bool, want: &str) -> Result<usize, Trap> {
        match (self.sig.desc.params.get(i), self.sig.slots.get(i)) {
            (Some(p), Some(&s)) if ok(&p.ty) => Ok(s),
            _ => Err(self.sig.bad("parameter", i, want)),
        }
    }

    fn word(&self, i: usize, ok: fn(&ApiType) -> bool, want: &str) -> Result<u64, Trap> {
        let s = self.slot(i, ok, want)?;
        self.raw.get(s).copied().ok_or_else(|| self.sig.bad("parameter", i, want))
    }

    pub fn i32(&self, i: usize) -> Result<i32, Trap> {
        self.word(i, |t| *t == ApiType::I32, "i32").map(|v| v as u32 as i32)
    }

    pub fn i64(&self, i: usize) -> Result<i64, Trap> {
        self.word(i, |t| *t == ApiType::I64, "i64").map(|v| v as i64)
    }

    pub fn f32(&self, i: usize) -> Result<f32, Trap> {
        self.word(i, |t| *t == ApiType::F32, "f32").map(|v| f32::from_bits(v as u32))
    }

    pub fn f64(&self, i: usize) -> Result<f64, Trap> {
        self.word(i, |t| *t == ApiType::F64, "f64").map(f64::from_bits)
    }

    /// Any non-zero low 32 bits is `true`.
    pub fn bool(&self, i: usize) -> Result<bool, Trap> {
        self.word(i, |t| *t == ApiType::Bool, "bool").map(|v| v as u32 != 0)
    }

    /// The entity handle bits (`Entity::to_bits`).
    pub fn entity(&self, i: usize) -> Result<u64, Trap> {
        self.word(i, |t| *t == ApiType::Entity, "entity")
    }

    /// A `ptr` or `&Struct` parameter.
    pub fn ptr(&self, i: usize) -> Result<u64, Trap> {
        self.word(i, |t| matches!(t, ApiType::Ptr | ApiType::Struct(_)), "ptr")
    }

    fn slice(&self, i: usize, ok: fn(&ApiType) -> bool, want: &str) -> Result<GuestSlice, Trap> {
        let s = self.slot(i, ok, want)?;
        match (self.raw.get(s), s.checked_add(1).and_then(|e| self.raw.get(e))) {
            (Some(&ptr), Some(&len)) => Ok(GuestSlice { ptr, len }),
            _ => Err(self.sig.bad("parameter", i, want)),
        }
    }

    pub fn str(&self, i: usize) -> Result<GuestSlice, Trap> {
        self.slice(i, |t| *t == ApiType::Str, "str")
    }

    pub fn bytes(&self, i: usize) -> Result<GuestSlice, Trap> {
        self.slice(i, |t| *t == ApiType::Bytes, "bytes")
    }
}

/// The results of one host call, by logical index (results are always one slot each).
/// Results that are not set are 0.
pub struct Results<'a> {
    sig: &'a Sig,
    raw: &'a mut [u64],
}

impl<'a> Results<'a> {
    pub fn len(&self) -> usize {
        self.sig.desc.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sig.desc.results.is_empty()
    }

    /// The raw lowered result slots.
    pub fn raw(&mut self) -> &mut [u64] {
        self.raw
    }

    fn put(&mut self, i: usize, ok: fn(&ApiType) -> bool, want: &str, v: u64) -> Result<(), Trap> {
        let fits = self.sig.desc.results.get(i).is_some_and(|p| ok(&p.ty));
        match self.raw.get_mut(i) {
            Some(slot) if fits => {
                *slot = v;
                Ok(())
            }
            _ => Err(self.sig.bad("result", i, want)),
        }
    }

    pub fn set_i32(&mut self, i: usize, v: i32) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::I32, "i32", v as u32 as u64)
    }

    pub fn set_i64(&mut self, i: usize, v: i64) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::I64, "i64", v as u64)
    }

    pub fn set_f32(&mut self, i: usize, v: f32) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::F32, "f32", v.to_bits() as u64)
    }

    pub fn set_f64(&mut self, i: usize, v: f64) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::F64, "f64", v.to_bits())
    }

    pub fn set_bool(&mut self, i: usize, v: bool) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::Bool, "bool", v as u64)
    }

    pub fn set_entity(&mut self, i: usize, v: u64) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::Entity, "entity", v)
    }

    pub fn set_ptr(&mut self, i: usize, v: u64) -> Result<(), Trap> {
        self.put(i, |t| *t == ApiType::Ptr, "ptr", v)
    }
}

/// Why the registry refused a definition. One variant per rule of e3-design.md section 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// A namespace name that is not identifiers joined by dots.
    InvalidNamespace(String),
    /// An empty owner for a namespace.
    EmptyOwner { namespace: String },
    /// `define_namespace` of a namespace that belongs to another owner.
    NamespaceOwner { namespace: String, owner: String, existing: String },
    /// `register`/`replace` into a namespace that was never defined.
    UnknownNamespace { function: String },
    /// A function name that is not an identifier.
    InvalidFunctionName { function: String },
    /// A function with an empty (or blank) doc.
    EmptyDoc { function: String },
    /// A parameter or result name that is not an identifier.
    InvalidParamName { function: String, name: String },
    /// Two parameters or results with the same name.
    DuplicateParam { function: String, name: String },
    /// A `str`, `bytes` or `&Struct` result (only scalars may be results).
    NonScalarResult { function: String, name: String },
    /// A `&Name` parameter of a struct that is not registered.
    UnknownStruct { function: String, name: String },
    /// `register` of a `namespace.name` that already exists (use `replace`).
    DuplicateFunction { function: String },
    /// `replace` of a function that does not exist.
    NoSuchFunction { function: String },
    /// `StructDesc::validate` failed.
    InvalidStruct(String),
    /// A struct name that is already registered.
    DuplicateStruct { name: String },
    /// A component id that another struct already uses.
    DuplicateComponent { id: u32, name: String, existing: String },
    /// `unregister_owner` would remove a struct that a function of another owner still uses.
    StructInUse { name: String, function: String },
    /// `register_event`/`replace_event` into a namespace that was never defined.
    UnknownEventNamespace { event: String },
    /// An event name that is not an identifier.
    InvalidEventName { event: String },
    /// An event with an empty (or blank) doc.
    EmptyEventDoc { event: String },
    /// An event payload naming a struct that is not registered.
    UnknownEventPayload { event: String, name: String },
    /// `register_event` of a `namespace.name` that already exists (use `replace_event`).
    DuplicateEvent { event: String },
    /// `replace_event` of an event that does not exist.
    NoSuchEvent { event: String },
    /// A function and an event may not have the same full name `namespace.name`.
    EventFunctionClash { name: String },
    /// `unregister_owner` would remove a struct that an event of another owner still carries.
    StructInUseByEvent { name: String, event: String },
    /// Registration is closed: the game loop already started (`Engine::api_open`). `what` names
    /// the refused item.
    Closed { what: String },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RegistryError as E;
        match self {
            E::Closed { what } => write!(f, "cannot register {what}: API registration is closed once the game loop starts; register in setup or Plugin::build (mods declare theirs in mod.toml)"),
            E::InvalidNamespace(n) => write!(f, "`{n}` is not a valid namespace name"),
            E::EmptyOwner { namespace } => write!(f, "namespace {namespace} has an empty owner"),
            E::NamespaceOwner { namespace, owner, existing } => {
                write!(f, "namespace {namespace} belongs to {existing}, not {owner}")
            }
            E::UnknownNamespace { function } => write!(f, "{function}: namespace is not defined"),
            E::InvalidFunctionName { function } => write!(f, "{function}: function name is not an identifier"),
            E::EmptyDoc { function } => write!(f, "{function}: doc is empty"),
            E::InvalidParamName { function, name } => {
                write!(f, "{function}: parameter or result name `{name}` is not an identifier")
            }
            E::DuplicateParam { function, name } => write!(f, "{function}: duplicate parameter or result `{name}`"),
            E::NonScalarResult { function, name } => {
                write!(f, "{function}: result `{name}` must be a scalar type")
            }
            E::UnknownStruct { function, name } => write!(f, "{function}: struct `{name}` is not registered"),
            E::DuplicateFunction { function } => write!(f, "{function} is already registered"),
            E::NoSuchFunction { function } => write!(f, "{function} is not registered"),
            E::InvalidStruct(msg) => f.write_str(msg),
            E::DuplicateStruct { name } => write!(f, "struct `{name}` is already registered"),
            E::DuplicateComponent { id, name, existing } => {
                write!(f, "struct `{name}`: component id {id} is already used by `{existing}`")
            }
            E::StructInUse { name, function } => {
                write!(f, "struct `{name}` is still used by {function}, which has another owner")
            }
            E::UnknownEventNamespace { event } => write!(f, "event {event}: namespace is not defined"),
            E::InvalidEventName { event } => write!(f, "event {event}: name is not an identifier"),
            E::EmptyEventDoc { event } => write!(f, "event {event}: doc is empty"),
            E::UnknownEventPayload { event, name } => {
                write!(f, "event {event}: payload struct `{name}` is not registered")
            }
            E::DuplicateEvent { event } => write!(f, "event {event} is already registered"),
            E::NoSuchEvent { event } => write!(f, "event {event} is not registered"),
            E::EventFunctionClash { name } => write!(f, "{name} is both a function and an event"),
            E::StructInUseByEvent { name, event } => {
                write!(f, "struct `{name}` is still the payload of event {event}, which has another owner")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

struct Entry<H> {
    sig: Arc<Sig>,
    ty: FuncType,
    raw: HostFunc<H>,
}

/// The host functions, namespaces and struct layouts an embedder offers to scripts.
///
/// - [`imports`](Self::imports) is the only producer of a non-empty [`Imports`], so whatever a
///   module can call was registered here, with a doc.
/// - [`index`](Self::index) is the same table as plain data ([`ApiIndex`]); its `game` field
///   is the name set with [`set_game`](Self::set_game) (default `""`).
pub struct HostRegistry<H> {
    game: String,
    namespaces: BTreeMap<String, NamespaceInfo>,
    functions: BTreeMap<(String, String), Entry<H>>,
    types: BTreeMap<String, StructDesc>,
    events: BTreeMap<(String, String), EventDesc>,
}

impl<H: 'static> Default for HostRegistry<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: 'static> HostRegistry<H> {
    pub fn new() -> Self {
        HostRegistry {
            game: String::new(),
            namespaces: BTreeMap::new(),
            functions: BTreeMap::new(),
            types: BTreeMap::new(),
            events: BTreeMap::new(),
        }
    }

    /// The `game` name written into [`index`](Self::index) (not hashed).
    pub fn set_game(&mut self, game: impl Into<String>) {
        self.game = game.into();
    }

    pub fn game(&self) -> &str {
        &self.game
    }

    /// Defines a namespace, or updates the version and doc of one with the same owner.
    pub fn define_namespace(
        &mut self,
        name: &str,
        version: u32,
        owner: &str,
        doc: &str,
    ) -> Result<(), RegistryError> {
        if !is_namespace(name) {
            return Err(RegistryError::InvalidNamespace(name.to_string()));
        }
        if owner.is_empty() {
            return Err(RegistryError::EmptyOwner { namespace: name.to_string() });
        }
        if let Some(ns) = self.namespaces.get_mut(name) {
            if ns.owner != owner {
                return Err(RegistryError::NamespaceOwner {
                    namespace: name.to_string(),
                    owner: owner.to_string(),
                    existing: ns.owner.clone(),
                });
            }
            ns.version = version;
            ns.doc = doc.to_string();
            return Ok(());
        }
        let info = NamespaceInfo { name: name.into(), version, owner: owner.into(), doc: doc.into() };
        self.namespaces.insert(name.to_string(), info);
        Ok(())
    }

    /// Checks `f` against the registration rules and fills in its owner.
    fn check(&self, f: &mut HostFn) -> Result<(), RegistryError> {
        let function = f.qualified();
        let ns = self
            .namespaces
            .get(&f.namespace)
            .ok_or_else(|| RegistryError::UnknownNamespace { function: function.clone() })?;
        if !is_ident(&f.name) {
            return Err(RegistryError::InvalidFunctionName { function });
        }
        if f.doc.trim().is_empty() {
            return Err(RegistryError::EmptyDoc { function });
        }
        let all = f.params.iter().chain(&f.results);
        for (i, p) in all.clone().enumerate() {
            if !is_ident(&p.name) {
                return Err(RegistryError::InvalidParamName { function, name: p.name.clone() });
            }
            if all.clone().take(i).any(|q| q.name == p.name) {
                return Err(RegistryError::DuplicateParam { function, name: p.name.clone() });
            }
            if let ApiType::Struct(s) = &p.ty
                && !self.types.contains_key(s)
            {
                return Err(RegistryError::UnknownStruct { function, name: s.clone() });
            }
        }
        if let Some(r) = f.results.iter().find(|r| !r.ty.is_scalar()) {
            return Err(RegistryError::NonScalarResult { function, name: r.name.clone() });
        }
        if self.events.contains_key(&(f.namespace.clone(), f.name.clone())) {
            return Err(RegistryError::EventFunctionClash { name: function });
        }
        f.owner = ns.owner.clone();
        Ok(())
    }

    fn entry<F>(desc: HostFn, f: F) -> Entry<H>
    where
        F: Fn(&mut Caller<'_, H>, &Args<'_>, &mut Results<'_>) -> Result<HostOk, Trap> + Send + Sync + 'static,
    {
        let ty = desc.func_type();
        let sig = Arc::new(Sig::new(desc));
        let s = sig.clone();
        let raw: HostFunc<H> = Arc::new(move |caller: &mut Caller<'_, H>, args: &[u64], res: &mut [u64]| {
            let a = Args { sig: &s, raw: args };
            let mut r = Results { sig: &s, raw: res };
            f(caller, &a, &mut r)
        });
        Entry { sig, ty, raw }
    }

    /// Registers a host function. Rules: the namespace exists (the function's `owner` becomes
    /// the namespace's owner), the name is an identifier, the doc is not blank, parameter and
    /// result names are unique identifiers, `str`/`bytes`/`&Struct` appear only as parameters,
    /// every `&Struct` is a registered type, and `namespace.name` is not registered yet.
    pub fn register<F>(&mut self, mut desc: HostFn, f: F) -> Result<(), RegistryError>
    where
        F: Fn(&mut Caller<'_, H>, &Args<'_>, &mut Results<'_>) -> Result<HostOk, Trap> + Send + Sync + 'static,
    {
        self.check(&mut desc)?;
        let key = (desc.namespace.clone(), desc.name.clone());
        if self.functions.contains_key(&key) {
            return Err(RegistryError::DuplicateFunction { function: desc.qualified() });
        }
        self.functions.insert(key, Self::entry(desc, f));
        Ok(())
    }

    /// Replaces a registered function (hot reload). Same rules as `register`, except that
    /// `namespace.name` must already exist. The signature may change; existing `Imports`
    /// snapshots and instances keep the old function.
    pub fn replace<F>(&mut self, mut desc: HostFn, f: F) -> Result<(), RegistryError>
    where
        F: Fn(&mut Caller<'_, H>, &Args<'_>, &mut Results<'_>) -> Result<HostOk, Trap> + Send + Sync + 'static,
    {
        self.check(&mut desc)?;
        let key = (desc.namespace.clone(), desc.name.clone());
        if !self.functions.contains_key(&key) {
            return Err(RegistryError::NoSuchFunction { function: desc.qualified() });
        }
        self.functions.insert(key, Self::entry(desc, f));
        Ok(())
    }

    /// Registers a struct layout: `StructDesc::validate` must pass, the name must be new and
    /// a component id must not be used by another struct.
    pub fn register_type(&mut self, desc: StructDesc) -> Result<(), RegistryError> {
        desc.validate().map_err(RegistryError::InvalidStruct)?;
        if self.types.contains_key(&desc.name) {
            return Err(RegistryError::DuplicateStruct { name: desc.name });
        }
        if let Some(id) = desc.component
            && let Some(other) = self.types.values().find(|t| t.component == Some(id))
        {
            return Err(RegistryError::DuplicateComponent { id, name: desc.name.clone(), existing: other.name.clone() });
        }
        self.types.insert(desc.name.clone(), desc);
        Ok(())
    }

    /// Checks `e` against the event rules and fills in its owner.
    fn check_event(&self, e: &mut EventDesc) -> Result<(), RegistryError> {
        let event = e.qualified();
        let ns = self
            .namespaces
            .get(&e.namespace)
            .ok_or_else(|| RegistryError::UnknownEventNamespace { event: event.clone() })?;
        if !is_ident(&e.name) {
            return Err(RegistryError::InvalidEventName { event });
        }
        if e.doc.trim().is_empty() {
            return Err(RegistryError::EmptyEventDoc { event });
        }
        if let Some(s) = &e.payload
            && !self.types.contains_key(s)
        {
            return Err(RegistryError::UnknownEventPayload { event, name: s.clone() });
        }
        if self.functions.contains_key(&(e.namespace.clone(), e.name.clone())) {
            return Err(RegistryError::EventFunctionClash { name: event });
        }
        e.owner = ns.owner.clone();
        Ok(())
    }

    /// Registers a script event. Rules: the namespace exists (the event's `owner` becomes the
    /// namespace's owner), the name is an identifier, the doc is not blank, the payload (if
    /// any) is a registered struct, no function has the same `namespace.name`, and the event
    /// is not registered yet.
    pub fn register_event(&mut self, mut desc: EventDesc) -> Result<(), RegistryError> {
        self.check_event(&mut desc)?;
        let key = (desc.namespace.clone(), desc.name.clone());
        if self.events.contains_key(&key) {
            return Err(RegistryError::DuplicateEvent { event: desc.qualified() });
        }
        self.events.insert(key, desc);
        Ok(())
    }

    /// Replaces a registered event (hot reload): same rules as `register_event`, except that
    /// `namespace.name` must already exist. The payload may change; `Imports` snapshots and
    /// instances linked earlier keep the old layout.
    pub fn replace_event(&mut self, mut desc: EventDesc) -> Result<(), RegistryError> {
        self.check_event(&mut desc)?;
        let key = (desc.namespace.clone(), desc.name.clone());
        if !self.events.contains_key(&key) {
            return Err(RegistryError::NoSuchEvent { event: desc.qualified() });
        }
        self.events.insert(key, desc);
        Ok(())
    }

    /// Removes every namespace (with its functions and events), event and struct owned by
    /// `owner` (a mod unload) and returns how many items (namespaces + functions + events +
    /// structs) were removed.
    ///
    /// If a function of another owner still takes `&S` for a struct `S` of this owner, the
    /// call fails with `StructInUse`; if an event of another owner still carries such a struct
    /// as its payload, with `StructInUseByEvent`. Then nothing is removed.
    pub fn unregister_owner(&mut self, owner: &str) -> Result<usize, RegistryError> {
        for e in self.functions.values() {
            if e.sig.desc.owner == owner {
                continue;
            }
            for p in &e.sig.desc.params {
                if let ApiType::Struct(s) = &p.ty
                    && self.types.get(s).is_some_and(|t| t.owner == owner)
                {
                    return Err(RegistryError::StructInUse { name: s.clone(), function: e.sig.desc.qualified() });
                }
            }
        }
        for e in self.events.values() {
            if e.owner == owner {
                continue;
            }
            if let Some(s) = &e.payload
                && self.types.get(s).is_some_and(|t| t.owner == owner)
            {
                return Err(RegistryError::StructInUseByEvent { name: s.clone(), event: e.qualified() });
            }
        }
        let count = |r: &Self| r.namespaces.len() + r.functions.len() + r.events.len() + r.types.len();
        let before = count(self);
        let gone: Vec<String> =
            self.namespaces.values().filter(|n| n.owner == owner).map(|n| n.name.clone()).collect();
        self.namespaces.retain(|_, n| n.owner != owner);
        self.functions.retain(|(ns, _), e| e.sig.desc.owner != owner && !gone.contains(ns));
        self.events.retain(|(ns, _), e| e.owner != owner && !gone.contains(ns));
        self.types.retain(|_, t| t.owner != owner);
        Ok(before - count(self))
    }

    pub fn function(&self, namespace: &str, name: &str) -> Option<&HostFn> {
        self.functions.get(&(namespace.to_string(), name.to_string())).map(|e| &e.sig.desc)
    }

    pub fn struct_type(&self, name: &str) -> Option<&StructDesc> {
        self.types.get(name)
    }

    /// The event `namespace.name`.
    pub fn event(&self, namespace: &str, name: &str) -> Option<&EventDesc> {
        self.events.get(&(namespace.to_string(), name.to_string()))
    }

    /// Events sorted by `(namespace, name)`.
    pub fn events(&self) -> impl Iterator<Item = &EventDesc> {
        self.events.values()
    }

    pub fn namespace(&self, name: &str) -> Option<&NamespaceInfo> {
        self.namespaces.get(name)
    }

    /// Functions sorted by `(namespace, name)`.
    pub fn functions(&self) -> impl Iterator<Item = &HostFn> {
        self.functions.values().map(|e| &e.sig.desc)
    }

    /// Namespaces sorted by name.
    pub fn namespaces(&self) -> impl Iterator<Item = &NamespaceInfo> {
        self.namespaces.values()
    }

    /// Struct layouts sorted by name.
    pub fn types(&self) -> impl Iterator<Item = &StructDesc> {
        self.types.values()
    }

    /// A snapshot of everything callable (cheap: `Arc` clones), with the namespace versions,
    /// the registered events with their payload struct name and size, and the index hash, for
    /// `Instance::new`. Later registrations do not affect it.
    pub fn imports(&self) -> Imports<H> {
        let mut imp = Imports::new();
        for ((ns, name), e) in &self.functions {
            imp.insert(ns, name, e.ty.clone(), e.raw.clone());
        }
        for ns in self.namespaces.values() {
            imp.namespaces.insert(ns.name.clone(), ns.version);
        }
        for ((ns, name), e) in &self.events {
            // A payload struct is registered (checked by `register_event`, kept by
            // `unregister_owner`), so its size is known.
            let payload = e.payload.as_ref().map(|s| (s.clone(), self.types.get(s).map_or(0, |t| t.size)));
            imp.events.insert((ns.clone(), name.clone()), payload);
        }
        imp.hash = Some(self.index().hash());
        imp
    }

    /// The registry as plain data, sorted as the index file is.
    pub fn index(&self) -> ApiIndex {
        ApiIndex {
            game: self.game.clone(),
            namespaces: self.namespaces.values().cloned().collect(),
            types: self.types.values().cloned().collect(),
            functions: self.functions.values().map(|e| e.sig.desc.clone()).collect(),
            events: self.events.values().cloned().collect(),
        }
    }
}

#[cfg(test)]
impl<H: 'static> HostRegistry<H> {
    /// Test helper: defines `namespace` if needed and registers or replaces `namespace.name`
    /// with a function over the raw slots (parameters `p0..`, results `r0..`).
    pub(crate) fn set_raw(
        &mut self,
        namespace: &str,
        name: &str,
        ty: FuncType,
        f: impl Fn(&mut Caller<'_, H>, &[u64], &mut [u64]) -> Result<HostOk, Trap> + Send + Sync + 'static,
    ) {
        if self.namespace(namespace).is_none() {
            self.define_namespace(namespace, 1, "test", "Test functions.").unwrap();
        }
        let mut d = HostFn::new(namespace, name).doc("Test function.");
        for (i, t) in ty.params.iter().enumerate() {
            d = d.param(format!("p{i}"), ApiType::from_val(*t));
        }
        for (i, t) in ty.results.iter().enumerate() {
            d = d.result(format!("r{i}"), ApiType::from_val(*t));
        }
        let w = move |c: &mut Caller<'_, H>, a: &Args<'_>, r: &mut Results<'_>| f(c, a.raw(), r.raw());
        if self.function(namespace, name).is_some() { self.replace(d, w) } else { self.register(d, w) }.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{FieldDesc, Prim};

    fn reg() -> HostRegistry<Vec<String>> {
        let mut r = HostRegistry::new();
        r.define_namespace("std", 1, "std", "Console.").unwrap();
        r
    }

    fn ok(_: &mut Caller<'_, Vec<String>>, _: &Args<'_>, _: &mut Results<'_>) -> Result<HostOk, Trap> {
        Ok(HostOk::Continue)
    }

    #[test]
    fn namespace_rules() {
        let mut r = reg();
        assert_eq!(r.define_namespace("std", 2, "std", "New doc."), Ok(()));
        assert_eq!(r.namespace("std").unwrap().version, 2);
        assert!(matches!(r.define_namespace("std", 1, "mod", ""), Err(RegistryError::NamespaceOwner { .. })));
        assert!(matches!(r.define_namespace("a..b", 1, "x", ""), Err(RegistryError::InvalidNamespace(_))));
        assert!(matches!(r.define_namespace("a", 1, "", ""), Err(RegistryError::EmptyOwner { .. })));
    }

    #[test]
    fn register_rules() {
        let mut r = reg();
        let f = || HostFn::new("std", "println").doc("Print.").param("text", ApiType::Str);
        assert_eq!(r.register(f(), ok), Ok(()));
        assert_eq!(r.function("std", "println").unwrap().owner, "std");
        assert!(matches!(r.register(f(), ok), Err(RegistryError::DuplicateFunction { .. })));
        assert!(r.replace(f().doc("Print again."), ok).is_ok());
        assert_eq!(r.function("std", "println").unwrap().doc, "Print again.");
        let e = |d: HostFn| r.clone_check(d);
        assert!(matches!(e(HostFn::new("nope", "f").doc("x")), Err(RegistryError::UnknownNamespace { .. })));
        assert!(matches!(e(HostFn::new("std", "1f").doc("x")), Err(RegistryError::InvalidFunctionName { .. })));
        assert!(matches!(e(HostFn::new("std", "f").doc("  ")), Err(RegistryError::EmptyDoc { .. })));
        assert!(matches!(
            e(HostFn::new("std", "f").doc("x").param("a b", ApiType::I32)),
            Err(RegistryError::InvalidParamName { .. })
        ));
        assert!(matches!(
            e(HostFn::new("std", "f").doc("x").param("a", ApiType::I32).result("a", ApiType::I32)),
            Err(RegistryError::DuplicateParam { .. })
        ));
        assert!(matches!(
            e(HostFn::new("std", "f").doc("x").result("s", ApiType::Str)),
            Err(RegistryError::NonScalarResult { .. })
        ));
        assert!(matches!(
            e(HostFn::new("std", "f").doc("x").param("t", ApiType::Struct("T".into()))),
            Err(RegistryError::UnknownStruct { .. })
        ));
        assert!(matches!(r.replace(HostFn::new("std", "zzz").doc("x"), ok), Err(RegistryError::NoSuchFunction { .. })));
        assert_eq!(r.imports().names().count(), 1);
    }

    impl<H: 'static> HostRegistry<H> {
        fn clone_check(&self, mut d: HostFn) -> Result<(), RegistryError> {
            self.check(&mut d)
        }
    }

    fn vec2(name: &str, owner: &str, id: Option<u32>) -> StructDesc {
        let mut s = StructDesc::new(name, 8, 4).field(FieldDesc::new("x", 0, Prim::F32, 2));
        s.owner = owner.into();
        s.component = id;
        s
    }

    #[test]
    fn types_and_unregister() {
        let mut r = reg();
        r.register_type(vec2("V", "engine", Some(7))).unwrap();
        assert!(matches!(r.register_type(vec2("V", "engine", None)), Err(RegistryError::DuplicateStruct { .. })));
        assert!(matches!(r.register_type(vec2("W", "engine", Some(7))), Err(RegistryError::DuplicateComponent { .. })));
        assert!(matches!(r.register_type(StructDesc::new("Bad", 3, 2)), Err(RegistryError::InvalidStruct(_))));
        r.define_namespace("modx", 1, "modx", "A mod.").unwrap();
        r.register_type(vec2("M", "modx", None)).unwrap();
        r.register(HostFn::new("modx", "f").doc("x").param("v", ApiType::Struct("V".into())), ok).unwrap();
        r.register(HostFn::new("modx", "g").doc("x").param("m", ApiType::Struct("M".into())), ok).unwrap();
        r.register(HostFn::new("std", "uses_m").doc("x").param("m", ApiType::Struct("M".into())), ok).unwrap();
        // std still uses modx's struct M: refused, nothing removed.
        assert!(matches!(r.unregister_owner("modx"), Err(RegistryError::StructInUse { .. })));
        assert!(r.function("modx", "f").is_some());
        r.unregister_owner("std").unwrap();
        // modx's function using engine's V keeps working; unloading modx removes ns + 2 fns + M.
        assert_eq!(r.unregister_owner("modx"), Ok(4));
        assert!(r.struct_type("V").is_some() && r.struct_type("M").is_none());
        assert_eq!(r.functions().count(), 0);
        assert_eq!(r.unregister_owner("nobody"), Ok(0));
    }

    #[test]
    fn args_and_results_are_checked() {
        let desc = HostFn::new("std", "f")
            .doc("x")
            .param("a", ApiType::I32)
            .param("s", ApiType::Str)
            .param("p", ApiType::Struct("T".into()))
            .param("e", ApiType::Entity)
            .result("r", ApiType::F32);
        let sig = Sig::new(desc);
        let raw = [0xFFFF_FFFF, 10, 20, 30, 40];
        let a = Args { sig: &sig, raw: &raw };
        assert_eq!(a.len(), 4);
        assert_eq!(a.i32(0), Ok(-1));
        assert_eq!(a.str(1), Ok(GuestSlice::new(10, 20)));
        assert_eq!(a.ptr(2), Ok(30));
        assert_eq!(a.entity(3), Ok(40));
        assert!(matches!(a.i64(0), Err(Trap::Host(_))));
        assert!(a.bytes(1).is_err() && a.i32(9).is_err() && a.bool(0).is_err());
        // Too few raw slots never panics.
        let short = Args { sig: &sig, raw: &[1] };
        assert!(short.str(1).is_err() && short.entity(3).is_err());
        let mut out = [0u64; 1];
        let mut r = Results { sig: &sig, raw: &mut out };
        assert!(r.set_i32(0, 1).is_err() && r.set_f32(1, 1.0).is_err());
        r.set_f32(0, 1.5).unwrap();
        assert_eq!(out[0], 1.5f32.to_bits() as u64);
    }
}
