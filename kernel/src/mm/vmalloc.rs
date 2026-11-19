//! Kernel vmalloc area: non-contiguous physical pages mapped contiguously
//! into the kernel-only VA window at 0xffff_9000_0000_0000 (1 GiB span,
//! one PML4 slot, shared into every address space like the direct map).
//! Provides vmalloc/vmap/vfree plus the M3-facing ioremap skeleton.
//!
//! M2 lifetime rules: every page-directory/table page installed for the
//! area is permanent (never returned to the buddy — upper levels are
//! shared into all aspaces); vfree unmaps leaf PTEs (invlpg each) and,
//! when the area owns its pages, frees each physical page exactly once
//! (the area leaves the table before any free, so double vfree is
//! rejected with the frame count untouched).

use crate::arch::x86_64::io;
use crate::arch::x86_64::VMALLOC_PML4_IDX;
use crate::klib::sync::Spinlock;
use crate::mm::frame::{kalloc, kfree, v2p, PGSIZE};
use crate::mm::pgtable::{self, boot_pml4, PTE_NX, PTE_P, PTE_W};

pub const VMALLOC_START: usize = 0xffff_9000_0000_0000;
pub const VMALLOC_SIZE: usize = 0x4000_0000; // 1 GiB
pub const VMALLOC_END: usize = VMALLOC_START + VMALLOC_SIZE;
/// Caller-set permission flag: allow execution (NX cleared).
pub const VM_EXEC: u64 = 1 << 0;
/// Read-write (mapped NX unless VM_EXEC).
pub const VM_RW: u64 = PTE_W;
/// Read-only (NX, no W).
pub const VM_RO: u64 = 0;

const _: () = assert!((VMALLOC_START >> 39) & 0x1ff == VMALLOC_PML4_IDX);

const MAX_AREAS: usize = 32;
/// One unmapped guard page between areas (overrun containment).
const GUARD_PAGES: usize = 1;

#[derive(Clone, Copy)]
struct Area {
    start: usize,
    npages: usize,
    /// vmalloc'd areas free their pages on vfree; vmap/ioremap areas
    /// never touch page ownership.
    owns: bool,
}

static VLOCK: Spinlock<()> = Spinlock::new("vmalloc", ());
static mut AREAS: [Option<Area>; MAX_AREAS] = [None; MAX_AREAS];

fn areas_mut() -> &'static mut [Option<Area>] {
    // SAFETY: guarded by VLOCK (and boot context before scheduling).
    unsafe { &mut *core::ptr::addr_of_mut!(AREAS) }
}

/// First-fit allocation with guard gaps; returns the area start VA.
fn alloc_area(npages: usize, owns: bool) -> Option<usize> {
    if npages == 0 || npages * PGSIZE > VMALLOC_SIZE {
        return None;
    }
    let want = (npages + GUARD_PAGES) * PGSIZE;
    let mut spans: [(usize, usize); MAX_AREAS] = [(0, 0); MAX_AREAS];
    let mut n = 0usize;
    for a in areas_mut().iter().flatten() {
        spans[n] = (a.start, a.start + (a.npages + GUARD_PAGES) * PGSIZE);
        n += 1;
    }
    spans[..n].sort_unstable();
    let mut candidate = VMALLOC_START;
    for &(start, end) in &spans[..n] {
        if start > candidate && start - candidate >= want {
            break; // gap in front of this span fits
        }
        candidate = candidate.max(end);
    }
    if candidate + want > VMALLOC_END {
        return None;
    }
    for slot in areas_mut().iter_mut() {
        if slot.is_none() {
            *slot = Some(Area {
                start: candidate,
                npages,
                owns,
            });
            return Some(candidate);
        }
    }
    None
}

/// Remove the area starting at va; false when absent (double vfree).
fn free_area(va: usize) -> Option<Area> {
    for slot in areas_mut().iter_mut() {
        if let Some(a) = *slot {
            if a.start == va {
                *slot = None;
                return Some(a);
            }
        }
    }
    None
}

/// Install one leaf PTE into the boot kernel tables (allocating PT pages
/// on demand; they are permanent).
/// SAFETY: va inside the vmalloc window; pa a valid frame or device base.
unsafe fn map_one(va: usize, pa: usize, perm: u64) -> bool {
    unsafe {
        let pte = pgtable::walk(boot_pml4(), va, true);
        if pte.is_null() {
            return false;
        }
        if *pte & PTE_P != 0 {
            return false; // remap
        }
        let bits = PTE_P | (perm & PTE_W) | if perm & VM_EXEC != 0 { 0 } else { PTE_NX };
        *pte = (pa as u64 & pgtable::PTE_ADDR) | bits;
        io::invlpg(va as u64);
        true
    }
}

/// Unmap [va, va+npages*PGSIZE): clears each leaf PTE (invlpg); frees the
/// physical frames when `owns`. Returns the number of frames freed
/// (each frame exactly once).
/// SAFETY: va is an active area start.
unsafe fn unmap_range(va: usize, npages: usize, owns: bool) -> usize {
    let mut freed = 0usize;
    for i in 0..npages {
        let a = va + i * PGSIZE;
        unsafe {
            let pte = pgtable::walk(boot_pml4(), a, false);
            if !pte.is_null() && *pte & PTE_P != 0 {
                if owns {
                    kfree(p2v_of(*pte));
                    freed += 1;
                }
                *pte = 0;
                io::invlpg(a as u64);
            }
        }
    }
    freed
}

/// SAFETY-helper: PTE -> direct-map page pointer.
unsafe fn p2v_of(pte: u64) -> *mut u8 {
    crate::mm::frame::p2v((pte & pgtable::PTE_ADDR) as usize) as *mut u8
}

/// vmalloc: npages fresh zeroed frames mapped contiguously at `perm`
/// (VM_RW / VM_RO / | VM_EXEC). Returns the VA, 0 on failure (all state
/// rolled back).
pub fn vmalloc(npages: usize, perm: u64) -> usize {
    let _g = VLOCK.lock();
    let Some(va) = alloc_area(npages, true) else {
        return 0;
    };
    let mut mapped = 0usize;
    for i in 0..npages {
        let p = kalloc();
        if p.is_null() {
            break;
        }
        // SAFETY: fresh frame; VA inside the vmalloc window.
        unsafe {
            crate::klib::string::memset(p, 0, PGSIZE);
            if !map_one(va + i * PGSIZE, v2p(p as usize), perm) {
                kfree(p);
                break;
            }
        }
        mapped += 1;
    }
    if mapped == npages {
        return va;
    }
    // SAFETY: va is the active area start; roll back what was mapped.
    unsafe { unmap_range(va, mapped, true) };
    free_area(va);
    0
}

/// vmap: map caller-owned physical frames contiguously (the caller keeps
/// page ownership; vfree on this VA never frees them).
pub fn vmap(pas: &[usize], perm: u64) -> usize {
    let _g = VLOCK.lock();
    let Some(va) = alloc_area(pas.len(), false) else {
        return 0;
    };
    for (i, &pa) in pas.iter().enumerate() {
        // SAFETY: VA inside the window; pa caller-owned.
        if !unsafe { map_one(va + i * PGSIZE, pa, perm) } {
            // SAFETY: rollback (owns=false: no frames touched).
            unsafe { unmap_range(va, i, false) };
            free_area(va);
            return 0;
        }
    }
    va
}

/// vfree / iounmap: unmap one area; owned frames are freed exactly once.
/// False (nothing touched) when va is not an active area start.
pub fn vfree(va: usize) -> bool {
    let _g = VLOCK.lock();
    let Some(area) = free_area(va) else {
        return false;
    };
    // SAFETY: area came from the table; leaf PTEs cleared page by page.
    unsafe { unmap_range(area.start, area.npages, area.owns) };
    true
}

/// ioremap skeleton (M3 APIC/MMIO dependency): map device memory [pa,
/// pa+len) into the vmalloc window RW NX. RAM ranges are rejected —
/// kernel RAM is reachable through the direct map already.
pub fn ioremap(pa: usize, len: usize) -> usize {
    if crate::mm::bootmem::in_usable(pa, len) {
        return 0;
    }
    let npages = len.div_ceil(PGSIZE);
    let _g = VLOCK.lock();
    let Some(va) = alloc_area(npages, false) else {
        return 0;
    };
    for i in 0..npages {
        // SAFETY: VA inside the window; MMIO frame.
        if !unsafe { map_one(va + i * PGSIZE, pa + i * PGSIZE, VM_RW) } {
            unsafe { unmap_range(va, i, false) };
            free_area(va);
            return 0;
        }
    }
    va
}

/// iounmap: vfree on an ioremap area (no frames owned).
pub fn iounmap(va: usize) -> bool {
    vfree(va)
}
