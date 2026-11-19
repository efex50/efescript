//! Linear memory (SPEC.md section 3).

use crate::trap::Trap;
use crate::types::PAGE_SIZE;

/// One linear memory: little-endian bytes addressed by `u64`, sized in 64 KiB pages.
#[derive(Debug)]
pub struct Memory {
    bytes: Vec<u8>,
    max_pages: u32,
}

impl Memory {
    /// `max_pages` is the effective maximum (module maximum clamped by the embedder cap).
    /// Returns `None` if `min_pages` exceeds it or the allocation fails.
    pub(crate) fn new(min_pages: u32, max_pages: u32) -> Option<Memory> {
        if min_pages > max_pages {
            return None;
        }
        let len = usize::try_from((min_pages as u64).checked_mul(PAGE_SIZE)?).ok()?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).ok()?;
        bytes.resize(len, 0);
        Some(Memory { bytes, max_pages })
    }

    /// Current size in pages.
    pub fn size_pages(&self) -> u64 {
        self.bytes.len() as u64 / PAGE_SIZE
    }

    /// Current size in bytes.
    pub fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Grows by `delta_pages`; returns the old size in pages, or `None` if the maximum would
    /// be exceeded or the allocation fails. New bytes are zero.
    pub fn grow(&mut self, delta_pages: u64) -> Option<u64> {
        let old = self.size_pages();
        let new = old.checked_add(delta_pages)?;
        if new > self.max_pages as u64 {
            return None;
        }
        if delta_pages == 0 {
            return Some(old);
        }
        let new_len = usize::try_from(new.checked_mul(PAGE_SIZE)?).ok()?;
        self.bytes.try_reserve_exact(new_len - self.bytes.len()).ok()?;
        self.bytes.resize(new_len, 0);
        Some(old)
    }

    /// Range `[addr, addr+len)` as `usize` bounds, if fully inside memory.
    #[inline]
    fn range(&self, addr: u64, len: u64) -> Result<(usize, usize), Trap> {
        let end = addr.checked_add(len).ok_or(Trap::OutOfBounds)?;
        if end > self.bytes.len() as u64 {
            return Err(Trap::OutOfBounds);
        }
        Ok((addr as usize, end as usize))
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), Trap> {
        let (s, e) = self.range(addr, buf.len() as u64)?;
        buf.copy_from_slice(self.bytes.get(s..e).ok_or(Trap::OutOfBounds)?);
        Ok(())
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), Trap> {
        let (s, e) = self.range(addr, data.len() as u64)?;
        self.bytes.get_mut(s..e).ok_or(Trap::OutOfBounds)?.copy_from_slice(data);
        Ok(())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// Loads `N` bytes at `base + off`.
    #[inline]
    pub fn load<const N: usize>(&self, base: u64, off: u32) -> Result<[u8; N], Trap> {
        let ea = base.checked_add(off as u64).ok_or(Trap::OutOfBounds)?;
        let (s, e) = self.range(ea, N as u64)?;
        self.bytes
            .get(s..e)
            .and_then(|sl| <[u8; N]>::try_from(sl).ok())
            .ok_or(Trap::OutOfBounds)
    }

    /// Stores `N` bytes at `base + off`.
    #[inline]
    pub fn store<const N: usize>(&mut self, base: u64, off: u32, data: [u8; N]) -> Result<(), Trap> {
        let ea = base.checked_add(off as u64).ok_or(Trap::OutOfBounds)?;
        let (s, e) = self.range(ea, N as u64)?;
        self.bytes.get_mut(s..e).ok_or(Trap::OutOfBounds)?.copy_from_slice(&data);
        Ok(())
    }

    /// `memory.copy`: both ranges are checked before anything is copied; overlap is handled.
    pub fn copy(&mut self, dst: u64, src: u64, len: u64) -> Result<(), Trap> {
        let (d, _) = self.range(dst, len)?;
        let (s, e) = self.range(src, len)?;
        self.bytes.copy_within(s..e, d);
        Ok(())
    }

    /// `memory.fill`.
    pub fn fill(&mut self, dst: u64, byte: u8, len: u64) -> Result<(), Trap> {
        let (s, e) = self.range(dst, len)?;
        self.bytes.get_mut(s..e).ok_or(Trap::OutOfBounds)?.fill(byte);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        let mut m = Memory::new(1, 2).unwrap();
        assert_eq!(m.len(), 65536);
        let mut b = [0u8; 1];
        assert!(m.read(65535, &mut b).is_ok());
        assert_eq!(m.read(65536, &mut b), Err(Trap::OutOfBounds));
        assert_eq!(m.read(u64::MAX, &mut b), Err(Trap::OutOfBounds));
        assert!(m.read(65536, &mut []).is_ok());
        assert_eq!(m.read(65537, &mut []), Err(Trap::OutOfBounds));
        assert!(m.load::<8>(65528, 0).is_ok());
        assert_eq!(m.load::<8>(65529, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.load::<1>(u64::MAX, 1), Err(Trap::OutOfBounds));
        assert_eq!(m.load::<1>(u64::MAX - 1, u32::MAX), Err(Trap::OutOfBounds));
        assert_eq!(m.store::<4>(65533, 0, [1; 4]), Err(Trap::OutOfBounds));
        assert!(m.store::<4>(65532, 0, [1; 4]).is_ok());
        assert_eq!(m.copy(0, 65535, 2), Err(Trap::OutOfBounds));
        assert_eq!(m.fill(65535, 1, 2), Err(Trap::OutOfBounds));
        assert_eq!(m.as_slice()[65535], 1);
    }

    #[test]
    fn grow_and_copy() {
        let mut m = Memory::new(0, 2).unwrap();
        assert_eq!(m.grow(0), Some(0));
        assert_eq!(m.grow(1), Some(0));
        assert_eq!(m.grow(2), None);
        assert_eq!(m.grow(u64::MAX), None);
        assert_eq!(m.grow(1), Some(1));
        assert_eq!(m.grow(1), None);
        assert_eq!(m.size_pages(), 2);
        m.write(0, &[1, 2, 3, 4]).unwrap();
        m.copy(2, 0, 4).unwrap();
        assert_eq!(&m.as_slice()[..6], &[1, 2, 1, 2, 3, 4]);
        m.fill(1, 9, 2).unwrap();
        assert_eq!(&m.as_slice()[..4], &[1, 9, 9, 2]);
        assert!(Memory::new(3, 2).is_none());
    }
}
