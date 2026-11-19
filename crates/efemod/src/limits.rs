//! Reader limits (MOD_PACKAGE.md section 6).

/// Limits applied by the package reader. The defaults are the hard caps; an embedder may
/// lower them, never raise them (see [`ReadLimits::clamped`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadLimits {
    /// Whole package file, in bytes.
    pub file_size: u64,
    /// Entries, including ignored and directory entries.
    pub entries: u32,
    /// The 7z header (the uncompressed metadata at the end of the file), in bytes.
    pub header_size: u64,
    /// LZMA2 dictionary size a coder may declare, in bytes.
    pub dictionary_size: u64,
    /// One entry name, in bytes.
    pub name_bytes: usize,
    /// Segments in one entry name.
    pub name_segments: usize,
    /// `manifest.json`, uncompressed.
    pub manifest_size: u64,
    /// `module.efvm`, uncompressed.
    pub module_size: u64,
    /// Any other entry, uncompressed.
    pub asset_size: u64,
    /// All entries together, uncompressed.
    pub total_size: u64,
}

const MIB: u64 = 1024 * 1024;

impl ReadLimits {
    /// The hard caps.
    pub const HARD: ReadLimits = ReadLimits {
        file_size: 64 * MIB,
        entries: 4096,
        header_size: 4 * MIB,
        dictionary_size: 64 * MIB,
        name_bytes: 240,
        name_segments: 16,
        manifest_size: 64 * 1024,
        module_size: 16 * MIB,
        asset_size: 32 * MIB,
        total_size: 128 * MIB,
    };

    /// These limits with every value above its hard cap lowered to the cap.
    pub fn clamped(&self) -> ReadLimits {
        let h = Self::HARD;
        ReadLimits {
            file_size: self.file_size.min(h.file_size),
            entries: self.entries.min(h.entries),
            header_size: self.header_size.min(h.header_size),
            dictionary_size: self.dictionary_size.min(h.dictionary_size),
            name_bytes: self.name_bytes.min(h.name_bytes),
            name_segments: self.name_segments.min(h.name_segments),
            manifest_size: self.manifest_size.min(h.manifest_size),
            module_size: self.module_size.min(h.module_size),
            asset_size: self.asset_size.min(h.asset_size),
            total_size: self.total_size.min(h.total_size),
        }
    }
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self::HARD
    }
}
