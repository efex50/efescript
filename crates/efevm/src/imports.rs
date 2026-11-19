//! Host function imports: the `Imports` an instance links against and the `Caller` handed to
//! host functions. `Imports` are produced by `HostRegistry::imports` (crate::api::registry).

use crate::api::registry::GuestSlice;
use crate::api::types::StructDesc;
use crate::error::LinkError;
use crate::instance::FiberId;
use crate::memory::Memory;
use crate::module::Module;
use crate::trap::Trap;
use crate::types::FuncType;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a host function asks the VM to do after it returns successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOk {
    Continue,
    /// Write the results, then suspend the fiber with `Exit::Yield`.
    Yield,
}

/// Access handed to host functions: host state, memory and globals.
pub struct Caller<'a, H> {
    host: &'a mut H,
    memory: &'a mut Memory,
    globals: &'a mut [u64],
    mutable: &'a [bool],
    gmask: &'a [u64],
    fiber: FiberId,
}

impl<'a, H> Caller<'a, H> {
    pub(crate) fn new(
        host: &'a mut H,
        memory: &'a mut Memory,
        globals: &'a mut [u64],
        mutable: &'a [bool],
        gmask: &'a [u64],
        fiber: FiberId,
    ) -> Self {
        Caller { host, memory, globals, mutable, gmask, fiber }
    }

    /// The fiber that is making this host call.
    pub fn fiber(&self) -> FiberId {
        self.fiber
    }

    pub fn host(&mut self) -> &mut H {
        self.host
    }

    pub fn memory(&self) -> &Memory {
        self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Memory {
        self.memory
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), Trap> {
        self.memory.read(addr, buf)
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), Trap> {
        self.memory.write(addr, data)
    }

    pub fn global(&self, idx: u32) -> Option<u64> {
        self.globals.get(idx as usize).copied()
    }

    /// `Trap::Host` on a bad index or an immutable global. The upper 32 bits of the value are
    /// cleared when the global is `i32` or `f32`.
    pub fn set_global(&mut self, idx: u32, v: u64) -> Result<(), Trap> {
        let i = idx as usize;
        match (self.globals.get_mut(i), self.mutable.get(i)) {
            (Some(slot), Some(true)) => {
                *slot = v & self.gmask.get(i).copied().unwrap_or(u64::MAX);
                Ok(())
            }
            (Some(_), _) => Err(Trap::Host(format!("global {idx} is immutable"))),
            _ => Err(Trap::Host(format!("no global {idx}"))),
        }
    }

    /// Checks that `[ptr, ptr+len)` lies in memory. Values with bit 63 set (negative as `i64`)
    /// are out of bounds.
    fn span(&self, ptr: u64, len: u64) -> Result<usize, Trap> {
        if ptr > i64::MAX as u64 || len > i64::MAX as u64 {
            return Err(Trap::OutOfBounds);
        }
        match ptr.checked_add(len) {
            Some(end) if end <= self.memory.len() => usize::try_from(len).map_err(|_| Trap::OutOfBounds),
            _ => Err(Trap::OutOfBounds),
        }
    }

    /// Copies a guest byte range out. Bounds are checked before anything is allocated.
    pub fn read_bytes(&self, s: GuestSlice) -> Result<Vec<u8>, Trap> {
        let n = self.span(s.ptr, s.len)?;
        let mut v = vec![0u8; n];
        self.memory.read(s.ptr, &mut v)?;
        Ok(v)
    }

    /// Reads UTF-8 text; invalid UTF-8 is `Trap::Host`.
    pub fn read_str(&self, s: GuestSlice) -> Result<String, Trap> {
        String::from_utf8(self.read_bytes(s)?)
            .map_err(|_| Trap::Host(format!("string at {} (length {}) is not valid UTF-8", s.ptr, s.len)))
    }

    pub fn write_bytes(&mut self, ptr: u64, data: &[u8]) -> Result<(), Trap> {
        self.span(ptr, data.len() as u64)?;
        self.memory.write(ptr, data)
    }

    /// Reads `desc.size` bytes at `ptr`.
    pub fn read_struct(&self, ptr: u64, desc: &StructDesc) -> Result<Vec<u8>, Trap> {
        self.read_bytes(GuestSlice::new(ptr, desc.size as u64))
    }

    /// Writes a struct; `bytes.len()` must equal `desc.size` (else `Trap::Host`).
    pub fn write_struct(&mut self, ptr: u64, desc: &StructDesc, bytes: &[u8]) -> Result<(), Trap> {
        if bytes.len() as u64 != desc.size as u64 {
            return Err(Trap::Host(format!(
                "struct `{}` is {} bytes, got {}",
                desc.name,
                desc.size,
                bytes.len()
            )));
        }
        self.write_bytes(ptr, bytes)
    }

    fn read_n<const N: usize>(&self, ptr: u64) -> Result<[u8; N], Trap> {
        self.span(ptr, N as u64)?;
        self.memory.load::<N>(ptr, 0)
    }

    fn write_n<const N: usize>(&mut self, ptr: u64, b: [u8; N]) -> Result<(), Trap> {
        self.span(ptr, N as u64)?;
        self.memory.store::<N>(ptr, 0, b)
    }

    /// Little-endian `u32` at `ptr`.
    pub fn read_u32(&self, ptr: u64) -> Result<u32, Trap> {
        self.read_n(ptr).map(u32::from_le_bytes)
    }

    pub fn read_u64(&self, ptr: u64) -> Result<u64, Trap> {
        self.read_n(ptr).map(u64::from_le_bytes)
    }

    pub fn read_f32(&self, ptr: u64) -> Result<f32, Trap> {
        self.read_n(ptr).map(f32::from_le_bytes)
    }

    pub fn read_f64(&self, ptr: u64) -> Result<f64, Trap> {
        self.read_n(ptr).map(f64::from_le_bytes)
    }

    pub fn write_u32(&mut self, ptr: u64, v: u32) -> Result<(), Trap> {
        self.write_n(ptr, v.to_le_bytes())
    }

    pub fn write_u64(&mut self, ptr: u64, v: u64) -> Result<(), Trap> {
        self.write_n(ptr, v.to_le_bytes())
    }

    pub fn write_f32(&mut self, ptr: u64, v: f32) -> Result<(), Trap> {
        self.write_n(ptr, v.to_le_bytes())
    }

    pub fn write_f64(&mut self, ptr: u64, v: f64) -> Result<(), Trap> {
        self.write_n(ptr, v.to_le_bytes())
    }
}

/// A raw host function: `(caller, lowered args, lowered results)`. `HostRegistry` wraps the
/// typed form (`Args`/`Results`) into this.
pub type HostFunc<H> =
    Arc<dyn Fn(&mut Caller<'_, H>, &[u64], &mut [u64]) -> Result<HostOk, Trap> + Send + Sync>;

/// The host functions a module can link against, keyed by (namespace, name), plus the
/// namespace versions, the registered script events (with their payload struct name and size)
/// and the API index hash of the registry they came from.
///
/// The only way to fill it is `HostRegistry::imports()`; `Imports::new()` is the empty set.
pub struct Imports<H> {
    pub(crate) funcs: BTreeMap<(String, String), (FuncType, HostFunc<H>)>,
    pub(crate) namespaces: BTreeMap<String, u32>,
    /// `(namespace, name)` -> `Some((payload struct, size in bytes))` or `None` (no payload).
    pub(crate) events: BTreeMap<(String, String), Option<(String, u32)>>,
    pub(crate) hash: Option<u64>,
}

impl<H> Imports<H> {
    /// No functions, no namespaces, no hash.
    pub fn new() -> Self {
        Imports { funcs: BTreeMap::new(), namespaces: BTreeMap::new(), events: BTreeMap::new(), hash: None }
    }

    pub(crate) fn insert(&mut self, namespace: &str, name: &str, ty: FuncType, f: HostFunc<H>) {
        self.funcs.insert((namespace.to_string(), name.to_string()), (ty, f));
    }

    /// Everything callable, sorted by (namespace, name), with its lowered signature.
    pub fn names(&self) -> impl Iterator<Item = (&str, &str, &FuncType)> {
        self.funcs.iter().map(|((ns, n), (t, _))| (ns.as_str(), n.as_str(), t))
    }

    /// The version of a namespace, if the registry defined it.
    pub fn namespace_version(&self, namespace: &str) -> Option<u32> {
        self.namespaces.get(namespace).copied()
    }

    /// `(namespace, version)` sorted by name.
    pub fn namespaces(&self) -> impl Iterator<Item = (&str, u32)> {
        self.namespaces.iter().map(|(n, v)| (n.as_str(), *v))
    }

    /// The payload of a registered event: `Some(Some((struct name, size)))`, `Some(None)` for
    /// an event without payload, `None` if no such event is registered.
    pub fn event(&self, namespace: &str, name: &str) -> Option<Option<(&str, u32)>> {
        self.events
            .get(&(namespace.to_string(), name.to_string()))
            .map(|p| p.as_ref().map(|(s, n)| (s.as_str(), *n)))
    }

    /// Every registered event sorted by (namespace, name), with its payload as in
    /// [`event`](Self::event).
    pub fn events(&self) -> impl Iterator<Item = (&str, &str, Option<(&str, u32)>)> {
        self.events.iter().map(|((ns, n), p)| (ns.as_str(), n.as_str(), p.as_ref().map(|(s, z)| (s.as_str(), *z))))
    }

    /// `ApiIndex::hash()` of the registry this snapshot came from (`None` for `Imports::new()`).
    pub fn api_hash(&self) -> Option<u64> {
        self.hash
    }

    pub(crate) fn get(&self, namespace: &str, name: &str) -> Option<&(FuncType, HostFunc<H>)> {
        self.funcs.get(&(namespace.to_string(), name.to_string()))
    }

    /// Every reason `module` cannot link against these imports, in this order: namespaces of
    /// the module's `api` section that are unknown or have another version, then each import
    /// that is missing or has another signature (in import order), then each event reference
    /// of section 14 that is not registered (`UnknownEvent`) or whose payload struct name or
    /// size differs from the registered one (`EventPayload`), in section order. If there is
    /// any error and
    /// the module records an index hash different from ours, a final `LinkError::ApiHash`
    /// says which index the module was built against. Empty means `Instance::new` links
    /// (memory limits aside). A different hash alone is not an error.
    pub fn check_module(&self, module: &Module) -> Vec<LinkError> {
        let data = module.data();
        let mut errs = Vec::new();
        if let Some(api) = &data.api {
            for (ns, v) in &api.namespaces {
                match self.namespaces.get(ns) {
                    None => errs.push(LinkError::UnknownNamespace { namespace: ns.clone() }),
                    Some(h) if h != v => {
                        errs.push(LinkError::NamespaceVersion { namespace: ns.clone(), module: *v, host: *h })
                    }
                    Some(_) => {}
                }
            }
        }
        for imp in &data.imports {
            let want = data.types.get(imp.type_idx as usize);
            match (self.get(&imp.namespace, &imp.name), want) {
                (Some((have, _)), Some(want)) if have == want => {}
                (Some((have, _)), Some(want)) => errs.push(LinkError::ImportTypeMismatch {
                    namespace: imp.namespace.clone(),
                    name: imp.name.clone(),
                    module: want.to_string(),
                    host: have.to_string(),
                }),
                _ => errs.push(LinkError::MissingImport { namespace: imp.namespace.clone(), name: imp.name.clone() }),
            }
        }
        for ev in &data.events {
            match self.events.get(&(ev.namespace.clone(), ev.name.clone())) {
                None => errs.push(LinkError::UnknownEvent { namespace: ev.namespace.clone(), name: ev.name.clone() }),
                Some(have) if *have == ev.payload => {}
                Some(have) => errs.push(LinkError::EventPayload {
                    namespace: ev.namespace.clone(),
                    name: ev.name.clone(),
                    module: payload_text(ev.payload.as_ref()),
                    host: payload_text(have.as_ref()),
                }),
            }
        }
        if let Some(api) = &data.api
            && !errs.is_empty()
            && self.hash != Some(api.hash)
        {
            errs.push(LinkError::ApiHash { module: api.hash, host: self.hash });
        }
        errs
    }
}

/// `&Name (N bytes)` or `no payload`, for `LinkError::EventPayload`.
fn payload_text(p: Option<&(String, u32)>) -> String {
    match p {
        Some((s, n)) => format!("&{s} ({n} bytes)"),
        None => "no payload".into(),
    }
}

impl<H> Default for Imports<H> {
    fn default() -> Self {
        Self::new()
    }
}
