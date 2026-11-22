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
use crate::fs::vfs::pagecache::AddressSpace;
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
    /// The dentry died while readers held references (ADR-0025): the
    /// release is deferred to the last wrapper_put.
    pub dying: bool,
    /// Slot generation (monotonic): the readahead hint's stale-slot
    /// guard — a recycled slot never matches an old hint.
    pub epoch: u64,
}

impl VfsInode {
    pub const fn empty() -> VfsInode {
        VfsInode {
            c: InodeCommon::empty(),
            xv6_ip: core::ptr::null_mut(),
            dying: false,
            epoch: 0,
        }
    }
}

static mut INODE_POOL: [VfsInode; NVINODE] = [const { VfsInode::empty() }; NVINODE];
/// Free wrapper chain threaded through `c.i_lru_next` (the s1 icache
/// LRU links are unused while a wrapper is free — s6 owns them).
static mut INODE_FREE: *mut VfsInode = core::ptr::null_mut();
static mut WRAP_EPOCH: u64 = 0;

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
        // i_count == 1: the owning dentry's reference (the icache
        // discipline, ADR-0025); readers add theirs via wrapper_get.
        e.c.i_count = 1;
        e.dying = false;
        WRAP_EPOCH += 1;
        e.epoch = WRAP_EPOCH;
        e.c.i_dev = (*ip).dev;
        e.c.i_ino = (*ip).inum;
        e.c.i_mode = iftype_to_mode((*ip).type_);
        e.c.i_op = &super::xv6_adapt::XV6_INODE_OPS;
        e.c.i_fop = &super::xv6_adapt::XV6_FILE_OPS;
        e.c.i_sb = &raw mut super::ROOT_SB;
        // The page-cache binding (m4r-s3): one mapping per wrapper,
        // 1:1 with its dentry — i_mapping points at the embedded
        // i_data (the Linux shape), regular files get the address-
        // space vtable.
        e.c.i_mapping = &raw mut e.c.i_data;
        e.c.i_data = AddressSpace {
            a_ops: if (*ip).type_ == crate::fs::xv6fs::layout::T_FILE {
                &super::xv6_adapt::XV6_ASPACE_OPS
            } else {
                core::ptr::null()
            },
            host: w as *mut u8,
            ..AddressSpace::EMPTY
        };
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

/// Release the dentry's reference on a wrapper (its dentry died).
/// With readers in flight (i_count > 1, e.g. a cached read sleeping
/// in readpage while an unlink negates the name — ADR-0025) the
/// release is deferred: the wrapper (and its mapping) stays alive
/// until the last wrapper_put, so pins never outlive their mapping.
/// Caller holds DENTRY_LOCK.
pub(crate) unsafe fn release_wrapper(w: *mut VfsInode) {
    unsafe {
        (*w).c.i_count -= 1; // the dentry's reference
        if (*w).c.i_count > 0 {
            (*w).dying = true;
            return; // wrapper_put finishes the release
        }
        finish_release_locked(w);
    }
}

/// The release tail, shared by release_wrapper and the deferred
/// wrapper_put: nlink == 0 defers the final iput to process context
/// via the zombie list (it may truncate + sleep); a plain drop
/// happens inline. Caller holds DENTRY_LOCK, i_count == 0.
unsafe fn finish_release_locked(w: *mut VfsInode) {
    unsafe {
        debug_assert!((*w).c.i_count == 0);
        let ip = (*w).xv6_ip;
        if (*ip).nlink < 1 {
            mutate::zombie_push(w); // owns the ref + slot for now
        } else {
            crate::fs::xv6fs::inode::iput(ip); // ref only; no sleep
            wrapper_free(w);
        }
    }
}

/// Find a live wrapper shadowing `ip` and take a reference on it
/// (i_count++; covers dying wrappers: an unlinked-but-open file keeps
/// its mapping until the last reader — the Linux page-cache shape).
/// The whole scan+take runs under DENTRY_LOCK, balanced against
/// release_wrapper's drop. Null: no cached name shadows the inode
/// (never walked, or long pruned) — callers fall back to uncached IO.
pub(crate) unsafe fn wrapper_get_for_ip(ip: *mut Xv6Inode) -> *mut VfsInode {
    unsafe {
        DENTRY_LOCK.raw_lock();
        let mut found = core::ptr::null_mut();
        for i in 0..NVINODE {
            let c = &raw mut INODE_POOL[i];
            // Live slots carry a mapping binding (wrapper_free clears
            // it); free-chain slots may hold stale xv6_ip values.
            if (*c).c.i_mapping.is_null() || (*c).xv6_ip != ip {
                continue;
            }
            (*c).c.i_count += 1;
            found = c;
            break;
        }
        DENTRY_LOCK.raw_release();
        found
    }
}

/// Drop a wrapper_get_for_ip reference. The last put on a dying
/// wrapper finishes the deferred dentry release. Must NOT be called
/// under PC_LOCK (the free path nests PC inside DENTRY).
pub(crate) unsafe fn wrapper_put(w: *mut VfsInode) {
    unsafe {
        DENTRY_LOCK.raw_lock();
        (*w).c.i_count -= 1;
        if (*w).c.i_count == 0 && (*w).dying {
            finish_release_locked(w);
        }
        DENTRY_LOCK.raw_release();
    }
}

/// Take a reader reference on a KNOWN wrapper (the kern_read_cached
/// shape: the walk's dentry pin does not survive an unlink's
/// negate-in-place, i_count does). Returns false when the wrapper
/// died in the walk-to-here window (freed slot) — callers fall back
/// to uncached IO.
pub(crate) unsafe fn wrapper_get(w: *mut VfsInode) -> bool {
    unsafe {
        DENTRY_LOCK.raw_lock();
        let live = !(*w).c.i_mapping.is_null();
        if live {
            (*w).c.i_count += 1;
        }
        DENTRY_LOCK.raw_release();
        live
    }
}

/// The wrapper's slot generation (readahead stale-hint guard).
pub(crate) fn wrapper_epoch(w: *mut VfsInode) -> u64 {
    unsafe { (*w).epoch }
}

/// Return a wrapper slot to the free chain. Caller holds DENTRY_LOCK.
/// The mapping dies here (dentry 1:1, ADR-0023/0024): idle clean pages
/// drop to the pool and their frames go back through free_data. A
/// pinned or dirty survivor is impossible in M4 (no write path; cache
/// readers never hold pins across calls) — fail loud rather than leak
/// pages into a recycled slot's fresh mapping.
pub(crate) unsafe fn wrapper_free(w: *mut VfsInode) {
    unsafe {
        debug_assert!(
            (*w).c.i_count == 0,
            "pagecache: wrapper_free with live reader refs"
        );
        // PC_LOCK nests inside the caller's DENTRY_LOCK (order pinned
        // in the pcache module doc; DENTRY_LOCK is held here).
        let _g = super::pcache::PC_LOCK.lock();
        let pc = super::pcache::pcache();
        let m = &mut (*w).c.i_data;
        pc.invalidate_mapping(m);
        assert!(
            m.nrpages == 0,
            "pagecache: mapping teardown with live pages"
        );
        (*w).c.i_mapping = core::ptr::null_mut();
        drop(_g);
        (*w).c.i_lru_next = INODE_FREE as *mut InodeCommon;
        INODE_FREE = w;
    }
}
