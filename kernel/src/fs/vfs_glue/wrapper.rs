//! wrapper.rs (m4r-s2b): the VfsInode wrapper pool — the binding
//! between dcache entries and legacy xv6fs inodes. One wrapper per
//! dentry (not per inode: the legacy cache already owns inode
//! identity, and the s1 Icache stays unconsumed until tmpfs, s6);
//! each wrapper holds exactly one iget'd reference, so a cached name
//! pins its inode against eviction. Pool exhaustion is not fatal —
//! callers fall back to uncached names.

use super::mutate;
use super::DENTRY_LOCK;
use crate::fs::vfs::ops::InodeCommon;
use crate::fs::vfs::{VfsErr, VfsResult, S_IFBLK, S_IFCHR, S_IFDIR, S_IFREG};
use crate::fs::xv6fs::inode::Inode as Xv6Inode;

const NVINODE: usize = 128;

/// VFS inode wrapper: the InodeCommon header plus the legacy inode it
/// shadows. `c` stays first so vtable fns recover the wrapper by
/// pointer cast.
#[repr(C)]
pub struct VfsInode {
    pub c: InodeCommon,
    pub xv6_ip: *mut Xv6Inode,
}

impl VfsInode {
    pub const fn empty() -> VfsInode {
        VfsInode {
            c: InodeCommon::empty(),
            xv6_ip: core::ptr::null_mut(),
        }
    }
}

static mut INODE_POOL: [VfsInode; NVINODE] = [const { VfsInode::empty() }; NVINODE];
/// Free wrapper chain threaded through `c.i_lru_next` (the s1 icache
/// LRU links are unused while a wrapper is free — s6 owns them).
static mut INODE_FREE: *mut VfsInode = core::ptr::null_mut();

/// Build the free chain. Boot context, no concurrency.
pub(super) unsafe fn init() {
    unsafe {
        INODE_FREE = core::ptr::null_mut();
        for i in (0..NVINODE).rev() {
            INODE_POOL[i].c.i_lru_next = INODE_FREE as *mut InodeCommon;
            INODE_FREE = &raw mut INODE_POOL[i];
        }
    }
}

/// Take a wrapper slot and bind it to `ip` (ownership of one iget'd
/// reference transfers to the wrapper). Caller holds DENTRY_LOCK.
pub(crate) unsafe fn wrap_new(ip: *mut Xv6Inode) -> VfsResult<*mut VfsInode> {
    // SAFETY: free chain guarded by the caller's DENTRY_LOCK.
    unsafe {
        if INODE_FREE.is_null() {
            return Err(VfsErr::NoSpc); // caller falls back to uncached
        }
        let w = INODE_FREE;
        INODE_FREE = (*w).c.i_lru_next as *mut VfsInode;
        let e = &mut *w;
        e.c = InodeCommon::empty();
        e.c.i_dev = (*ip).dev;
        e.c.i_ino = (*ip).inum;
        e.c.i_mode = iftype_to_mode((*ip).type_);
        e.c.i_op = &super::xv6_adapt::XV6_INODE_OPS;
        e.c.i_fop = &super::xv6_adapt::XV6_FILE_OPS;
        e.c.i_sb = &raw mut super::ROOT_SB;
        e.xv6_ip = ip;
        Ok(w)
    }
}

/// wrap_new for callers that do NOT hold DENTRY_LOCK — the adapter
/// vtable fns (lookup/create) run with the lock dropped by the glue
/// walk/mutate phases, so they take it here just for the pool.
pub(crate) unsafe fn wrap_new_unlocked(ip: *mut Xv6Inode) -> VfsResult<*mut VfsInode> {
    // SAFETY: balanced raw hand-off; the pool is only touched under
    // DENTRY_LOCK and no holder of it sleeps.
    unsafe {
        DENTRY_LOCK.raw_lock();
        let r = wrap_new(ip);
        DENTRY_LOCK.raw_release();
        r
    }
}

fn iftype_to_mode(t: i16) -> u16 {
    use crate::fs::xv6fs::layout::{T_DEV, T_DIR, T_FILE};
    // xv6fs carries no DAC bits; rwxrwxrwx is the honest image of the
    // legacy no-permission-check semantics (generic DAC always allows).
    match t {
        T_DIR => S_IFDIR | 0o777,
        T_FILE => S_IFREG | 0o777,
        T_DEV => S_IFCHR | 0o777,
        _ => S_IFBLK | 0o777,
    }
}

/// Release a wrapper (its dentry died): the shadowed inode reference
/// goes back. nlink == 0 defers the final iput to process context via
/// the zombie list (it may truncate + sleep); a plain drop happens
/// inline. Caller holds DENTRY_LOCK.
pub(crate) unsafe fn release_wrapper(w: *mut VfsInode) {
    unsafe {
        let ip = (*w).xv6_ip;
        if (*ip).nlink < 1 {
            mutate::zombie_push(w); // owns the ref + slot for now
        } else {
            crate::fs::xv6fs::inode::iput(ip); // ref only; no sleep
            wrapper_free(w);
        }
    }
}

/// Return a wrapper slot to the free chain. Caller holds DENTRY_LOCK.
pub(crate) unsafe fn wrapper_free(w: *mut VfsInode) {
    unsafe {
        (*w).c.i_lru_next = INODE_FREE as *mut InodeCommon;
        INODE_FREE = w;
    }
}
