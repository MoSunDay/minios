//! Buffer cache: port of `bio.c` (xv6-public).
//!
//! The buffer cache is a linked list of `Buf` structures holding cached
//! copies of disk block contents. Caching disk blocks in memory reduces the
//! number of disk reads and also provides a synchronization point for disk
//! blocks used by multiple processes.
//!
//! Interface:
//! * To get a buffer for a particular disk sector, call [`bread`].
//! * After changing buffer data, call [`bwrite`] to write it to disk.
//! * When done with the buffer, call [`brelse`].
//! * Do not use the buffer after calling brelse.
//! * Only one process at a time can use a buffer,
//!     so do not keep them longer than necessary.
//!
//! The implementation uses three state flags internally:
//! * B_BUSY: the block has been returned from bread
//!     and has not been passed back to brelse.
//! * B_VALID: the buffer data has been read from the disk.
//! * B_DIRTY: the buffer data has been modified
//!     and needs to be written to disk.
//!
//! Locking: xv6's `struct { spinlock lock; ... } bcache` is ported as a
//! `static mut BCACHE` with an embedded [`RawLock`], released/reacquired
//! through [`crate::kernel::sched::sleep`] exactly like bio.c.

use crate::drivers::block::ide::iderw;
use crate::fs::xv6fs::layout::{has_flag, BSIZE, NBUF};
use crate::kernel::sched::{acquire, release, sleep, wakeup, RawLock};

// buf.h: buffer state flags.
pub const B_BUSY: i32 = 0x1; // buffer is locked by some process
pub const B_VALID: i32 = 0x2; // buffer has been read from disk
pub const B_DIRTY: i32 = 0x4; // buffer needs to be written to disk

/// buf.h `struct buf`. Managed through raw pointers exactly like xv6
/// (LRU list via prev/next, disk queue via qnext).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Buf {
    pub flags: i32,
    pub dev: u32,
    pub sector: u32,
    /// LRU cache list (xv6: `prev`).
    pub prev: *mut Buf,
    /// LRU cache list (xv6: `next`).
    pub next: *mut Buf,
    /// Disk queue (xv6: `qnext`).
    pub qnext: *mut Buf,
    pub data: [u8; BSIZE],
}

impl Buf {
    const fn empty() -> Buf {
        Buf {
            flags: 0,
            dev: u32::MAX, // xv6: b->dev = -1
            sector: 0,
            prev: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
            qnext: core::ptr::null_mut(),
            data: [0; BSIZE],
        }
    }
}

/// bio.c's `bcache`: buffer array + LRU head (`head.next` is most recently
/// used). The lock is a sibling static (RawLock is Sync on its own; the
/// Buf array is not, so the data stays `static mut`), matching sched's
/// TICKS/TICKS_LOCK split of xv6's embedded-spinlock structs.
pub struct BCache {
    pub buf: [Buf; NBUF],
    pub head: Buf,
}

// SAFETY: every field is guarded by BCACHE_LOCK (plus the B_BUSY protocol
// between bget() and brelse()); the statics are unique.
static BCACHE_LOCK: RawLock = RawLock::new("bcache");
static mut BCACHE: BCache = BCache {
    buf: [Buf::empty(); NBUF],
    head: Buf::empty(),
};

/// bio.c binit(): create the linked list of buffers (initlock is const).
pub fn binit() {
    // SAFETY: boot-time init on the boot CPU before any bread(); mirrors
    // binit()'s list construction pointer for pointer.
    unsafe {
        acquire(&BCACHE_LOCK);
        BCACHE.head.prev = &raw mut BCACHE.head;
        BCACHE.head.next = &raw mut BCACHE.head;
        for i in 0..NBUF {
            let b: *mut Buf = &raw mut BCACHE.buf[i];
            (*b).next = BCACHE.head.next;
            (*b).prev = &raw mut BCACHE.head;
            (*BCACHE.head.next).prev = b;
            BCACHE.head.next = b;
        }
        release(&BCACHE_LOCK);
    }
}

/// Look through buffer cache for sector on device dev.
/// If not found, allocate fresh block.
/// In either case, return B_BUSY buffer. (bio.c bget)
fn bget(dev: u32, sector: u32) -> *mut Buf {
    // SAFETY: bcache.lock discipline exactly as in bio.c bget().
    unsafe {
        acquire(&BCACHE_LOCK);

        let head: *mut Buf = &raw mut BCACHE.head;
        'outer: loop {
            // Is the sector already cached?
            let mut b = BCACHE.head.next;
            while b != head {
                if (*b).dev == dev && (*b).sector == sector {
                    if !has_flag((*b).flags, B_BUSY) {
                        (*b).flags |= B_BUSY;
                        release(&BCACHE_LOCK);
                        return b;
                    }
                    // xv6: sleep(b, &bcache.lock);
                    sleep(b as usize, &BCACHE_LOCK);
                    continue 'outer; // bio.c: goto loop
                }
                b = (*b).next;
            }

            // Not cached; recycle some non-busy and clean buffer.
            let mut b = BCACHE.head.prev;
            while b != head {
                if !has_flag((*b).flags, B_BUSY) && !has_flag((*b).flags, B_DIRTY) {
                    (*b).dev = dev;
                    (*b).sector = sector;
                    (*b).flags = B_BUSY;
                    release(&BCACHE_LOCK);
                    return b;
                }
                b = (*b).prev;
            }
            panic!("bget: no buffers");
        }
    }
}

/// Return a B_BUSY buf with the contents of the indicated disk sector.
/// (bio.c bread)
pub fn bread(dev: u32, sector: u32) -> *mut Buf {
    let b = bget(dev, sector);
    // SAFETY: b is B_BUSY (from bget) and lacks B_VALID, so iderw reads it.
    unsafe {
        if !has_flag((*b).flags, B_VALID) {
            iderw(b);
        }
    }
    b
}

/// Write b's contents to disk. Must be B_BUSY. (bio.c bwrite)
///
/// # Safety
/// `b` must come from [`bread`]/bget and must be B_BUSY and not released.
pub unsafe fn bwrite(b: *mut Buf) {
    if !has_flag((*b).flags, B_BUSY) {
        panic!("bwrite");
    }
    (*b).flags |= B_DIRTY;
    iderw(b);
}

/// Release a B_BUSY buffer. Move to the head of the MRU list. (bio.c brelse)
///
/// # Safety
/// `b` must be B_BUSY and must not be used after this call.
pub unsafe fn brelse(b: *mut Buf) {
    if !has_flag((*b).flags, B_BUSY) {
        panic!("brelse");
    }

    // SAFETY: brelse holds bcache.lock while relinking, like bio.c.
    unsafe {
        acquire(&BCACHE_LOCK);

        (*(*b).next).prev = (*b).prev;
        (*(*b).prev).next = (*b).next;
        (*b).next = BCACHE.head.next;
        (*b).prev = &raw mut BCACHE.head;
        (*BCACHE.head.next).prev = b;
        BCACHE.head.next = b;

        // Wake anyone waiting for this buffer (xv6: wakeup(b)).
        wakeup(b as usize);

        (*b).flags &= !B_BUSY; // release(&bcache.lock)
        release(&BCACHE_LOCK);
    }
}
