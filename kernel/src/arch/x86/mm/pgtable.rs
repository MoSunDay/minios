//! pgtable: two-level page tables, ported from xv6 vm.c.
//!
//! There is one page table per process plus kpgdir used while no process
//! runs. setupkvm()/exec() build every table like this (vm.c:95):
//!   0..KERNBASE                  user memory
//!   KERNBASE..KERNBASE+EXTMEM    -> 0..EXTMEM (I/O space: VGA, PIC, PIT)
//!   kernel image                 -> its own physical pages
//!   kernel_end..KERNBASE+PHYSTOP -> free physical memory (rw)
//!   0xFE000000..0                -> direct device space
//!
//! Divergences from xv6: (1) our linker script has no `data` symbol, so
//! the kernel image is mapped RW in one piece instead of splitting
//! text+rodata (RO) from data (RW); (2) EXTMEM is 0x110000 (kernel LMA)
//! instead of xv6's 0x100000 because the low trampoline occupies
//! 0x100000..0x110000.
//!
//! Signatures follow the integration contract in kernel/sched/mod.rs:
//! unsafe fns, `*mut u32` pgdirs, u32 addresses.

use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::x86::kernel::idt;
use crate::arch::x86::kernel::Trapframe;
use crate::arch::x86::mm::layout::{
    kernel_end, kernel_start, p2v, v2p, DEVSPACE, KERNBASE, PHYSTOP,
};
use crate::lib::string::{memmove, memset};
use crate::lib::sync::{pop_intr_off, push_intr_off};
use crate::mm::frame::{kalloc, kfree, PGSIZE};

// ---- mmu.h ------------------------------------------------------------

pub type Pde = u32; // page directory entry
pub type Pte = u32; // page table entry

pub const NPTENTRIES: usize = 1024;
pub const NPDENTRIES: usize = 1024;

pub const PTE_P: u32 = 0x001; // present
pub const PTE_W: u32 = 0x002; // writeable
pub const PTE_U: u32 = 0x004; // user
#[allow(dead_code)] // mmu.h flag, kept for 4 MiB mappings
pub const PTE_PS: u32 = 0x080; // page size

pub const EXTMEM: usize = 0x110_000; // start of extended memory (kernel LMA)

#[inline]
pub fn pgrounddown(a: usize) -> usize {
    a & !(PGSIZE - 1)
}
#[inline]
pub fn pgroundup(a: usize) -> usize {
    (a + PGSIZE - 1) & !(PGSIZE - 1)
}
#[inline]
fn pdx(va: usize) -> usize {
    (va >> 22) & 0x3FF
}
#[inline]
fn ptx(va: usize) -> usize {
    (va >> 12) & 0x3FF
}
#[inline]
pub fn pte_addr(pte: u32) -> usize {
    (pte & !0xFFF) as usize
}
#[inline]
pub fn pte_flags(pte: u32) -> u32 {
    pte & 0xFFF
}

// ---- control registers (x86.h) -----------------------------------------

/// lcr3 (x86.h:143).
///
/// SAFETY: caller must pass a valid page-directory physical address that
/// keeps the current instruction stream mapped.
pub unsafe fn lcr3(val: usize) {
    unsafe { core::arch::asm!("movl %eax, %cr3", in("eax") val as u32, options(att_syntax)) };
}

/// rcr3 (x86.h:147).
#[allow(dead_code)] // x86.h helper; used by future debug/copy-on-write code
pub fn rcr3() -> usize {
    let val: u32;
    // SAFETY: cr3 read has no side effects.
    unsafe { core::arch::asm!("movl %cr3, %eax", out("eax") val, options(nomem, att_syntax)) };
    val as usize
}

/// rcr2 (x86.h:136): page fault linear address.
pub fn rcr2() -> usize {
    let val: u32;
    // SAFETY: cr2 read has no side effects.
    unsafe { core::arch::asm!("movl %cr2, %eax", out("eax") val, options(nomem, att_syntax)) };
    val as usize
}

// ---- kernel page directory (vm.c:10) ------------------------------------

static KPGDIR: AtomicUsize = AtomicUsize::new(0);

/// Address of the kernel page directory (null before kvmalloc).
pub fn kpgdir() -> *mut Pde {
    KPGDIR.load(Ordering::SeqCst) as *mut Pde
}

// ---- walkers (vm.c:44, vm.c:70) -----------------------------------------

/// walkpgdir (vm.c:44): PTE slot for `va`; builds page-table pages when
/// `alloc != 0`. Null when a table is needed but cannot be allocated.
///
/// SAFETY: pgdir must be a live page directory.
pub unsafe fn walkpgdir(pgdir: *mut Pde, va: u32, alloc: i32) -> *mut Pte {
    unsafe {
        let va = va as usize;
        let pde = pgdir.add(pdx(va));
        let pgtab: *mut Pte;
        if *pde & PTE_P != 0 {
            pgtab = p2v(pte_addr(*pde)) as *mut Pte;
        } else {
            let page = if alloc != 0 { kalloc() } else { null_mut() };
            if page.is_null() {
                return null_mut();
            }
            // Make sure all those PTE_P bits are zero.
            memset(page, 0, PGSIZE);
            // Permissions here are overly generous; PTEs restrict further.
            *pde = (v2p(page as usize) as u32) | PTE_P | PTE_W | PTE_U;
            pgtab = page as *mut Pte;
        }
        pgtab.add(ptx(va))
    }
}

/// mappages (vm.c:70): create PTEs mapping [va, va+size) -> [pa, ...);
/// neither end need be aligned. 0 on success, -1 on failure.
///
/// SAFETY: pgdir must be a live page directory; panics on remap.
pub unsafe fn mappages(pgdir: *mut Pde, va: u32, size: u32, pa: u32, perm: u32) -> i32 {
    unsafe {
        let mut a = pgrounddown(va as usize);
        // uint-wrap arithmetic matches xv6 (DEVSPACE row wraps phys_end).
        let last = pgrounddown(va.wrapping_add(size).wrapping_sub(1) as usize);
        let mut pa = pa as usize;
        loop {
            let pte = walkpgdir(pgdir, a as u32, 1);
            if pte.is_null() {
                return -1;
            }
            if *pte & PTE_P != 0 {
                panic!("remap");
            }
            *pte = (pa as u32) | perm | PTE_P;
            if a == last {
                break;
            }
            a += PGSIZE;
            pa += PGSIZE;
        }
    }
    0
}

// ---- kernel address space (vm.c:115..153) -------------------------------

/// One `struct kmap` row (vm.c:115).
struct Kmap {
    virt: usize,
    phys_start: usize,
    phys_end: usize,
    perm: u32,
}

/// The kernel's mappings, present in every page table (vm.c:120).
fn kmap() -> [Kmap; 4] {
    let klink = kernel_start(); // xv6 KERNLINK: first kernel VMA
    [
        // I/O space (VGA 0xB8000, PIC, PIT live below 0x110000)
        Kmap {
            virt: KERNBASE,
            phys_start: 0,
            phys_end: EXTMEM,
            perm: PTE_W,
        },
        // kernel text+rodata+data (xv6 splits RO text at `data`)
        Kmap {
            virt: klink,
            phys_start: v2p(klink),
            phys_end: v2p(kernel_end()),
            perm: PTE_W,
        },
        // free physical memory
        Kmap {
            virt: kernel_end(),
            phys_start: v2p(kernel_end()),
            phys_end: PHYSTOP,
            perm: PTE_W,
        },
        // more devices: DEVSPACE..4GB (phys_end wraps at 0)
        Kmap {
            virt: DEVSPACE,
            phys_start: DEVSPACE,
            phys_end: 0,
            perm: PTE_W,
        },
    ]
}

/// setupkvm (vm.c:128): kernel part of a page table; null on failure.
pub unsafe fn setupkvm() -> *mut Pde {
    unsafe {
        if p2v(PHYSTOP) > DEVSPACE {
            panic!("PHYSTOP too high");
        }
        let pgdir = kalloc() as *mut Pde;
        if pgdir.is_null() {
            return null_mut();
        }
        memset(pgdir as *mut u8, 0, PGSIZE);
        for k in kmap().iter() {
            let size = k.phys_end.wrapping_sub(k.phys_start);
            if mappages(
                pgdir,
                k.virt as u32,
                size as u32,
                k.phys_start as u32,
                k.perm,
            ) < 0
            {
                return null_mut();
            }
        }
        pgdir
    }
}

/// kvmalloc (vm.c:149): build kpgdir and load cr3. The boot CPU already
/// runs on the trampoline page directory, so — exactly like xv6 — this
/// just switches; nothing is copied.
pub unsafe fn kvmalloc() {
    unsafe {
        let pgdir = setupkvm();
        KPGDIR.store(pgdir as usize, Ordering::SeqCst);
        switchkvm();
    }
}

/// switchkvm (vm.c:157): switch the h/w page table register to kpgdir.
pub unsafe fn switchkvm() {
    unsafe { lcr3(v2p(kpgdir() as usize)) };
}

/// switchuvm (vm.c:163): switch this CPU's TSS and address space to a
/// user process. `tf` is the process's trap frame at the top of its
/// kernel stack (xv6 takes `struct proc*`; esp0 is tf + sizeof(tf),
/// i.e. kstack + KSTACKSIZE).
///
/// SAFETY: `pgdir` must be a live user page directory and `tf` a frame
/// placed at the top of the proc's kstack.
pub unsafe fn switchuvm(pgdir: *mut Pde, tf: *const Trapframe) {
    unsafe {
        push_intr_off();
        idt::load_tss(tf as usize + core::mem::size_of::<Trapframe>());
        if pgdir.is_null() {
            panic!("switchuvm: no pgdir");
        }
        lcr3(v2p(pgdir as usize));
        pop_intr_off();
    }
}

// ---- user address space (vm.c:215..303) ---------------------------------

/// allocuvm (vm.c:215): grow a process from oldsz to newsz (unaligned ok).
/// Returns the new size, or 0 on error.
///
/// SAFETY: pgdir must be a live user page directory.
pub unsafe fn allocuvm(pgdir: *mut Pde, oldsz: u32, newsz: u32) -> u32 {
    unsafe {
        if newsz as usize >= KERNBASE {
            return 0;
        }
        if newsz < oldsz {
            return oldsz;
        }
        let mut a = pgroundup(oldsz as usize);
        while a < newsz as usize {
            let mem = kalloc();
            if mem.is_null() {
                crate::printk!("allocuvm out of memory\n");
                deallocuvm(pgdir, newsz, oldsz);
                return 0;
            }
            memset(mem, 0, PGSIZE);
            mappages(
                pgdir,
                a as u32,
                PGSIZE as u32,
                v2p(mem as usize) as u32,
                PTE_W | PTE_U,
            );
            a += PGSIZE;
        }
    }
    newsz
}

/// deallocuvm (vm.c:238): shrink from oldsz to newsz (neither aligned).
/// Returns newsz.
///
/// SAFETY: pgdir must be a live user page directory.
pub unsafe fn deallocuvm(pgdir: *mut Pde, oldsz: u32, newsz: u32) -> u32 {
    unsafe {
        if newsz >= oldsz {
            return oldsz;
        }
        let mut a = pgroundup(newsz as usize);
        while a < oldsz as usize {
            let pte = walkpgdir(pgdir, a as u32, 0);
            if pte.is_null() {
                a += (NPTENTRIES - 1) * PGSIZE; // skip a whole page table
            } else if *pte & PTE_P != 0 {
                let pa = pte_addr(*pte);
                if pa == 0 {
                    panic!("kfree");
                }
                kfree(p2v(pa) as *mut u8);
                *pte = 0;
            }
            a += PGSIZE;
        }
    }
    newsz
}

/// freevm (vm.c:266): free a page table and all the physical pages it
/// maps (user pages, page-table pages, and the directory itself).
pub unsafe fn freevm(pgdir: *mut Pde) {
    unsafe {
        if pgdir.is_null() {
            panic!("freevm: no pgdir");
        }
        deallocuvm(pgdir, KERNBASE as u32, 0);
        for i in 0..NPDENTRIES {
            if *pgdir.add(i) & PTE_P != 0 {
                let v = p2v(pte_addr(*pgdir.add(i)));
                kfree(v as *mut u8);
            }
        }
        kfree(pgdir as *mut u8);
    }
}

/// copyuvm (vm.c:281): duplicate a process's user address space of size
/// `sz` into a fresh page table (for fork); null on failure.
pub unsafe fn copyuvm(pgdir: *mut Pde, sz: u32) -> *mut Pde {
    unsafe {
        let d = setupkvm();
        if d.is_null() {
            return null_mut();
        }
        let mut i = 0usize;
        while i < sz as usize {
            let pte = walkpgdir(pgdir, i as u32, 0);
            if pte.is_null() {
                panic!("copyuvm: pte should exist");
            }
            if *pte & PTE_P == 0 {
                panic!("copyuvm: page not present");
            }
            let (pa, flags) = (pte_addr(*pte), pte_flags(*pte));
            let mem = kalloc();
            if mem.is_null() {
                freevm(d);
                return null_mut();
            }
            memmove(mem, p2v(pa) as *mut u8, PGSIZE);
            if mappages(d, i as u32, PGSIZE as u32, v2p(mem as usize) as u32, flags) < 0 {
                freevm(d);
                return null_mut();
            }
            i += PGSIZE;
        }
        d
    }
}

// copy helpers live in pgcopy.rs; re-exported for the integration
// contract paths (crate::arch::x86::mm::pgtable::{copyout, ...}).
pub use super::pgcopy::{clearpteu, copyin, copyout};
