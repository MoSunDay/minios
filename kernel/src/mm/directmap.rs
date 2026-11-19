//! Full direct map: every probed physical page reachable at DIRECT_VMA
//! through 2 MiB supervisor RW NX huge pages, built up at boot from the
//! loader-probed memory top (the boot tables pre-map the first 256 MiB;
//! this module extends and verifies coverage up to probed_top). Also the
//! validated p2v/v2p entry points (bounds + canonical checks).

use crate::arch::x86_64::io;
use crate::arch::x86_64::{DIRECT_VMA, PHYS_TOP};
use crate::mm::frame::{p2v, v2p, PGSIZE};
use crate::mm::pgtable::{boot_pml4, PTE_NX, PTE_P, PTE_W};

/// PML4 slot of the direct map (DIRECT_VMA >> 39).
pub const DM_PML4_IDX: usize = (DIRECT_VMA >> 39) & 0x1ff;
const PDE_PS: u64 = 1 << 7;
const PDE_ADDR: u64 = 0x000f_ffff_ffff_f000;
const PDPT_SPAN: usize = 1 << 30; // 1 GiB per PDPT entry
const PDE_BYTES: usize = 2 * 1024 * 1024; // 2 MiB per PDE

/// Physical top actually covered by the direct map.
static mut DM_TOP: usize = 0;

pub fn dm_top() -> usize {
    // SAFETY: written once at init, read afterwards.
    unsafe { DM_TOP }
}

/// Ensure [0, probed_top) is covered by 2 MiB huge PDEs in the boot
/// PML4's direct-map slot, allocating page directories from the buddy as
/// needed. Idempotent; safe to call with the boot 256 MiB pre-map only.
pub fn init(probed_top: usize) {
    let top = probed_top.min(PHYS_TOP);
    // SAFETY: page-table pages reached through the direct map; single
    // CPU, interrupts off at boot.
    unsafe {
        let pml4 = boot_pml4();
        let pdpt = p2v(((*pml4.add(DM_PML4_IDX)) & PDE_ADDR) as usize) as *mut u64;
        let npd = top.div_ceil(PDPT_SPAN);
        for gi in 0..npd {
            if *pdpt.add(gi) & PTE_P == 0 {
                let pd = crate::mm::frame::kalloc();
                if pd.is_null() {
                    panic!("directmap: no page for PD");
                }
                crate::klib::string::memset(pd, 0, PGSIZE);
                *pdpt.add(gi) = v2p(pd as usize) as u64 | PTE_P | PTE_W | PTE_NX;
                io::invlpg((DIRECT_VMA + gi * PDPT_SPAN) as u64);
            }
            let pd = p2v(((*pdpt.add(gi)) & PDE_ADDR) as usize) as *mut u64;
            let n = (top - gi * PDPT_SPAN).div_ceil(PDE_BYTES);
            for pi in 0..n.min(512) {
                let pa = gi * PDPT_SPAN + pi * PDE_BYTES;
                *pd.add(pi) = (pa as u64 & PDE_ADDR) | PTE_P | PTE_W | PTE_NX | PDE_PS;
            }
        }
    }
    // SAFETY: boot-context write.
    unsafe { DM_TOP = top };
}

/// True when va is a canonical x86-64 address (either half).
pub fn canonical(va: usize) -> bool {
    let hi = (va as u64) >> 47;
    hi == 0 || hi == 0x1ffff
}

/// Bounds-checked p2v: Some(va) iff pa is inside the mapped direct map.
pub fn p2v_checked(pa: usize) -> Option<usize> {
    if pa < dm_top() {
        Some(pa + DIRECT_VMA)
    } else {
        None
    }
}

/// Bounds + canonical checked v2p: Some(pa) iff va is canonical and
/// inside the mapped direct-map window.
pub fn v2p_checked(va: usize) -> Option<usize> {
    if !canonical(va) || va < DIRECT_VMA {
        return None;
    }
    let pa = va - DIRECT_VMA;
    if pa < dm_top() {
        Some(pa)
    } else {
        None
    }
}

/// Walk to the 2 MiB PDE covering va in the direct map (huge page);
/// returns the raw entry (0 when absent).
/// SAFETY: caller passes any va; the walk only touches the kernel tables.
pub unsafe fn dm_pde(va: usize) -> u64 {
    unsafe {
        let pml4 = boot_pml4();
        let pdpt = p2v(((*pml4.add(DM_PML4_IDX)) & PDE_ADDR) as usize) as *mut u64;
        let pdpte = *pdpt.add((va >> 30) & 0x1ff);
        if pdpte & PTE_P == 0 {
            return 0;
        }
        let pd = p2v((pdpte & PDE_ADDR) as usize) as *mut u64;
        *pd.add((va >> 21) & 0x1ff)
    }
}
