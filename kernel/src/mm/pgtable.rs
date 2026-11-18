//! Four-level page tables (xv6 vm.c semantics, x86_64).
//!
//! Kernel half of every address space is shared with the boot tables:
//!   PML4[509] direct map (all RAM), PML4[510..511] kernel image + high half.
//! User half (PML4[0..256]) is per-process. While a user CR3 is loaded the
//! kernel reaches physical memory through the direct map only.

use crate::mm::frame::{kalloc, kfree, p2v, v2p, PGSIZE};

pub const PTE_P: u64 = 1 << 0;
pub const PTE_W: u64 = 1 << 1;
pub const PTE_U: u64 = 1 << 2;
pub const PTE_NX: u64 = 1 << 63;
pub const PTE_ADDR: u64 = 0x000f_ffff_ffff_f000;

pub const NPTENTRIES: usize = 512;

#[inline]
pub fn pgrounddown(a: usize) -> usize {
    a & !(PGSIZE - 1)
}
#[inline]
pub fn pgroundup(a: usize) -> usize {
    (a + PGSIZE - 1) & !(PGSIZE - 1)
}

/// Direct pointer to a page-table page (direct-map address).
pub type PtPtr = *mut u64;

/// View a page-table page as its entry array (raw pointer cast only).
#[inline]
fn entries(table: PtPtr) -> *mut [u64; NPTENTRIES] {
    table as *mut [u64; NPTENTRIES]
}

/// Physical address of the boot PML4 (set once by main from CR3).
static mut BOOT_PML4_PA: usize = 0;

pub fn set_boot_pml4(pa: usize) {
    // SAFETY: boot-time, single CPU.
    unsafe { BOOT_PML4_PA = pa };
}

pub fn boot_pml4() -> PtPtr {
    // SAFETY: page-table page reached via the direct map.
    p2v(unsafe { BOOT_PML4_PA }) as PtPtr
}

/// Physical address of the boot PML4 (the scheduler's idle CR3).
pub fn boot_pml4_pa() -> usize {
    // SAFETY: boot-time write, read afterwards.
    unsafe { BOOT_PML4_PA }
}

/// A fresh PML4 sharing the kernel half with the boot tables; null on OOM.
pub fn setup_user_pml4() -> PtPtr {
    let pml4 = kalloc() as PtPtr;
    if pml4.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: freshly allocated, page-aligned.
    unsafe {
        (*entries(pml4)).fill(0);
        let boot = boot_pml4();
        // Share the kernel half: direct map (510) and kernel image (511).
        for i in 510..512 {
            (*entries(pml4))[i] = (*entries(boot))[i];
        }
    }
    pml4
}

/// walk: return the PTE for `va` in `pml4`, allocating intermediate levels
/// when `alloc` is set; null when missing or on OOM.
/// SAFETY: pml4 must be a valid table pointer.
pub unsafe fn walk(pml4: PtPtr, va: usize, alloc: bool) -> PtPtr {
    let mut table = pml4;
    for level in (1..=3).rev() {
        let idx = (va >> (12 + 9 * level)) & 0x1ff;
        // SAFETY: table pointers are page-aligned table pages.
        let pte = unsafe { table.add(idx) };
        let entry = unsafe { *pte };
        if entry & PTE_P != 0 {
            table = p2v((entry & PTE_ADDR) as usize) as PtPtr;
        } else {
            if !alloc {
                return core::ptr::null_mut();
            }
            let page = kalloc() as PtPtr;
            if page.is_null() {
                return core::ptr::null_mut();
            }
            // SAFETY: fresh page.
            unsafe {
                (*entries(page)).fill(0);
                *pte = v2p(page as usize) as u64 | PTE_P | PTE_W | PTE_U;
            }
            table = page;
        }
    }
    table.add((va >> 12) & 0x1ff)
}

/// mappages: map [va, va+size) at pa with `perm`; -1 on failure.
/// SAFETY: pml4 valid; va/pa/size page-consistent; pages not double mapped.
pub unsafe fn mappages(pml4: PtPtr, va: usize, size: usize, pa: usize, perm: u64) -> i32 {
    let mut a = pgrounddown(va);
    let last = pgrounddown(va + size - 1);
    while a <= last {
        let pte = unsafe { walk(pml4, a, true) };
        if pte.is_null() {
            return -1;
        }
        if unsafe { *pte } & PTE_P != 0 {
            panic!("mappages: remap {:#x}", a);
        }
        unsafe { *pte = (pa + (a - pgrounddown(va))) as u64 | perm | PTE_P };
        a += PGSIZE;
    }
    0
}

#[allow(dead_code)]
/// Unmap and free the page under va (no-op when absent).
/// SAFETY: pml4 valid.
pub unsafe fn unmap_page(pml4: PtPtr, va: usize) {
    let pte = unsafe { walk(pml4, va, false) };
    if pte.is_null() {
        return;
    }
    let entry = unsafe { *pte };
    if entry & PTE_P == 0 {
        return;
    }
    kfree(p2v((entry & PTE_ADDR) as usize) as *mut u8);
    unsafe { *pte = 0 };
}

/// Free a user PML4 and every mapped user page (freevm). Kernel-half entries
/// are shared and must not be freed.
/// SAFETY: pml4 came from setup_user_pml4 and is no longer in CR3.
pub unsafe fn free_user_pml4(pml4: PtPtr) {
    unsafe {
        for i in 0..256usize {
            let pdpte = (*entries(pml4))[i];
            if pdpte & PTE_P == 0 {
                continue;
            }
            let pdpt = p2v((pdpte & PTE_ADDR) as usize) as PtPtr;
            for j in 0..NPTENTRIES {
                let pde = (*entries(pdpt))[j];
                if pde & PTE_P == 0 {
                    continue;
                }
                let pd = p2v((pde & PTE_ADDR) as usize) as PtPtr;
                for k in 0..NPTENTRIES {
                    let pte = (*entries(pd))[k];
                    if pte & PTE_P == 0 {
                        continue;
                    }
                    kfree(p2v((pte & PTE_ADDR) as usize) as *mut u8);
                }
                kfree(pd as *mut u8);
            }
            kfree(pdpt as *mut u8);
        }
        kfree(pml4 as *mut u8);
    }
}

#[allow(dead_code)]
/// Copy the user half of `src` into a fresh PML4 (fork, no COW in M1R).
/// SAFETY: src valid; user mappings only live in the low half.
pub unsafe fn copy_user_half(src: PtPtr) -> PtPtr {
    let dst = setup_user_pml4();
    if dst.is_null() {
        return core::ptr::null_mut();
    }
    let mut failed = false;
    unsafe {
        'outer: for i in 0..256usize {
            let pdpte = (*entries(src))[i];
            if pdpte & PTE_P == 0 {
                continue;
            }
            let sp = p2v((pdpte & PTE_ADDR) as usize) as PtPtr; // pdpt
            for j in 0..NPTENTRIES {
                let pde = (*entries(sp))[j];
                if pde & PTE_P == 0 {
                    continue;
                }
                let spd = p2v((pde & PTE_ADDR) as usize) as PtPtr; // pd
                for k in 0..NPTENTRIES {
                    let pte = (*entries(spd))[k];
                    if pte & PTE_P == 0 {
                        continue;
                    }
                    let spt = p2v((pte & PTE_ADDR) as usize) as PtPtr; // pt
                    for l in 0..NPTENTRIES {
                        let spe = (*entries(spt))[l];
                        if spe & PTE_P == 0 {
                            continue;
                        }
                        let newp = kalloc();
                        if newp.is_null() {
                            free_user_pml4(dst);
                            failed = true;
                            break 'outer;
                        }
                        crate::klib::string::memmove(
                            newp,
                            p2v((spe & PTE_ADDR) as usize) as *mut u8,
                            PGSIZE,
                        );
                        let va = ((i as u64) << 39
                            | (j as u64) << 30
                            | (k as u64) << 21
                            | (l as u64) << 12) as usize;
                        if mappages(
                            dst,
                            va,
                            PGSIZE,
                            v2p(newp as usize),
                            spe & (PTE_W | PTE_U | PTE_NX),
                        ) < 0
                        {
                            free_user_pml4(dst);
                            failed = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
    if failed {
        return core::ptr::null_mut();
    }
    dst
}
