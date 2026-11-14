//! frame: page-granular physical allocator, ported from xv6 kalloc.c.
//!
//! Free pages are threaded on a singly linked list whose `next` pointer
//! lives inside the freed page itself (xv6 `struct run`). xv6 keeps
//! `{spinlock lock; int use_lock; struct run *freelist}` in one struct;
//! here the same three fields become three statics (no OOP), and the
//! freelist head has a single storage location for both boot phases.

use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86::mm::layout::{kernel_end, v2p, PHYSTOP};
use crate::lib::string::memset;
use crate::lib::sync::Spinlock;

pub const PGSIZE: usize = 4096;

/// xv6 kmem.lock (guard object; the payload is unit).
static KMEM_LOCK: Spinlock<()> = Spinlock::new("kmem", ());

/// xv6 kmem.use_lock: false during phase-1 boot, true after kinit2.
static USE_LOCK: AtomicBool = AtomicBool::new(false);

/// xv6 kmem.freelist: head of the free-page chain (`struct run` overlay).
static mut FREELIST: *mut u8 = null_mut();

/// Phase 1 (kalloc.c:29): called from main while still on the boot page
/// tables; only the pages mapped by the trampoline go on the free list and
/// locking is off (single CPU, interrupts disabled).
pub fn kinit1(vstart: usize, vend: usize) {
    USE_LOCK.store(false, Ordering::SeqCst);
    freerange(vstart, vend);
}

/// Phase 2 (kalloc.c:39): add the rest of physical memory, enable locking.
pub fn kinit2(vstart: usize, vend: usize) {
    freerange(vstart, vend);
    USE_LOCK.store(true, Ordering::SeqCst);
}

/// freerange (kalloc.c:46): free every whole page in [vstart, vend).
pub fn freerange(vstart: usize, vend: usize) {
    let mut p = vstart & !(PGSIZE - 1); // PGROUNDUP(vstart)
    while p + PGSIZE <= vend {
        kfree(p as *mut u8);
        p += PGSIZE;
    }
}

/// kfree (kalloc.c:56): free the page at virtual `v`. Junk-fills it to
/// catch dangling references. Panics on obviously bogus pointers.
pub fn kfree(v: *mut u8) {
    let vaddr = v as usize;
    if vaddr % PGSIZE != 0 || vaddr < kernel_end() || v2p(vaddr) >= PHYSTOP {
        panic!("kfree: bad page {:#x}", vaddr);
    }

    // Fill with junk to catch dangling refs.
    memset(v, 1, PGSIZE);

    let guard = if USE_LOCK.load(Ordering::SeqCst) {
        Some(KMEM_LOCK.lock())
    } else {
        None
    };
    // SAFETY: v is a whole aligned page we still own; storing a pointer at
    // its start is the xv6 `struct run` overlay. FREELIST is only touched
    // here and in kalloc, under KMEM_LOCK whenever use_lock is set.
    unsafe {
        *(v as *mut *mut u8) = FREELIST;
        FREELIST = v;
    }
    drop(guard);
}

/// kalloc (kalloc.c:81): allocate one 4096-byte page; null on exhaustion.
pub fn kalloc() -> *mut u8 {
    let guard = if USE_LOCK.load(Ordering::SeqCst) {
        Some(KMEM_LOCK.lock())
    } else {
        None
    };
    // SAFETY: the head (if non-null) came from kfree's run overlay; its
    // first word is the next link. Popping keeps FREELIST consistent.
    let r = unsafe {
        let r = FREELIST;
        if !r.is_null() {
            FREELIST = *(r as *mut *mut u8);
        }
        r
    };
    drop(guard);
    r
}
