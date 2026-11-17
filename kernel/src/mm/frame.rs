//! Physical frame allocator over the Multiboot memory map (xv6 kalloc.c
//! semantics, 64-bit): free pages threaded through their first word.
//!
//! All RAM is also reachable through the fixed direct map at DIRECT_VMA,
//! so kernel pointers to physical pages never depend on the current CR3.

use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::x86_64::{DIRECT_VMA, PHYS_TOP};
use crate::klib::string::memset;
use crate::klib::sync::Spinlock;

pub const PGSIZE: usize = 4096;

static KMEM_LOCK: Spinlock<()> = Spinlock::new("kmem", ());
static mut FREELIST: *mut u8 = null_mut();
static FREE_FRAMES: AtomicUsize = AtomicUsize::new(0);
/// Below this physical address everything is boot/kernel/reserved.
static mut FLOOR_PA: usize = 0;

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

/// Seed the free list from the loader memory map: every whole page that is
/// (a) available, (b) at or above the kernel image end, (c) inside the
/// direct-mapped window. Called once at boot with interrupts off.
pub fn init(mbi_phys: usize) {
    // SAFETY: single-threaded boot context; static mut writes.
    unsafe {
        FLOOR_PA = (kernel_phys_end() + PGSIZE - 1) & !(PGSIZE - 1);
        FREELIST = null_mut();
    }
    crate::multiboot::for_each_available(mbi_phys, |base, len| {
        let start = ((base as usize) + PGSIZE - 1) & !(PGSIZE - 1);
        let end = ((base + len) as usize).min(PHYS_TOP);
        free_range(start, end);
    });
}

fn free_range(start: usize, end: usize) {
    let floor = unsafe { FLOOR_PA };
    let mut p = start.max(floor);
    while p + PGSIZE <= end {
        // SAFETY: page inside the direct map and above the kernel image.
        unsafe { push_free(p2v(p) as *mut u8) };
        p += PGSIZE;
    }
}

/// kfree: junk-fill and push one page (direct-map pointer).
/// SAFETY: the page must not be referenced anywhere else.
pub unsafe fn push_free(v: *mut u8) {
    memset(v, 1, PGSIZE);
    let _guard = KMEM_LOCK.lock();
    // SAFETY: whole aligned page we own; the first word becomes the link.
    unsafe {
        *(v as *mut *mut u8) = FREELIST;
        FREELIST = v;
    }
    FREE_FRAMES.fetch_add(1, Ordering::SeqCst);
}

#[allow(dead_code)]
pub fn free_frames() -> usize {
    FREE_FRAMES.load(Ordering::SeqCst)
}

/// kalloc: allocate one zeroed?? (xv6: not zeroed; callers memset) page.
pub fn kalloc() -> *mut u8 {
    let _guard = KMEM_LOCK.lock();
    // SAFETY: see push_free; the head link is valid while the lock is held.
    let r = unsafe {
        let r = FREELIST;
        if !r.is_null() {
            FREELIST = *(r as *mut *mut u8);
        }
        r
    };
    if !r.is_null() {
        FREE_FRAMES.fetch_sub(1, Ordering::SeqCst);
    }
    r
}

pub fn kfree(v: *mut u8) {
    let pa = v2p(v as usize);
    if v as usize % PGSIZE != 0 || pa < unsafe { FLOOR_PA } || pa >= PHYS_TOP {
        panic!("kfree: bad page {:#x}", v as usize);
    }
    // SAFETY: validated page.
    unsafe { push_free(v) };
}
