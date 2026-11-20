//! The allocator behind the wasm32 `#[global_allocator]`, written against a small [`Memory`]
//! trait so its logic is tested natively.
//!
//! Design: a bump pointer over pages obtained from `memory.grow`, plus one free list per
//! power-of-two size class (16 bytes and up). Limits, on purpose:
//!
//! - every request is rounded up to a power of two, so a block wastes up to half its size;
//! - freed blocks are reused only for requests of the same class; nothing is split or merged,
//!   so a program that frees many small blocks and then asks for big ones keeps growing;
//! - alignment above 16 bytes is honoured, but such blocks are never reused (a `dealloc` of one
//!   leaks it);
//! - memory is never returned to the host (wasm linear memory cannot shrink);
//! - single threaded: the wasm32-unknown-unknown target has no threads.

use core::alloc::Layout;
use core::ptr::null_mut;

/// Wasm page size.
pub const PAGE: usize = 65536;
/// Alignment of every pooled block.
const BASE_ALIGN: usize = 16;
const CLASSES: usize = usize::BITS as usize;

/// A linear memory that can grow.
pub trait Memory {
    /// Adds `pages` zeroed pages and returns the address of the first new one, or null.
    fn grow(&mut self, pages: usize) -> *mut u8;
}

struct FreeNode {
    next: *mut FreeNode,
}

/// The heap state.
pub struct Heap {
    cur: usize,
    end: usize,
    free: [*mut FreeNode; CLASSES],
}

impl Heap {
    pub const fn new() -> Heap {
        Heap { cur: 0, end: 0, free: [null_mut(); CLASSES] }
    }

    /// Allocates a block, or returns null when the memory cannot grow.
    ///
    /// # Safety
    /// `mem` must be the same memory on every call, and the heap must not be used from two
    /// threads at once.
    pub unsafe fn alloc<M: Memory>(&mut self, mem: &mut M, layout: Layout) -> *mut u8 {
        if layout.align() > BASE_ALIGN {
            return self.bump(mem, layout.size().max(1), layout.align());
        }
        let size = match layout.size().max(BASE_ALIGN).checked_next_power_of_two() {
            Some(s) => s,
            None => return null_mut(),
        };
        let class = size.trailing_zeros() as usize;
        let head = self.free[class];
        if !head.is_null() {
            // SAFETY: a node in the list is a block this heap handed out and got back.
            self.free[class] = unsafe { (*head).next };
            return head as *mut u8;
        }
        self.bump(mem, size, BASE_ALIGN)
    }

    /// Returns a block to its free list.
    ///
    /// # Safety
    /// `ptr` must come from `alloc` of this heap with the same `layout`.
    pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        if layout.align() > BASE_ALIGN {
            return; // leaked, see the module docs
        }
        let size = layout.size().max(BASE_ALIGN).next_power_of_two();
        let class = size.trailing_zeros() as usize;
        let node = ptr as *mut FreeNode;
        // SAFETY: the block is at least 16 bytes and 16-aligned.
        unsafe { (*node).next = self.free[class] };
        self.free[class] = node;
    }

    fn bump<M: Memory>(&mut self, mem: &mut M, size: usize, align: usize) -> *mut u8 {
        // Try the current region first, then grow once; a grow always makes the request fit.
        for _ in 0..2 {
            let start = match self.cur.checked_add(align - 1) {
                Some(v) => v & !(align - 1),
                None => return null_mut(),
            };
            if let Some(stop) = start.checked_add(size)
                && stop <= self.end
            {
                self.cur = stop;
                return start as *mut u8;
            }
            let need = match size.checked_add(align) {
                Some(n) => n.div_ceil(PAGE),
                None => return null_mut(),
            };
            let base = mem.grow(need);
            if base.is_null() {
                return null_mut();
            }
            let base = base as usize;
            if base == self.end && self.end != 0 {
                self.end += need * PAGE; // contiguous: keep the rest of the old region
            } else {
                // Address 0 is the null pointer: leave the first block of memory unused.
                self.cur = if base == 0 { BASE_ALIGN } else { base };
                self.end = base + need * PAGE;
            }
        }
        null_mut()
    }
}

impl Default for Heap {
    fn default() -> Heap {
        Heap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{Layout as L, alloc_zeroed, dealloc};

    /// A fake linear memory: one big zeroed buffer, `grow` hands out consecutive pages.
    struct Fake {
        base: *mut u8,
        pages: usize,
        used: usize,
    }

    impl Fake {
        fn new(pages: usize) -> Fake {
            let base = unsafe { alloc_zeroed(L::from_size_align(pages * PAGE, PAGE).unwrap()) };
            Fake { base, pages, used: 0 }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            unsafe { dealloc(self.base, L::from_size_align(self.pages * PAGE, PAGE).unwrap()) };
        }
    }

    impl Memory for Fake {
        fn grow(&mut self, pages: usize) -> *mut u8 {
            if self.used + pages > self.pages {
                return null_mut();
            }
            let p = unsafe { self.base.add(self.used * PAGE) };
            self.used += pages;
            p
        }
    }

    fn lay(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn blocks_are_distinct_aligned_and_inside_memory() {
        let mut mem = Fake::new(4);
        let mut h = Heap::new();
        let mut got = Vec::new();
        for size in [1usize, 7, 16, 17, 100, 1000, 5000] {
            let p = unsafe { h.alloc(&mut mem, lay(size, 8)) };
            assert!(!p.is_null());
            assert_eq!(p as usize % 16, 0);
            assert!(p as usize >= mem.base as usize && (p as usize) < mem.base as usize + 4 * PAGE);
            unsafe { p.write_bytes(0xAB, size) };
            got.push((p as usize, size.max(16).next_power_of_two()));
        }
        got.sort();
        for w in got.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "blocks overlap: {w:?}");
        }
    }

    #[test]
    fn freed_blocks_are_reused_by_class() {
        let mut mem = Fake::new(2);
        let mut h = Heap::new();
        let a = unsafe { h.alloc(&mut mem, lay(40, 8)) };
        unsafe { h.dealloc(a, lay(40, 8)) };
        // 33..=64 bytes share a class.
        let b = unsafe { h.alloc(&mut mem, lay(64, 8)) };
        assert_eq!(a, b);
        let c = unsafe { h.alloc(&mut mem, lay(8, 8)) };
        assert_ne!(b, c);
    }

    #[test]
    fn big_alignment_is_honoured_and_never_reused() {
        let mut mem = Fake::new(4);
        let mut h = Heap::new();
        let a = unsafe { h.alloc(&mut mem, lay(10, 4096)) };
        assert_eq!(a as usize % 4096, 0);
        unsafe { h.dealloc(a, lay(10, 4096)) };
        let b = unsafe { h.alloc(&mut mem, lay(10, 4096)) };
        assert_ne!(a, b);
        assert_eq!(b as usize % 4096, 0);
    }

    #[test]
    fn grows_across_pages_and_reports_exhaustion() {
        let mut mem = Fake::new(3);
        let mut h = Heap::new();
        // Each block is 32 KiB: two per page.
        let mut n = 0;
        loop {
            let p = unsafe { h.alloc(&mut mem, lay(30000, 8)) };
            if p.is_null() {
                break;
            }
            n += 1;
            assert!(n < 100);
        }
        assert!(n >= 4, "only {n} blocks fit in 3 pages");
        // A request larger than all of memory fails cleanly.
        assert!(unsafe { h.alloc(&mut mem, lay(10 * PAGE, 8)) }.is_null());
        assert!(unsafe { h.alloc(&mut mem, lay(usize::MAX / 4, 8)) }.is_null());
    }

    #[test]
    fn a_block_bigger_than_a_page_gets_enough_pages() {
        let mut mem = Fake::new(8);
        let mut h = Heap::new();
        let p = unsafe { h.alloc(&mut mem, lay(PAGE + 1, 8)) };
        assert!(!p.is_null());
        unsafe { p.write_bytes(1, PAGE + 1) };
    }
}
