/// An entity handle, `hecs::Entity::to_bits` as a plain `u64`.
///
/// It is `repr(transparent)`, so it crosses the guest boundary as one `i64` (the `entity`
/// API type). A stale or forged handle is rejected by the engine with a status code.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity(pub u64);

impl Entity {
    /// The handle as the engine spells it.
    pub const fn to_bits(self) -> u64 {
        self.0
    }

    /// A handle from the bits the engine gave out.
    pub const fn from_bits(bits: u64) -> Entity {
        Entity(bits)
    }
}
