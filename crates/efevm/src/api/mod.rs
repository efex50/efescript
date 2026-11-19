//! The host registry and the API index (crates/efevm/API_INDEX.md).

pub mod index;
pub mod interface;
pub mod json;
pub mod registry;
pub mod types;

pub use index::{ApiError, ApiIndex};
pub use registry::{Args, GuestSlice, HostRegistry, RegistryError, Results};
pub use types::{
    ApiType, EventDesc, FieldDesc, HostFn, NamespaceInfo, Param, Prim, StructDesc, fnv1a32, fnv1a64, is_ident, is_type_name,
    is_namespace,
};
