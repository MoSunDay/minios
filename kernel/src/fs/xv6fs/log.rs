//! Simple logging: port of `log.c` (xv6-public), using the classic
//! begin_trans/commit_trans single-transaction discipline.
//!
//! Each system call that might write the file system should be surrounded
//! with `begin_op()` and `end_op()` calls (classic xv6: begin_trans /
//! commit_trans).
//!
//! The log holds at most one transaction at a time. Commit forces the log
//! (with commit record) to disk, then installs the affected blocks to disk,
//! then erases the log. begin_op() lets several sys calls share one
//! transaction but ensures only one commit is in flight; others wait.
//!
//! Allowing only one commit at a time means that the file system code
//! doesn't have to worry about the possibility of one transaction reading a
//! block that another one has modified, for example an i-node block.
//!
//! Read-only system calls don't need to use transactions, though this means
//! that they may observe uncommitted data. I-node and buffer locks prevent
//! read-only calls from seeing inconsistent data.
//!
//! The log is a physical re-do log containing disk blocks. The on-disk log
//! format:
//!   header block, containing block #s for block A, B, C, ...
//!   block A
//!   block B
//!   block C
//!   ...
//! Log appends are synchronous.
//!
//! Locking: log.c's `struct log { struct spinlock lock; ... }` is ported as
//! a `static mut LOG` with an embedded RawLock; waiters sleep on the log's
//! address through crate::kernel::sched::sleep/wakeup.

use crate::fs::buffer::{bread, brelse, bwrite, Buf, B_DIRTY};
use crate::fs::xv6fs::layout::{BSIZE, LOGSIZE};
use crate::fs::xv6fs::readsb;
use crate::kernel::sched::{acquire, release, sleep, wakeup, RawLock};
use core::mem::size_of;

/// Contents of the header block, used for both the on-disk header block and
/// to keep track in memory of logged block #s before commit.
/// (log.c `struct logheader`)
#[repr(C)]
struct LogHeader {
    n: i32,
    block: [u32; LOGSIZE],
}

impl LogHeader {
    const fn empty() -> LogHeader {
        LogHeader {
            n: 0,
            block: [0; LOGSIZE],
        }
    }
}

/// log.c `struct log`; the lock is a sibling static LOG_LOCK (RawLock is
/// Sync on its own; Log contains plain data, so it stays `static mut`),
/// like sched's TICKS/TICKS_LOCK split of xv6's embedded-spinlock structs.
struct Log {
    start: u32,
    size: u32,
    /// Nonzero while a transaction is active. (log.c `log.busy`)
    busy: bool,
    dev: u32,
    lh: LogHeader,
}

// SAFETY: all fields are guarded by LOG_LOCK; the statics are unique.
static LOG_LOCK: RawLock = RawLock::new("log");
static mut LOG: Log = Log {
    start: 0,
    size: 0,
    busy: false,
    dev: 0,
    lh: LogHeader::empty(),
};

/// Sleeping channel for the log (xv6: `sleep(&log, ...)` on the struct).
#[allow(static_mut_refs)]
fn log_chan() -> usize {
    core::ptr::addr_of!(LOG) as usize
}

/// log.c initlog() (minios calls it loginit(dev) in the boot sequence).
///
/// Reads the super block from `dev` and locates the log. The classic
/// on-disk format keeps the log in the LAST `sb.nlog` blocks of the disk
/// (xv6 log.c: `log.start = sb.size - sb.nlog`); the super block carries no
/// logstart field. Then recovers any committed-but-not-installed
/// transaction left by a crash.
pub fn loginit(dev: u32) {
    if size_of::<LogHeader>() >= BSIZE {
        panic!("loginit: too big logheader");
    }

    let sb = readsb(dev);
    // SAFETY: boot-time init; xv6 initlog holds log.lock across the field
    // writes and releases before recover_from_log().
    unsafe {
        acquire(&LOG_LOCK);
        LOG.dev = dev;
        LOG.start = sb.size - sb.nlog;
        LOG.size = sb.nlog;
        release(&LOG_LOCK);
    }

    recover_from_log();
}

/// xv6-faithful spelling of [`loginit`].
pub fn initlog(dev: u32) {
    loginit(dev);
}

/// Copy committed blocks from log to their home location. (log.c
/// install_trans)
fn install_trans() {
    // Snapshot the transaction under the log lock, then run the (buffered)
    // block copies; xv6 reads log.lh.n / log.lh.block[tail] directly, which
    // is safe for the same reason: only the committing sys call touches lh.
    let (dev, start, n, blocks) = {
        // SAFETY: brief critical section copying the header out.
        unsafe {
            acquire(&LOG_LOCK);
            let dev = LOG.dev;
            let start = LOG.start;
            let n = LOG.lh.n;
            let mut blocks = [0u32; LOGSIZE];
            blocks[..n as usize].copy_from_slice(&LOG.lh.block[..n as usize]);
            release(&LOG_LOCK);
            (dev, start, n, blocks)
        }
    };
    for tail in 0..n as usize {
        // SAFETY: bread/brelse/bwrite per the buffer-cache contract.
        unsafe {
            let lbuf = bread(dev, start + tail as u32 + 1); // read log block
            let dbuf = bread(dev, blocks[tail]); // read dst
                                                 // copy block to dst
            (*dbuf).data.copy_from_slice(&(*lbuf).data);
            bwrite(dbuf); // write dst to disk
            brelse(lbuf);
            brelse(dbuf);
        }
    }
}

/// Read the log header from disk into the in-memory log header.
/// (log.c read_head)
fn read_head() {
    // SAFETY: brief critical sections bracketing the buffered read.
    unsafe {
        acquire(&LOG_LOCK);
        let dev = LOG.dev;
        let start = LOG.start;
        release(&LOG_LOCK);

        // bread returns a B_BUSY buffer; the header is read out of the
        // block data exactly like log.c's cast of buf->data.
        let buf = bread(dev, start);
        let lh = (*buf).data.as_ptr() as *const LogHeader;
        let n = (*lh).n.clamp(0, LOGSIZE as i32);

        acquire(&LOG_LOCK);
        LOG.lh.n = n;
        for i in 0..n as usize {
            LOG.lh.block[i] = (*lh).block[i];
        }
        release(&LOG_LOCK);

        brelse(buf);
    }
}

/// Write in-memory log header to disk. This is the true point at which the
/// current transaction commits. (log.c write_head)
fn write_head() {
    // SAFETY: buf is B_BUSY; hb aliases buf->data while the block is held.
    unsafe {
        acquire(&LOG_LOCK);
        let dev = LOG.dev;
        let start = LOG.start;
        release(&LOG_LOCK);

        let buf = bread(dev, start);
        let hb = (*buf).data.as_mut_ptr() as *mut LogHeader;

        acquire(&LOG_LOCK);
        let n = LOG.lh.n;
        for i in 0..n as usize {
            (*hb).block[i] = LOG.lh.block[i];
        }
        release(&LOG_LOCK);

        (*hb).n = n;
        bwrite(buf);
        brelse(buf);
    }
}

/// Commit a transaction: write the header to disk (the real commit point),
/// install the logged blocks, then erase the log. (log.c commit)
fn commit() {
    // SAFETY: log.lock only guards the lh.n checks/updates here.
    unsafe {
        acquire(&LOG_LOCK);
        let has_work = LOG.lh.n > 0;
        release(&LOG_LOCK);
        if has_work {
            write_head(); // Write header to disk -- the real commit
            install_trans(); // Now install writes to home locations
            acquire(&LOG_LOCK);
            LOG.lh.n = 0;
            release(&LOG_LOCK);
            write_head(); // Erase the transaction from the log
        }
    }
}

fn recover_from_log() {
    read_head();
    install_trans(); // if committed, copy from log to disk
                     // SAFETY: erase the recovered header like log.c recover_from_log().
    unsafe {
        acquire(&LOG_LOCK);
        LOG.lh.n = 0;
        release(&LOG_LOCK);
    }
    write_head(); // clear the log
}

/// Called at the start of each FS system call that writes the file system.
/// (log.c begin_op, later xv6 revision)
pub fn begin_op() {
    // SAFETY: sleep() releases log.lock while waiting and re-acquires it
    // before returning; loop mirrors log.c's begin_trans().
    unsafe {
        acquire(&LOG_LOCK);
        while LOG.busy {
            sleep(log_chan(), &LOG_LOCK);
        }
        LOG.busy = true;
        release(&LOG_LOCK);
    }
}

/// Commit the active transaction and release the log to other writers.
/// (log.c end_trans/commit_trans)
pub fn end_op() {
    commit();
    // SAFETY: wake writers waiting in begin_op().
    unsafe {
        acquire(&LOG_LOCK);
        LOG.busy = false;
        wakeup(log_chan());
        release(&LOG_LOCK);
    }
}

/// Caller has modified b->data and is done with the buffer.
/// Append the block to the log and record the block number, but don't write
/// the log header (which would commit the write). log_write() replaces
/// bwrite(); a typical use is:
/// ```text
/// bp = bread(...)
/// modify bp->data[]
/// log_write(bp)
/// brelse(bp)
/// ```
/// (log.c log_write)
///
/// # Safety
/// `b` must be B_BUSY, from [`bread`], and begin_op() must be active.
pub unsafe fn log_write(b: *mut Buf) {
    let sector = (*b).sector;
    let dev = (*b).dev;

    // log absorbtion: unconditional block match, exactly like xv6
    // (`if (log.lh.block[i] == b->blockno) break;`). Only the sys calls of
    // the single open transaction touch lh, so xv6 does this without the
    // log lock; we take it briefly to read/write the shared state.
    let (i, start, n) = {
        // SAFETY: brief critical section for the absorption scan.
        unsafe {
            acquire(&LOG_LOCK);
            if !LOG.busy {
                panic!("log_write outside of op");
            }
            if LOG.lh.n >= LOGSIZE as i32 || LOG.lh.n >= LOG.size as i32 - 1 {
                panic!("too big a transaction");
            }
            let mut i: usize = 0;
            while i < LOG.lh.n as usize {
                if LOG.lh.block[i] == sector {
                    break;
                }
                i += 1;
            }
            LOG.lh.block[i] = sector;
            let r = (i, LOG.start, LOG.lh.n);
            release(&LOG_LOCK);
            r
        }
    };

    // SAFETY: bread/bwrite/brelse per the buffer-cache contract.
    unsafe {
        let lbuf = bread(dev, start + i as u32 + 1);
        (*lbuf).data.copy_from_slice(&(*b).data);
        bwrite(lbuf);
        brelse(lbuf);
    }

    if i == n as usize {
        // SAFETY: brief critical section bumping the header count.
        unsafe {
            acquire(&LOG_LOCK);
            LOG.lh.n += 1;
            release(&LOG_LOCK);
        }
    }
    (*b).flags |= B_DIRTY; // XXX prevent eviction
}
