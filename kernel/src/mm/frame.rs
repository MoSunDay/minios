//! Physical frame allocation front end (xv6 kalloc.c call surface) over
//! the boot memory map + buddy zones + SLUB. Callers keep using
//! kalloc/kfree/p2v/v2p; the implementation moved to mm::bootmem,
//! mm::buddy, mm::slub (and the direct map grew mm::directmap).
//!
//! All RAM is also reachable through the fixed direct map at DIRECT_VMA,
//! so kernel pointers to physical pages never depend on the current CR3.

use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::x86_64::{DIRECT_VMA, PHYS_TOP};
use crate::klib::string::memset;
use crate::klib::sync::Spinlock;
use crate::mm::{bootmem, buddy, directmap, slub};

pub const PGSIZE: usize = 4096;
/// Top pfn managed by the buddy (PHYS_TOP / PGSIZE).
pub const MAX_PFN: usize = PHYS_TOP / PGSIZE;
/// 16 MiB DMA/low zone boundary (ISA DMA reach, Linux convention).
pub const DMA_TOP_PFN: usize = 0x100_0000 / PGSIZE;

static KMEM_LOCK: Spinlock<()> = Spinlock::new("kmem", ());
static mut FLOOR_PA: usize = 0;

/// The mem_map: one Page per physical frame.
static mut MEM_MAP: [buddy::Page; MAX_PFN] = [buddy::Page::EMPTY; MAX_PFN];
/// [DMA, NORMAL]: allocation prefers the tail of the array.
static mut ZONES: [buddy::Zone; 2] = [
    buddy::Zone::new("DMA", 0, DMA_TOP_PFN as u32),
    buddy::Zone::new("NORMAL", DMA_TOP_PFN as u32, MAX_PFN as u32),
];
static mut SLUB: slub::Slub = slub::Slub::EMPTY;
static FREE_FRAMES: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    static __kernel_phys_end: u8;
}

pub fn kernel_phys_end() -> usize {
    // SAFETY: linker symbol, read as integer only.
    core::ptr::addr_of!(__kernel_phys_end) as usize
}

#[inline]
pub fn p2v(pa: usize) -> usize {
    pa + DIRECT_VMA
}

#[inline]
pub fn v2p(va: usize) -> usize {
    va - DIRECT_VMA
}

fn mem_map_mut() -> &'static mut [buddy::Page] {
    // SAFETY: guarded by KMEM_LOCK after boot; boot context is
    // single-threaded.
    unsafe { &mut *core::ptr::addr_of_mut!(MEM_MAP) }
}

fn zones_mut() -> &'static mut [buddy::Zone; 2] {
    // SAFETY: see mem_map_mut.
    unsafe { &mut *core::ptr::addr_of_mut!(ZONES) }
}

fn slub_mut() -> &'static mut slub::Slub {
    // SAFETY: guarded by KMEM_LOCK.
    unsafe { &mut *core::ptr::addr_of_mut!(SLUB) }
}

fn recount_free() {
    let n = zones_mut().iter().map(|z| z.free_pages).sum();
    FREE_FRAMES.store(n, Ordering::SeqCst);
}

/// One-shot boot init: parse the loader map, seed the buddy from every
/// usable page at or above the kernel image end, extend the direct map
/// over all probed memory. Called once with interrupts off.
pub fn init(mbi_phys: usize) {
    // SAFETY: single-threaded boot context; static mut writes.
    unsafe {
        FLOOR_PA = (kernel_phys_end() + PGSIZE - 1) & !(PGSIZE - 1);
    }
    bootmem::init(mbi_phys, unsafe { FLOOR_PA }, PHYS_TOP);
    // Mark usable pages as boot-allocated (count 1, not reserved);
    // everything else stays PG_RESERVED and never enters the buddy.
    for reg in bootmem::usable().iter() {
        let mut pa = reg.base as usize;
        let end = (reg.base + reg.len) as usize;
        while pa < end {
            let pfn = pa / PGSIZE;
            if pfn < MAX_PFN {
                let pg = &mut mem_map_mut()[pfn];
                pg.flags = 0;
                pg.count = 1;
            }
            pa += PGSIZE;
        }
    }
    {
        let m = mem_map_mut();
        let z = zones_mut();
        for zi in 0..2 {
            buddy::release_zone(&mut z[zi], &mut *m);
        }
    }
    recount_free();
    directmap::init(bootmem::probed_top_pa());
}

/// kalloc: one zeroed?? (xv6: not zeroed; callers memset) page.
pub fn kalloc() -> *mut u8 {
    let _guard = KMEM_LOCK.lock();
    alloc_page_locked().unwrap_or(null_mut())
}

/// Internal order-0 allocator with the lock already held (also the SLUB
/// page source).
fn alloc_page_locked() -> Option<*mut u8> {
    let m = mem_map_mut();
    let z = zones_mut();
    // NORMAL first (index 1), DMA fallback (index 0).
    for zi in [1usize, 0] {
        if let Some(pfn) = buddy::alloc_order(z, m, zi, 0) {
            recount_free();
            return Some(p2v(pfn as usize * PGSIZE) as *mut u8);
        }
    }
    None
}

/// kfree: drop one reference to a page (direct-map pointer). COW-shared
/// frames (count > 1) only decrement; the last reference junk-fills the
/// page and hands it back to the buddy.
pub fn kfree(v: *mut u8) {
    let pa = v2p(v as usize);
    if v as usize % PGSIZE != 0 || pa < unsafe { FLOOR_PA } || pa >= PHYS_TOP {
        panic!("kfree: bad page {:#x}", v as usize);
    }
    let _guard = KMEM_LOCK.lock();
    let m = mem_map_mut();
    let pg = &mut m[pa / PGSIZE];
    if pg.flags & buddy::PG_RESERVED != 0 || pg.flags & buddy::PG_FREE != 0 || pg.count == 0 {
        panic!("kfree: double free or bad frame {:#x}", pa);
    }
    if pg.count > 1 {
        // Another owner still maps this frame; keep the page resident.
        pg.count -= 1;
        return;
    }
    memset(v, 1, PGSIZE);
    if !buddy::free_page(zones_mut(), m, (pa / PGSIZE) as u32) {
        panic!("kfree: double free or bad frame {:#x}", pa);
    }
    recount_free();
}

/// Current share count of an allocated frame (COW bookkeeping; 0 when
/// the frame is free or reserved).
pub fn page_ref(pa: usize) -> u16 {
    let _guard = KMEM_LOCK.lock();
    let pg = &mem_map_mut()[pa / PGSIZE];
    if pg.flags & buddy::PG_FREE != 0 || pg.flags & buddy::PG_RESERVED != 0 {
        return 0;
    }
    pg.count
}

/// Add one share on an allocated frame (fork maps the parent's frames
/// into the child address space).
pub fn page_ref_inc(pa: usize) {
    let _guard = KMEM_LOCK.lock();
    let pg = &mut mem_map_mut()[pa / PGSIZE];
    if pg.flags & buddy::PG_FREE != 0 || pg.flags & buddy::PG_RESERVED != 0 || pg.count == 0 {
        panic!("page_ref_inc: unallocated frame {:#x}", pa);
    }
    pg.count += 1;
}

/// Drop one share taken by `page_ref_inc` (never the allocation ticket:
/// the final release always goes through `kfree`).
pub fn page_ref_dec(pa: usize) {
    let _guard = KMEM_LOCK.lock();
    let pg = &mut mem_map_mut()[pa / PGSIZE];
    if pg.flags & buddy::PG_FREE != 0 || pg.flags & buddy::PG_RESERVED != 0 || pg.count < 2 {
        panic!("page_ref_dec: unshared frame {:#x}", pa);
    }
    pg.count -= 1;
}

/// kmalloc: SLUB object allocation (8..=2048 byte classes); sizes above
/// the classes take whole pages. Null on OOM or canary damage.
pub fn kmalloc(size: usize) -> *mut u8 {
    if size == 0 {
        return null_mut();
    }
    if slub::class_for(size).is_none() {
        // Above the largest class: whole zeroed page(s) go through kalloc.
        if size > PGSIZE {
            return null_mut();
        }
        let p = {
            let _guard = KMEM_LOCK.lock();
            alloc_page_locked()
        };
        return match p {
            Some(p) => {
                memset(p, 0, PGSIZE);
                p
            }
            None => null_mut(),
        };
    }
    let _guard = KMEM_LOCK.lock();
    let mut page_src = || match alloc_page_locked() {
        Some(p) => {
            memset(p, 0, PGSIZE);
            p
        }
        None => null_mut(),
    };
    slub::kmalloc_core(slub_mut(), size, &mut page_src)
}

/// kmfree: return one kmalloc object (or a whole-page kmalloc pointer).
/// False on non-slab pointer or double free.
pub fn kmfree(ptr: *mut u8) -> bool {
    let _guard = KMEM_LOCK.lock();
    slub::kfree_core(slub_mut(), ptr)
}

/// M2 suite support: full seeding invariant — a pfn sits inside the
/// buddy iff its page lies wholly in a usable region.
pub fn verify_seed() -> bool {
    let m = mem_map_mut();
    for pfn in 0..MAX_PFN {
        let pg = m[pfn];
        let inmap = bootmem::in_usable(pfn * PGSIZE, PGSIZE);
        let free = pg.flags & (buddy::PG_FREE | buddy::PG_RESERVED) == buddy::PG_FREE;
        if free != inmap {
            return false;
        }
        if free && pg.count != 0 {
            return false;
        }
    }
    true
}

/// M2 suite support: largest free order in the NORMAL zone.
pub fn largest_free_normal() -> Option<u8> {
    buddy::largest_free_order(&zones_mut()[1])
}

/// M2 evidence: (zone name, free order-0 pages) for both zones.
pub fn zone_summary() -> [(&'static str, usize); 2] {
    let z = zones_mut();
    [(z[0].name, z[0].free_pages), (z[1].name, z[1].free_pages)]
}

/// Free frames in order-0 units across both zones.
pub fn free_frames() -> usize {
    FREE_FRAMES.load(Ordering::SeqCst)
}
