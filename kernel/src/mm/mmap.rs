//! Anonymous mmap/munmap/mprotect core (M2, plans/linux26/22 §4):
//! minimal-set Linux semantics over the VMA table — MAP_ANONYMOUS|
//! MAP_PRIVATE mappings only, top-down placement between the heap limit
//! and the stack region, eager zero-fill, mprotect PTE re-permissioning
//! with invlpg, and munmap restricted to anonymous mappings.

use super::frame::{kfree, p2v, PGSIZE};
use super::pgtable::{self, PTE_ADDR, PTE_NX, PTE_P, PTE_U, PTE_W};
use super::vm::{Mm, HEAP_LIMIT, USER_STACK_PAGES, USER_STACK_TOP};
use super::vma::{self, VMA_ANON, VMA_HEAP};
use crate::arch::x86_64::io;

/// Lowest anonymous mmap placement (== brk ceiling).
pub const MMAP_FLOOR: usize = HEAP_LIMIT;
/// Highest anonymous mmap placement: below the stack reservation and a
/// 64 MiB growth/guard gap under it (mirrors exec's bound).
pub const MMAP_CEILING: usize = USER_STACK_TOP - 0x1000_0000 - USER_STACK_PAGES * PGSIZE;

/// PTE permission bits for a PROT_* set (present + user implied;
/// non-executable unless PROT_X, as EFER.NXE is on).
pub fn prot_to_pte(prot: u32) -> u64 {
    let mut pte = PTE_P | PTE_U;
    if prot & vma::PROT_W != 0 {
        pte |= PTE_W;
    }
    if prot & vma::PROT_X == 0 {
        pte |= PTE_NX;
    }
    pte
}

impl Mm {
    /// mmap(2) anonymous core. `addr` 0 picks the highest free gap
    /// (top-down); MAP_FIXED demands the exact aligned address. PROT_NONE
    /// reserves the range without mapping pages. Returns the mapping
    /// start, or 0 on bad args/placement/OOM (with the VMA table intact).
    pub fn mmap_anon(&mut self, addr: usize, len: usize, prot: u32, fixed: bool) -> usize {
        if len == 0 || prot & !vma::PROT_ALL != 0 {
            return 0;
        }
        let end_len = pgtable::pgroundup(len);
        let start = if fixed {
            if addr % PGSIZE != 0 || addr < MMAP_FLOOR || addr + end_len > MMAP_CEILING {
                return 0;
            }
            if self.vmas.overlap(addr, addr + end_len).is_some() {
                return 0;
            }
            addr
        } else {
            match self
                .vmas
                .find_area_topdown(end_len, MMAP_FLOOR, MMAP_CEILING)
            {
                Some(a) => a,
                None => return 0,
            }
        };
        let new = vma::Vma {
            start,
            end: start + end_len,
            prot,
            flags: VMA_ANON,
        };
        if !self.vmas.insert(new) {
            return 0;
        }
        if prot != vma::PROT_NONE && self.alloc(start, new.end, prot_to_pte(prot)) == 0 {
            // Out of memory: roll the reservation back untouched.
            self.vmas.remove_range(start, new.end);
            return 0;
        }
        start
    }

    /// munmap(2) core: page-aligned [addr, addr+len) must be fully
    /// covered by anonymous VMAs only; their pages are freed and the
    /// PTEs cleared. False leaves the address space untouched.
    pub fn munmap(&mut self, addr: usize, len: usize) -> bool {
        if len == 0 {
            return false;
        }
        let start = pgtable::pgrounddown(addr);
        let end = start + pgtable::pgroundup(len);
        // Policy (M2 minimal set): only anonymous mappings unmap; the
        // range must be fully covered so the PTE walk matches the VMA
        // extent exactly.
        let mut cover = start;
        let mut i = 0usize;
        while i < self.vmas.count {
            let v = self.vmas.vmas[i];
            if v.end <= start {
                i += 1;
                continue;
            }
            if v.start >= end {
                break;
            }
            if v.start > cover || v.flags & (VMA_ANON | VMA_HEAP) == 0 {
                return false;
            }
            cover = cover.max(v.end);
            i += 1;
        }
        if cover < end {
            return false;
        }
        if !self.vmas.remove_range(start, end) {
            return false;
        }
        self.unmap_pages(start, end);
        true
    }

    /// mprotect(2) core: re-protect fully covered [addr, addr+len);
    /// PTE permission bits follow (PROT_NONE keeps pages resident but
    /// strips PTE_U). False leaves everything untouched.
    pub fn mprotect(&mut self, addr: usize, len: usize, prot: u32) -> bool {
        if len == 0 || prot & !vma::PROT_ALL != 0 {
            return false;
        }
        let start = pgtable::pgrounddown(addr);
        let end = start + pgtable::pgroundup(len);
        if !self.vmas.set_prot(start, end, prot) {
            return false;
        }
        let mut a = start;
        while a < end {
            // SAFETY: valid table; walk allocates nothing.
            let pte = unsafe { pgtable::walk(self.pml4, a, false) };
            if !pte.is_null() && unsafe { *pte } & PTE_P != 0 {
                let entry = unsafe { *pte };
                let pa = entry & PTE_ADDR;
                if prot == vma::PROT_NONE {
                    // Resident but user-inaccessible.
                    unsafe { *pte = pa | PTE_P | PTE_NX };
                } else {
                    unsafe { *pte = pa | prot_to_pte(prot) };
                }
                // SAFETY: this mm may be the active CR3.
                io::invlpg(a as u64);
            }
            a += PGSIZE;
        }
        true
    }

    /// Unmap and free every present page in [start, end) (no VMA
    /// bookkeeping: callers own the table).
    fn unmap_pages(&mut self, start: usize, end: usize) {
        let mut a = start;
        while a < end {
            // SAFETY: valid table; walk allocates nothing.
            let pte = unsafe { pgtable::walk(self.pml4, a, false) };
            if !pte.is_null() && unsafe { *pte } & PTE_P != 0 {
                let entry = unsafe { *pte };
                // SAFETY: page-table page reached via the direct map.
                kfree(p2v((entry & PTE_ADDR) as usize) as *mut u8);
                unsafe { *pte = 0 };
                // SAFETY: this mm may be the active CR3.
                io::invlpg(a as u64);
            }
            a += PGSIZE;
        }
    }
}
