//! xv6 file-system core: port of `fs.c` (xv6-public), low-level block /
//! inode-content layers. Five layers (fs.c header comment):
//!   + Blocks: allocator for raw disk blocks.      (this file)
//!   + Log: crash recovery for multi-step updates. (log.rs)
//!   + Files: inode allocator, reading, writing, metadata. (inode.rs)
//!   + Directories: inode with special contents (list of other inodes!) (namei.rs)
//!   + Names: paths like /usr/rtm/xv6/fs.c for convenient naming. (namei.rs)
//!
//! The higher-level system call implementations live elsewhere (sysfile
//! port, Agent B).

pub mod devsw;
pub mod inode;
pub mod layout;
pub mod log;
pub mod namei;

pub use layout::*;

use crate::fs::buffer::{bread, brelse};
use crate::fs::xv6fs::inode::{iupdate, Inode};
use crate::fs::xv6fs::log::log_write;
use core::mem::size_of;

/// Read the super block. (fs.c readsb)
pub fn readsb(dev: u32) -> Superblock {
    let bp = bread(dev, 1);
    let mut sb = Superblock::default();
    // SAFETY: bp->data is a full BSIZE block; we copy sizeof(Superblock)
    // bytes out of it like fs.c's memmove(sb, bp->data, sizeof(*sb)).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (*bp).data.as_ptr(),
            &mut sb as *mut Superblock as *mut u8,
            size_of::<Superblock>(),
        );
        brelse(bp);
    }
    sb
}

/// Zero a block. (fs.c bzero)
pub fn bzero(dev: u32, bno: u32) {
    let bp = bread(dev, bno);
    // SAFETY: bp is B_BUSY from bread; data is BSIZE bytes.
    unsafe {
        (*bp).data.fill(0);
        log_write(bp);
        brelse(bp);
    }
}

// Blocks.

/// Allocate a zeroed disk block. (fs.c balloc)
pub fn balloc(dev: u32) -> u32 {
    let sb = readsb(dev);

    let mut b: u32 = 0;
    while b < sb.size {
        let bp = bread(dev, bblock(b, sb.ninodes));
        let mut bi: u32 = 0;
        while bi < bpb() && b + bi < sb.size {
            // SAFETY: bp is B_BUSY; index bi/8 is inside bp->data.
            unsafe {
                let m: u8 = 1 << (bi % 8);
                if (*bp).data[(bi / 8) as usize] & m == 0 {
                    // Is block free?
                    (*bp).data[(bi / 8) as usize] |= m; // Mark block in use.
                    log_write(bp);
                    brelse(bp);
                    bzero(dev, b + bi);
                    return b + bi;
                }
            }
            bi += 1;
        }
        // SAFETY: bp is B_BUSY.
        unsafe { brelse(bp) };
        b += bpb();
    }
    panic!("balloc: out of blocks");
}

/// Free a disk block. (fs.c bfree)
pub fn bfree(dev: u32, b: u32) {
    let sb = readsb(dev);
    let bp = bread(dev, bblock(b, sb.ninodes));
    let bi = b % bpb();
    let m: u8 = 1 << (bi % 8);
    // SAFETY: bp is B_BUSY; bi/8 indexes inside bp->data.
    unsafe {
        if (*bp).data[(bi / 8) as usize] & m == 0 {
            panic!("freeing free block");
        }
        (*bp).data[(bi / 8) as usize] &= !m;
        log_write(bp);
        brelse(bp);
    }
}

/// fs.c iinit(): xv6 only does `initlock(&icache.lock, "icache")`; the minios
/// inode-table spinlock is const-initialized, so this is a parity no-op that
/// integrators should still call from the boot sequence after `binit()`/
/// `ideinit()`, before the first `iget()`. Use `log::loginit(dev)` for the
/// log/super-block side of fs initialization.
pub fn iinit() {}

// Inode content (fs.c).
//
// The content (data) associated with each inode is stored in blocks on the
// disk. The first NDIRECT block numbers are listed in ip->addrs[]. The next
// NINDIRECT blocks are listed in block ip->addrs[NDIRECT].

/// Return the disk block address of the nth block in inode ip.
/// If there is no such block, bmap allocates one. (fs.c bmap)
///
/// # Safety
/// `ip` must be a locked inode (I_BUSY held by caller).
pub unsafe fn bmap(ip: *mut Inode, bn: u32) -> u32 {
    unsafe {
        if bn < NDIRECT as u32 {
            if (*ip).addrs[bn as usize] == 0 {
                (*ip).addrs[bn as usize] = balloc((*ip).dev);
            }
            return (*ip).addrs[bn as usize];
        }
        let bn = bn - NDIRECT as u32;

        if bn < NINDIRECT as u32 {
            // Load indirect block, allocating if necessary.
            if (*ip).addrs[NDIRECT] == 0 {
                (*ip).addrs[NDIRECT] = balloc((*ip).dev);
            }
            let bp = bread((*ip).dev, (*ip).addrs[NDIRECT]);
            let a = (*bp).data.as_mut_ptr() as *mut u32;
            if *a.add(bn as usize) == 0 {
                *a.add(bn as usize) = balloc((*ip).dev);
                log_write(bp);
            }
            let addr = *a.add(bn as usize);
            brelse(bp);
            return addr;
        }

        panic!("bmap: out of range");
    }
}

/// Truncate inode (discard contents). (fs.c itrunc)
/// Only called when the inode has no links to it (no directory entries
/// referring to it) and has no in-memory reference to it (is not an open
/// file or current directory).
///
/// # Safety
/// `ip` must be a locked inode (I_BUSY held by caller, as in iput()).
pub unsafe fn itrunc(ip: *mut Inode) {
    unsafe {
        for i in 0..NDIRECT {
            if (*ip).addrs[i] != 0 {
                bfree((*ip).dev, (*ip).addrs[i]);
                (*ip).addrs[i] = 0;
            }
        }

        if (*ip).addrs[NDIRECT] != 0 {
            let bp = bread((*ip).dev, (*ip).addrs[NDIRECT]);
            let a = (*bp).data.as_ptr() as *const u32;
            for j in 0..NINDIRECT {
                if *a.add(j) != 0 {
                    bfree((*ip).dev, *a.add(j));
                }
            }
            brelse(bp);
            bfree((*ip).dev, (*ip).addrs[NDIRECT]);
            (*ip).addrs[NDIRECT] = 0;
        }

        (*ip).size = 0;
        iupdate(ip);
    }
}
