//! Inodes: port of the inode half of `fs.c` (xv6-public) + `struct inode`
//! from `file.h`.
//!
//! Inode cache discipline (fs.c comment):
//! * An inode and its copy in memory are protected by a per-fs lock; the
//!   cache entry is only correct when the I_VALID bit is set in ip->flags.
//!   ilock() reads the inode from the disk and sets I_VALID, while iput()
//!   clears I_VALID if ip->ref has fallen to zero.
//! * File system code may only examine and modify the information in an
//!   inode and its content if it has first locked the inode. The I_BUSY flag
//!   indicates that the inode is locked. ilock() sets I_BUSY, while iunlock
//!   clears it.
//!
//! Typical sequence: `iget` -> `ilock` -> ... -> `iunlock` -> `iput`.

use crate::fs::buffer::{bread, brelse};
use crate::fs::xv6fs::devsw::{devsw_read, devsw_write};
use crate::fs::xv6fs::layout::{
    has_flag, iblock, ipb, DInode, Stat, Superblock, BSIZE, MAXFILE, NDEV, NDIRECT, NINODE, T_DEV,
};
use crate::fs::xv6fs::log::log_write;
use crate::fs::xv6fs::{bmap, itrunc, readsb};
use crate::klib::sync::Spinlock;
use crate::proc::sleep::{sleep, wakeup};
use core::mem::size_of;

/// file.h: in-memory inode flags.
pub const I_BUSY: i32 = 0x1;
pub const I_VALID: i32 = 0x2;

/// file.h: in-memory copy of an inode (`struct inode`), embedded disk-inode
/// fields included. Managed through raw pointers like xv6.
#[derive(Clone, Copy)]
pub struct Inode {
    /// Device number.
    pub dev: u32,
    /// Inode number.
    pub inum: u32,
    /// Reference count.
    pub ref_: i32,
    /// I_BUSY, I_VALID.
    pub flags: i32,

    // copy of disk inode (struct dinode) from here down
    pub type_: i16,
    pub major: i16,
    pub minor: i16,
    pub nlink: i16,
    pub size: u32,
    pub addrs: [u32; NDIRECT + 1],
}

impl Inode {
    const fn empty() -> Inode {
        Inode {
            dev: 0,
            inum: 0,
            ref_: 0,
            flags: 0,
            type_: 0,
            major: 0,
            minor: 0,
            nlink: 0,
            size: 0,
            addrs: [0; NDIRECT + 1],
        }
    }
}

/// The inode table: fs.c's `struct { spinlock lock; inode[NINODE]; } icache;`
/// — the lock is a sibling static (RawLock is Sync on its own; the Inode
/// array is not, so the data stays `static mut`), like sched's
/// TICKS/TICKS_LOCK split of xv6's embedded-spinlock structs.
pub struct ICache {
    pub inode: [Inode; NINODE],
}

// SAFETY: entries are guarded by ICACHE_LOCK (plus the I_BUSY protocol for
// the embedded inode fields); the statics are unique.
static ICACHE_LOCK: Spinlock<()> = Spinlock::new("icache", ());
static mut ICACHE: ICache = ICache {
    inode: [Inode::empty(); NINODE],
};

/// xv6 acquire()/release() spelling over the spinlock primitive (the raw
/// pair is required: sleep() drops the lock mid-critical-section).
/// SAFETY: caller must balance each acquire with a release of the same lock.
#[inline]
unsafe fn acquire(l: &Spinlock<()>) {
    l.raw_lock();
}

/// SAFETY: the lock must be held by this context.
#[inline]
unsafe fn release(l: &Spinlock<()>) {
    l.raw_release();
}

/// Find the inode with number inum on device dev and return the in-memory
/// copy. Does not lock the inode and does not read it from disk. (fs.c iget)
///
/// # Safety
/// callers must eventually [`iput`] the returned inode.
pub unsafe fn iget(dev: u32, inum: u32) -> *mut Inode {
    // SAFETY: icache.lock guards ref counts and slot assignment (fs.c iget).
    unsafe {
        acquire(&ICACHE_LOCK);

        // Is the inode already cached?
        let mut empty: *mut Inode = core::ptr::null_mut();
        for i in 0..NINODE {
            let ip: *mut Inode = &raw mut ICACHE.inode[i];
            if (*ip).ref_ > 0 && (*ip).dev == dev && (*ip).inum == inum {
                (*ip).ref_ += 1;
                release(&ICACHE_LOCK);
                return ip;
            }
            if empty.is_null() && (*ip).ref_ == 0 {
                empty = ip; // Remember empty slot.
            }
        }

        // Recycle an inode cache entry.
        let ip = empty;
        if ip.is_null() {
            panic!("iget: no inodes");
        }
        (*ip).dev = dev;
        (*ip).inum = inum;
        (*ip).ref_ = 1;
        (*ip).flags = 0;
        release(&ICACHE_LOCK);
        ip
    }
}

/// Increment reference count for ip. (fs.c idup); returns ip for the
/// `ip = idup(ip1)` idiom.
///
/// # Safety
/// `ip` must have come from [`iget`] and still be referenced.
pub unsafe fn idup(ip: *mut Inode) -> *mut Inode {
    // SAFETY: icache.lock guards the refcount bump (fs.c idup).
    unsafe {
        acquire(&ICACHE_LOCK);
        (*ip).ref_ += 1;
        release(&ICACHE_LOCK);
    }
    ip
}

/// Allocate a new inode with the given type on device dev.
/// A free inode has a type of zero. (fs.c ialloc)
///
/// # Safety
/// caller must be inside a begin_op/end_op transaction (log_write is used).
#[allow(dead_code)] // S4d: sys_open/create/mkdir allocate inodes
pub unsafe fn ialloc(dev: u32, type_: i16) -> *mut Inode {
    let sb: Superblock = readsb(dev);

    for inum in 1..sb.ninodes {
        let bp = bread(dev, iblock(inum));
        // SAFETY: bp is B_BUSY; dip points at the inum%IPB'th dinode in it.
        unsafe {
            let dip = ((*bp).data.as_mut_ptr() as *mut DInode).add((inum % ipb() as u32) as usize);
            if (*dip).type_ == 0 {
                // a free inode
                core::ptr::write_bytes(dip as *mut u8, 0, size_of::<DInode>());
                (*dip).type_ = type_;
                log_write(bp); // mark it allocated on the disk
                brelse(bp);
                return iget(dev, inum);
            }
            brelse(bp);
        }
    }
    panic!("ialloc: no inodes");
}

/// Copy a modified in-memory inode to disk. (fs.c iupdate)
///
/// # Safety
/// `ip` must be a locked inode.
pub unsafe fn iupdate(ip: *mut Inode) {
    let bp = bread((*ip).dev, iblock((*ip).inum));
    // SAFETY: bp is B_BUSY; dip addresses the ip->inum%IPB'th dinode.
    unsafe {
        let dip =
            ((*bp).data.as_mut_ptr() as *mut DInode).add(((*ip).inum % ipb() as u32) as usize);
        (*dip).type_ = (*ip).type_;
        (*dip).major = (*ip).major;
        (*dip).minor = (*ip).minor;
        (*dip).nlink = (*ip).nlink;
        (*dip).size = (*ip).size;
        (*dip).addrs.copy_from_slice(&(*ip).addrs);
        log_write(bp);
        brelse(bp);
    }
}

/// Lock the given inode. Reads the inode from disk if necessary. (fs.c ilock)
///
/// # Safety
/// `ip` must have ref > 0 (from iget/ialloc) and not be locked by caller.
pub unsafe fn ilock(ip: *mut Inode) {
    if ip.is_null() || (*ip).ref_ < 1 {
        panic!("ilock");
    }

    // SAFETY: sleep() releases icache.lock while waiting and re-acquires it
    // before returning; the flags update is under the lock (fs.c ilock).
    unsafe {
        acquire(&ICACHE_LOCK);
        while has_flag((*ip).flags, I_BUSY) {
            sleep(ip as usize, &ICACHE_LOCK);
        }
        (*ip).flags |= I_BUSY;
        release(&ICACHE_LOCK);
    }

    if !has_flag((*ip).flags, I_VALID) {
        let bp = bread((*ip).dev, iblock((*ip).inum));
        // SAFETY: bp is B_BUSY; dip addresses our dinode in the block.
        unsafe {
            let dip =
                ((*bp).data.as_ptr() as *const DInode).add(((*ip).inum % ipb() as u32) as usize);
            (*ip).type_ = (*dip).type_;
            (*ip).major = (*dip).major;
            (*ip).minor = (*dip).minor;
            (*ip).nlink = (*dip).nlink;
            (*ip).size = (*dip).size;
            (*ip).addrs.copy_from_slice(&(*dip).addrs);
            brelse(bp);
        }
        (*ip).flags |= I_VALID;
        if (*ip).type_ == 0 {
            panic!("ilock: no type");
        }
    }
}

/// Unlock the given inode. (fs.c iunlock)
///
/// # Safety
/// `ip` must be locked (I_BUSY) with ref > 0.
pub unsafe fn iunlock(ip: *mut Inode) {
    if ip.is_null() || !has_flag((*ip).flags, I_BUSY) || (*ip).ref_ < 1 {
        panic!("iunlock");
    }

    // SAFETY: icache.lock guards the flags update (fs.c iunlock).
    unsafe {
        acquire(&ICACHE_LOCK);
        (*ip).flags &= !I_BUSY;
        wakeup(ip as usize);
        release(&ICACHE_LOCK);
    }
}

/// Drop a reference to an in-memory inode. (fs.c iput)
/// If that was the last reference, the inode cache entry can be recycled.
/// If that was the last reference and the inode has no links to it, free the
/// inode (and its content) on disk.
///
/// # Safety
/// `ip` must have ref > 0 and must not be locked (I_BUSY) by the caller.
pub unsafe fn iput(ip: *mut Inode) {
    // SAFETY: icache.lock guards the ref/flags transitions (fs.c iput).
    unsafe {
        acquire(&ICACHE_LOCK);
        if (*ip).ref_ == 1 && has_flag((*ip).flags, I_VALID) && (*ip).nlink == 0 {
            // inode has no links: truncate and free inode.
            if has_flag((*ip).flags, I_BUSY) {
                panic!("iput busy");
            }
            (*ip).flags |= I_BUSY;
        } else {
            (*ip).ref_ -= 1;
            release(&ICACHE_LOCK);
            return;
        }
        release(&ICACHE_LOCK);
    }

    itrunc(ip);
    (*ip).type_ = 0;
    iupdate(ip);

    // SAFETY: second icache.lock section frees the cache slot (fs.c iput).
    unsafe {
        acquire(&ICACHE_LOCK);
        (*ip).flags = 0;
        wakeup(ip as usize);
        (*ip).ref_ -= 1;
        release(&ICACHE_LOCK);
    }
}

/// Common idiom: unlock, then put. (fs.c iunlockput)
/// # Safety: `ip` must be locked by the caller.
pub unsafe fn iunlockput(ip: *mut Inode) {
    iunlock(ip);
    iput(ip);
}

/// Copy stat information from inode. (fs.c stati)
///
/// # Safety
/// `ip` must be locked; `st` must be valid for writes.
#[allow(dead_code)] // S4d: sys_fstat
pub unsafe fn stati(ip: *const Inode, st: *mut Stat) {
    unsafe {
        (*st).dev = (*ip).dev as i32;
        (*st).ino = (*ip).inum;
        (*st).type_ = (*ip).type_;
        (*st).nlink = (*ip).nlink;
        (*st).size = (*ip).size;
    }
}

/// Read data from inode. (fs.c readi)
///
/// # Safety
/// `ip` must be locked; `dst` must be valid for `n` bytes (may be user
/// memory, as in xv6 — callers validate elsewhere).
pub unsafe fn readi(ip: *mut Inode, dst: *mut u8, mut off: u32, mut n: u32) -> i32 {
    unsafe {
        if (*ip).type_ == T_DEV {
            let major = (*ip).major;
            if major < 0 || major as usize >= NDEV {
                return -1;
            }
            // SAFETY: major bounds-checked above; table boot-initialized.
            match devsw_read(major) {
                None => -1,
                Some(read) => read(ip, dst, n as i32),
            }
        } else {
            if off > (*ip).size || off.checked_add(n).is_none() {
                return -1;
            }
            if off + n > (*ip).size {
                n = (*ip).size - off;
            }

            let mut tot: u32 = 0;
            while tot < n {
                let bp = bread((*ip).dev, bmap(ip, off / BSIZE as u32));
                let m = core::cmp::min(n - tot, BSIZE as u32 - off % BSIZE as u32);
                core::ptr::copy_nonoverlapping(
                    (*bp).data.as_ptr().add((off % BSIZE as u32) as usize),
                    dst.add(tot as usize),
                    m as usize,
                );
                brelse(bp);
                tot += m;
                off += m;
            }
            n as i32
        }
    }
}

/// Write data to inode. (fs.c writei)
///
/// # Safety
/// `ip` must be locked; `src` must be valid for `n` bytes (may be user
/// memory). Caller must have called [`crate::fs::xv6fs::log::begin_op`].
#[allow(dead_code)] // S4d: write path lands with the file table
pub unsafe fn writei(ip: *mut Inode, src: *const u8, mut off: u32, n: u32) -> i32 {
    unsafe {
        if (*ip).type_ == T_DEV {
            let major = (*ip).major;
            if major < 0 || major as usize >= NDEV {
                return -1;
            }
            // SAFETY: major bounds-checked above; table boot-initialized.
            return match devsw_write(major) {
                None => -1,
                Some(write) => write(ip, src, n as i32),
            };
        }

        if off > (*ip).size || off.checked_add(n).is_none() {
            return -1;
        }
        if off + n > MAXFILE as u32 * BSIZE as u32 {
            return -1;
        }

        let mut tot: u32 = 0;
        while tot < n {
            let bp = bread((*ip).dev, bmap(ip, off / BSIZE as u32));
            let m = core::cmp::min(n - tot, BSIZE as u32 - off % BSIZE as u32);
            core::ptr::copy_nonoverlapping(
                src.add(tot as usize),
                (*bp).data.as_mut_ptr().add((off % BSIZE as u32) as usize),
                m as usize,
            );
            log_write(bp);
            brelse(bp);
            tot += m;
            off += m;
        }

        if n > 0 && off > (*ip).size {
            (*ip).size = off;
            iupdate(ip);
        }
        n as i32
    }
}
