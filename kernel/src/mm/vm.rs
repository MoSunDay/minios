//! Per-process address-space bookkeeping: the xv6 `sz` model is retired
//! for an M2 VMA table (mm/vma.rs) + a byte-exact brk; segment, stack,
//! heap and anonymous mappings each own a VMA, and the page tables are
//! driven from it (mm/mmap.rs, proc/exec.rs).

use super::frame::{kfree, v2p, PGSIZE};
use super::oom;
use super::pgtable::{self, PtPtr};
use super::vma::{VmaTable, VMA_HEAP};

pub const USER_STACK_TOP: usize = 0x0000_4000_0000_0000;
/// Eight pages (32 KiB): xv6's single page suits PDP-style C locals, but
/// the Rust user runtime's frames (and the usertests ports' buffers) need
/// headroom below USER_STACK_TOP.
pub const USER_STACK_PAGES: usize = 8;
/// brk ceiling == anonymous mmap floor: the low 16 TiB quadrant is the
/// heap's, the top-down mmap area starts above it (plans/linux26/22 §4).
pub const HEAP_LIMIT: usize = 0x0000_1000_0000_0000;

/// A user address space: one PML4, its VMA table and the current break.
pub struct Mm {
    pub pml4: PtPtr,
    pub vmas: VmaTable,
    /// Byte-exact brk (page-aligned right after exec; heap VMA end).
    pub brk: usize,
}

impl Mm {
    /// Empty user aspace (kernel half shared).
    pub fn new() -> Option<Mm> {
        let pml4 = pgtable::setup_user_pml4();
        if pml4.is_null() {
            return None;
        }
        Some(Mm {
            pml4,
            vmas: VmaTable::empty(),
            brk: 0,
        })
    }

    pub fn cr3_pa(&self) -> usize {
        v2p(self.pml4 as usize)
    }

    /// allocuvm: grow [oldsz, newsz) with zeroed pages carrying `perm`
    /// (full PTE bits incl. PTE_P/PTE_U; mappages adds nothing).
    pub fn alloc(&mut self, oldsz: usize, newsz: usize, perm: u64) -> usize {
        if newsz <= oldsz {
            return oldsz;
        }
        let mut a = pgtable::pgroundup(oldsz);
        while a < newsz {
            // Anonymous user pages go through the M2 OOM valve (22 §5).
            let page = oom::alloc_user_page();
            if page.is_null() {
                return 0;
            }
            // SAFETY: fresh page.
            unsafe {
                crate::klib::string::memset(page, 0, PGSIZE);
                if pgtable::mappages(self.pml4, a, PGSIZE, v2p(page as usize), perm) < 0 {
                    kfree(page);
                    return 0;
                }
            }
            a += PGSIZE;
        }
        newsz
    }

    /// deallocuvm: shrink to newsz, freeing whole pages in
    /// [pgroundup(newsz), pgroundup(oldsz)). The page straddling an
    /// unaligned newsz stays mapped (Linux brk keeps live bytes below
    /// the break; the heap VMA tracks the exact boundary).
    pub fn dealloc(&mut self, oldsz: usize, newsz: usize) -> usize {
        if newsz >= oldsz {
            return oldsz;
        }
        let mut a = pgtable::pgroundup(newsz);
        let last = pgtable::pgroundup(oldsz);
        while a < last {
            // SAFETY: valid table.
            unsafe { pgtable::unmap_page(self.pml4, a) };
            a += PGSIZE;
        }
        newsz
    }

    /// brk(2) core: move the byte-exact break to `new`. Grows map zeroed
    /// RW|NX pages; shrinks free whole pages above the new break and trim
    /// the heap VMA. Returns the (possibly unchanged) break on success,
    /// or the old break unchanged when the request is out of range or
    /// mapping fails (Linux brk failure contract).
    pub fn set_brk(&mut self, new: usize) -> usize {
        let old = self.brk;
        if new == old {
            return old;
        }
        if new > HEAP_LIMIT || new < self.heap_floor() {
            return old;
        }
        if new > old {
            // Data pages are NX (plans/linux26/22 §2): grow P|U|RW|NX
            // (mappages ORs in nothing beyond PTE_P).
            const HEAP_PERM: u64 =
                pgtable::PTE_P | pgtable::PTE_U | pgtable::PTE_W | pgtable::PTE_NX;
            if self.alloc(old, new, HEAP_PERM) == 0 {
                return old;
            }
            if !self.vmas.insert(vma_heap(old, new)) {
                return old;
            }
        } else {
            self.dealloc(old, new);
            self.vmas.remove_range(new, old);
        }
        self.brk = new;
        new
    }

    /// Lowest legal break: the start of the heap VMA if one exists,
    /// else the current break (exec's page-aligned initial break).
    fn heap_floor(&self) -> usize {
        for i in 0..self.vmas.count {
            if self.vmas.vmas[i].flags & VMA_HEAP != 0 {
                return self.vmas.vmas[i].start;
            }
        }
        self.brk
    }

    /// freevm: drop the whole aspace (page tables + user pages).
    pub fn free(&mut self) {
        if !self.pml4.is_null() {
            // SAFETY: table no longer referenced by any CR3.
            unsafe { pgtable::free_user_pml4(self.pml4) };
            self.pml4 = core::ptr::null_mut();
        }
        self.vmas = VmaTable::empty();
        self.brk = 0;
    }

    /// copyuvm: duplicate the aspace (fork).
    pub fn copy(&self) -> Option<Mm> {
        // SAFETY: self.pml4 is a valid user PML4.
        let dst = unsafe { pgtable::copy_user_half(self.pml4) };
        if dst.is_null() {
            return None;
        }
        Some(Mm {
            pml4: dst,
            vmas: self.vmas,
            brk: self.brk,
        })
    }

    /// Load one zeroed page at `va` with `perm` (demand-fill primitive).
    #[allow(dead_code)] // S5 fault-path zero-fill
    pub fn map_zero_page(&mut self, va: usize, perm: u64) -> bool {
        // Demand fills are anonymous user pages: OOM valve applies.
        let page = oom::alloc_user_page();
        if page.is_null() {
            return false;
        }
        // SAFETY: fresh page.
        unsafe {
            crate::klib::string::memset(page, 0, PGSIZE);
            if pgtable::mappages(self.pml4, va, PGSIZE, v2p(page as usize), perm) < 0 {
                kfree(page);
                return false;
            }
        }
        true
    }
}

fn vma_heap(start: usize, end: usize) -> super::vma::Vma {
    super::vma::Vma {
        start,
        end,
        prot: super::vma::PROT_RW,
        flags: VMA_HEAP,
    }
}
