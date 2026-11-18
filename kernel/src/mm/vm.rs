//! Per-process address-space bookkeeping (xv6 proc->sz / pgdir semantics).

use super::frame::{kalloc, kfree, v2p, PGSIZE};
use super::pgtable::{self, PtPtr};

/// A user address space: one PML4 plus the byte size of mapped user memory
/// (xv6's sz: text+data+bss+heap+stack, all page-rounded at the edges).
pub struct Mm {
    pub pml4: PtPtr,
    pub sz: usize,
}

pub const USER_STACK_TOP: usize = 0x0000_4000_0000_0000;
/// Eight pages (32 KiB): xv6's single page suits PDP-style C locals, but
/// the Rust user runtime's frames (and the usertests ports' buffers) need
/// headroom below USER_STACK_TOP.
pub const USER_STACK_PAGES: usize = 8;

impl Mm {
    /// Empty user aspace (kernel half shared).
    pub fn new() -> Option<Mm> {
        let pml4 = pgtable::setup_user_pml4();
        if pml4.is_null() {
            return None;
        }
        Some(Mm { pml4, sz: 0 })
    }

    pub fn cr3_pa(&self) -> usize {
        v2p(self.pml4 as usize)
    }

    /// allocuvm: grow [oldsz, newsz) with zeroed RW|U pages.
    pub fn alloc(&mut self, oldsz: usize, newsz: usize) -> usize {
        if newsz <= oldsz {
            return oldsz;
        }
        let mut a = pgtable::pgroundup(oldsz);
        while a < newsz {
            let page = kalloc();
            if page.is_null() {
                return 0;
            }
            // SAFETY: fresh page.
            unsafe {
                crate::klib::string::memset(page, 0, PGSIZE);
                if pgtable::mappages(
                    self.pml4,
                    a,
                    PGSIZE,
                    v2p(page as usize),
                    pgtable::PTE_W | pgtable::PTE_U,
                ) < 0
                {
                    kfree(page);
                    return 0;
                }
            }
            a += PGSIZE;
        }
        newsz
    }

    /// deallocuvm: shrink to newsz, freeing whole pages.
    pub fn dealloc(&mut self, oldsz: usize, newsz: usize) -> usize {
        if newsz >= oldsz {
            return oldsz;
        }
        let mut a = pgtable::pgroundup(newsz);
        let last = pgtable::pgroundup(oldsz);
        // Free the page straddling newsz, if any (xv6 frees it too).
        let first = pgtable::pgrounddown(newsz);
        if first < a {
            // SAFETY: valid table.
            unsafe { pgtable::unmap_page(self.pml4, first) };
        }
        while a < last {
            // SAFETY: valid table.
            unsafe { pgtable::unmap_page(self.pml4, a) };
            a += PGSIZE;
        }
        newsz
    }

    /// freevm: drop the whole aspace (page tables + user pages).
    pub fn free(&mut self) {
        if !self.pml4.is_null() {
            // SAFETY: table no longer referenced by any CR3.
            unsafe { pgtable::free_user_pml4(self.pml4) };
            self.pml4 = core::ptr::null_mut();
        }
        self.sz = 0;
    }

    /// copyuvm: duplicate the aspace (fork).
    #[allow(dead_code)] // S2 fork address-space clone
    pub fn copy(&self) -> Option<Mm> {
        // SAFETY: self.pml4 is a valid user PML4.
        let dst = unsafe { pgtable::copy_user_half(self.pml4) };
        if dst.is_null() {
            return None;
        }
        Some(Mm {
            pml4: dst,
            sz: self.sz,
        })
    }

    /// Load one zeroed RW|U page at `va` (stack page install).
    #[allow(dead_code)] // S3 heap extension zero-fill
    pub fn map_zero_page(&mut self, va: usize) -> bool {
        let page = kalloc();
        if page.is_null() {
            return false;
        }
        // SAFETY: fresh page.
        unsafe {
            crate::klib::string::memset(page, 0, PGSIZE);
            if pgtable::mappages(
                self.pml4,
                va,
                PGSIZE,
                v2p(page as usize),
                pgtable::PTE_W | pgtable::PTE_U,
            ) < 0
            {
                kfree(page);
                return false;
            }
        }
        true
    }
}
