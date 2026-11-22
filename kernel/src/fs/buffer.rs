//! Buffer cache: port of bio.c (xv6-public). One lock for the whole
//! cache; per-buffer B_BUSY/B_VALID/B_DIRTY flags; LRU via the
//! head-anchored list. Fixed-size static state, raw-pointer handles —
//! the ptable precedent: lifetime is the kernel's.

use super::BSIZE;
use crate::drivers::block::ide::bdev_strategy;
use crate::klib::sync::Spinlock;
use crate::proc::sleep::{sleep, wakeup};
use core::sync::atomic::{AtomicI32, Ordering};

pub const B_BUSY: i32 = 0x1; // locked by some process
pub const B_VALID: i32 = 0x2; // holds data read from disk
pub const B_DIRTY: i32 = 0x4; // must be written to disk

/// Relaxed loads/stores: the flags are guarded by the bcache/IDE locks on
/// the mutating side; the completion wait in `bdev_strategy` only needs
/// the read to actually re-read memory each iteration (x86 TSO provides
/// the ordering once the compiler is stopped from hoisting the load).

/// Cached blocks (xv6 NBUF).
const NBUF: usize = 30;

/// A block buffer (xv6 struct buf). `prev`/`next` form the LRU list;
/// `qnext` chains the IDE request queue.
#[repr(C)]
pub struct Buf {
    pub flags: AtomicI32,
    pub dev: u32,
    pub sector: u32,
    prev: *mut Buf,
    next: *mut Buf,
    pub(crate) qnext: *mut Buf,
    pub data: [u8; BSIZE],
}

struct BCache {
    head: Buf, // list sentry; head.next is most recently used
    bufs: [Buf; NBUF],
}

static BCACHE_LOCK: Spinlock<()> = Spinlock::new("bcache", ());
// SAFETY: `BCACHE` is only touched while holding BCACHE_LOCK.
static mut BCACHE: BCache = BCache {
    head: Buf {
        flags: AtomicI32::new(0),
        dev: 0,
        sector: 0,
        prev: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
        qnext: core::ptr::null_mut(),
        data: [0; BSIZE],
    },
    bufs: [NEW_BUF; NBUF],
};

const NEW_BUF: Buf = Buf {
    flags: AtomicI32::new(0),
    dev: 0,
    sector: 0,
    prev: core::ptr::null_mut(),
    next: core::ptr::null_mut(),
    qnext: core::ptr::null_mut(),
    data: [0; BSIZE],
};

/// Link the whole cache into the most-recent/head.next list (binit).
pub fn binit() {
    // SAFETY: boot-time one-shot before any buffer use.
    unsafe {
        let b = core::ptr::addr_of_mut!(BCACHE);
        (*b).head.prev = core::ptr::addr_of_mut!((*b).head);
        (*b).head.next = core::ptr::addr_of_mut!((*b).head);
        for i in 0..NBUF {
            let bp = core::ptr::addr_of_mut!((*b).bufs[i]);
            (*bp).next = (*b).head.next;
            (*bp).prev = core::ptr::addr_of_mut!((*b).head);
            (*(*b).head.next).prev = bp;
            (*b).head.next = bp;
        }
    }
}

/// Look for `sector` on `dev` in the cache; recycle an unused clean
/// buffer otherwise. Busy hits sleep and rescan (bget).
///
/// # Safety
/// Returns a B_BUSY buffer; the caller must `brelse` it exactly once.
unsafe fn bget(dev: u32, sector: u32) -> *mut Buf {
    // SAFETY: every path below holds BCACHE_LOCK exactly once (sleep
    // re-acquires it before returning), matching xv6 bget.
    unsafe {
        BCACHE_LOCK.raw_lock();
        'outer: loop {
            let b = core::ptr::addr_of_mut!(BCACHE);
            let mut bp = (*b).head.next;
            while bp != core::ptr::addr_of_mut!((*b).head) {
                if (*bp).dev == dev && (*bp).sector == sector {
                    if (*bp).flags.load(Ordering::Relaxed) & B_BUSY == 0 {
                        (*bp).flags.fetch_or(B_BUSY, Ordering::Relaxed);
                        BCACHE_LOCK.raw_release();
                        return bp;
                    }
                    sleep(bp as usize, &BCACHE_LOCK);
                    // rescan from the head (xv6 `goto loop`)
                    continue 'outer;
                }
                bp = (*bp).next;
            }
            break;
        }
        // Not cached: recycle the least-recently-used clean buffer.
        let b = core::ptr::addr_of_mut!(BCACHE);
        let mut bp = (*b).head.prev;
        while bp != core::ptr::addr_of_mut!((*b).head) {
            if (*bp).flags.load(Ordering::Relaxed) & (B_BUSY | B_DIRTY) == 0 {
                (*bp).dev = dev;
                (*bp).sector = sector;
                (*bp).flags.store(B_BUSY, Ordering::Relaxed);
                BCACHE_LOCK.raw_release();
                return bp;
            }
            bp = (*bp).prev;
        }
        panic!("bget: no buffers");
    }
}

/// Total bread dispatches (m4r-s3): the page-cache hit gate reconciles
/// this against pc misses/readpages — a cached range re-read must move
/// it by zero. Metadata reads (ilock/dir walks) count too; suite cases
/// snapshot around cache-only activity.
pub fn bread_count() -> u64 {
    BREAD_COUNT.load(Ordering::Relaxed)
}

static BREAD_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read a sector through the cache (bread).
///
/// # Safety
/// The returned buffer is B_BUSY|B_VALID; release with `brelse`.
pub unsafe fn bread(dev: u32, sector: u32) -> *mut Buf {
    BREAD_COUNT.fetch_add(1, Ordering::Relaxed);
    // SAFETY: bget discipline upheld; iderw waits for completion.
    unsafe {
        let bp = bget(dev, sector);
        if (*bp).flags.load(Ordering::Relaxed) & B_VALID == 0 {
            bdev_strategy(bp);
        }
        bp
    }
}

/// Write a busy buffer to disk (bwrite). Synchronous: returns once the
/// block is clean and valid again.
///
/// # Safety
/// `bp` must be B_BUSY from bread/bget.
pub unsafe fn bwrite(bp: *mut Buf) {
    // SAFETY: B_BUSY ownership upheld.
    unsafe {
        if (*bp).flags.load(Ordering::Relaxed) & B_BUSY == 0 {
            panic!("bwrite: buffer not busy");
        }
        (*bp).flags.fetch_or(B_DIRTY, Ordering::Relaxed);
        bdev_strategy(bp);
    }
}

/// Release a busy buffer; relink it as most recently used and wake
/// waiters (brelse).
///
/// # Safety
/// `bp` must be B_BUSY; the caller must not touch it afterwards.
pub unsafe fn brelse(bp: *mut Buf) {
    // SAFETY: bcache lock guards the list and flags.
    unsafe {
        BCACHE_LOCK.raw_lock();
        let b = core::ptr::addr_of_mut!(BCACHE);
        (*(*bp).next).prev = (*bp).prev;
        (*(*bp).prev).next = (*bp).next;
        (*bp).next = (*b).head.next;
        (*bp).prev = core::ptr::addr_of_mut!((*b).head);
        (*(*b).head.next).prev = bp;
        (*b).head.next = bp;

        (*bp).flags.fetch_and(!B_BUSY, Ordering::Relaxed);
        wakeup(bp as usize);

        BCACHE_LOCK.raw_release();
    }
}
