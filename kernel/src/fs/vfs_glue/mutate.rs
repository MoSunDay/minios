//! Namespace mutations over the dcache (kern_create/kern_unlink/
//! kern_link) plus the two deferred lifecycles: unlinked inodes whose
//! wrappers hold the last reference (zombies — the final iput may
//! truncate + sleep, so it drains in process context) and invalidated
//! dentry slots (retired under DENTRY_LOCK, recycled after a grace
//! period via call_rcu — the kernel's first real RCU consumer,
//! ADR-0022's payoff).
//!
//! Every mutation runs in three phases: resolve + pin under
//! DENTRY_LOCK, fs-op unlocked (ilock/begin_op sleep between
//! processes), cohere under DENTRY_LOCK again. Pins are dentry refs,
//! which pin wrappers, which pin legacy inodes — an iget'd inode can
//! never be freed from under a cached name.

use super::xv6_adapt;
use super::{VfsInode, DENTRY_LOCK};
use crate::fs::vfs::dcache::InvalidateOutcome;
use crate::fs::vfs::dentry::Dentry;
use crate::fs::vfs::ops::InodeCommon;
use crate::fs::vfs::{VfsErr, VfsResult, S_IFDIR, S_IFREG};
use crate::fs::xv6fs::inode::Inode as Xv6Inode;
use crate::fs::xv6fs::layout::{DIRSIZ, T_DIR, T_FILE};
use crate::kernel::rcu::{call_rcu, RcuHead};

/// What kern_create builds (the sys_mkdir/sys_mknod/sys_open(O_CREAT)
/// kinds). Dev carries the rdev pair; the frozen vtable's create has
/// no rdev parameter (mknod is outside the M4 Core set), so Dev takes
/// the adapter's dedicated spelling instead of the vtable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    File,
    Dir,
    Dev(i16, i16),
}

/// create(2)-family core: resolve the parent, reuse an existing entry
/// when the kind matches (the sysfile.c create contract: file-on-file
/// only), else run the fs create and cache the name positive. Returns
/// an unlocked iget'd legacy inode (caller iput balances).
pub fn kern_create(path: &[u8], kind: CreateKind) -> VfsResult<*mut Xv6Inode> {
    reclaim_zombies();
    // Phase A: resolve + pin the parent, peek for an existing entry.
    let mut name = [0u8; DIRSIZ];
    // SAFETY: lock discipline per phase; pins keep wrappers alive.
    let (pd, existing) = unsafe {
        DENTRY_LOCK.raw_lock();
        let r = (|| -> VfsResult<(*mut Dentry, Option<*mut VfsInode>)> {
            let pd = super::parent_locked(path, &mut name)?;
            let n = name_len(&name);
            let hit = super::dc().lookup(pd, &name[..n]);
            if !hit.is_null() && !(*hit).is_negative() {
                return Ok((pd, Some((*hit).inode() as *mut VfsInode)));
            }
            Ok((pd, None))
        })();
        DENTRY_LOCK.raw_release();
        r?
    };
    // Existing entry: reuse on the sysfile.c rule (file-on-file).
    if let Some(w) = existing {
        // SAFETY: the wrapper pins the inode; type is stable.
        unsafe {
            let ip = (*w).xv6_ip;
            let ok = kind == CreateKind::File && (*ip).type_ == T_FILE;
            super::dput_unlocked(pd);
            if ok {
                return Ok(crate::fs::xv6fs::inode::idup(ip));
            }
            return Err(VfsErr::Exist);
        }
    }
    // Phase B: fs create (sleeps; the parent dentry pin holds the
    // wrapper, which holds the legacy inode).
    let dir_c = unsafe { (*pd).inode() };
    let mode = match kind {
        CreateKind::File => S_IFREG | 0o777,
        CreateKind::Dir => S_IFDIR | 0o777,
        CreateKind::Dev(_, _) => 0, // unused: dedicated spelling below
    };
    let n = name_len(&name);
    super::relieve_inode_pressure(2); // ialloc + dirlookup iget
                                      // SAFETY: vtable/op protocol; the wrapper outlives the call.
    let made = unsafe {
        match kind {
            CreateKind::Dev(major, minor) => {
                xv6_adapt::create_dev(dir_c, name.as_ptr(), n, major, minor)
            }
            CreateKind::Dir => ((*(*dir_c).i_op).mkdir)(dir_c, name.as_ptr(), n, mode),
            CreateKind::File => ((*(*dir_c).i_op).create)(dir_c, name.as_ptr(), n, mode),
        }
    };
    let made = match made {
        Ok(m) => m,
        Err(e) => {
            super::dput_unlocked(pd);
            return Err(e);
        }
    };
    let w = made as *mut VfsInode;
    // Phase C: cache the new name positive (flips a negative in place).
    // SAFETY: lock held; the wrapper ref transfers to the dentry.
    unsafe {
        DENTRY_LOCK.raw_lock();
        match super::dc().insert(pd, &name[..n], made) {
            Ok(d) => {
                super::dc().dput(d); // dentry rides the wrapper's ref
            }
            Err(_) => {
                // Lost a race to insert the same name: the winner's
                // entry stands; ours is redundant.
                super::release_wrapper(w);
            }
        }
        DENTRY_LOCK.raw_release();
        super::dput_unlocked(pd);
        Ok(crate::fs::xv6fs::inode::idup((*w).xv6_ip))
    }
}

/// unlink(2)/rmdir(2) core (xv6 folds them): remove the name, drop
/// nlink, invalidate the cached dentry (deferred recycle via
/// call_rcu), release the wrapper (zombie when the inode just lost
/// its last link).
pub fn kern_unlink(path: &[u8]) -> VfsResult<()> {
    reclaim_zombies();
    let mut name = [0u8; DIRSIZ];
    // SAFETY: phases per the module doc.
    unsafe {
        // Phase A: parent + pinned child dentry when cached positive.
        // The pin targets phase C's invalidate precisely (no prune or
        // same-name recreate can steal the slot mid-tx).
        DENTRY_LOCK.raw_lock();
        let r = (|| -> VfsResult<(*mut Dentry, *mut Dentry, *mut VfsInode)> {
            let pd = super::parent_locked(path, &mut name)?;
            let n = name_len(&name);
            let hit = super::dc().lookup(pd, &name[..n]);
            if hit.is_null() || (*hit).is_negative() {
                return Ok((pd, core::ptr::null_mut(), core::ptr::null_mut()));
            }
            let child = super::dc().dget(hit);
            let w = (*child).inode() as *mut VfsInode;
            Ok((pd, child, w))
        })();
        DENTRY_LOCK.raw_release();
        let (pd, child, w) = r?;
        // Phase B: the fs unlink (transaction; may sleep).
        let dir_c = (*pd).inode();
        let n = name_len(&name);
        super::relieve_inode_pressure(2); // adapter dirlookup iget
        let r = ((*(*dir_c).i_op).unlink)(dir_c, name.as_ptr(), n);
        // Phase C: coherence. Drop our pin first so an otherwise-idle
        // leaf detaches (exercising the deferred recycle) instead of
        // negating; children-bearing entries still negate in place.
        DENTRY_LOCK.raw_lock();
        match r {
            Ok(()) => {
                if !child.is_null() {
                    super::dc().dput(child);
                }
                match super::dc().invalidate(pd, &name[..n]) {
                    InvalidateOutcome::Detached(d) => {
                        retire_enqueue(d);
                        if !w.is_null() {
                            super::release_wrapper(w);
                        }
                    }
                    InvalidateOutcome::Negated => {
                        if !w.is_null() {
                            super::release_wrapper(w);
                        }
                    }
                    InvalidateOutcome::Miss => {}
                }
            }
            Err(_) => {
                if !child.is_null() {
                    super::dc().dput(child);
                }
            }
        }
        super::dc().dput(pd);
        DENTRY_LOCK.raw_release();
        r
    }
}

/// link(2) core: bump nlink, add the new name, cache it positive with
/// its own wrapper (one wrapper per dentry; each holds its own iget
/// ref). Pre-checks (old exists + not a dir, new parent resolvable,
/// new name free) run in phase A, so failure paths never touch the
/// disk — the legacy bump-then-unwind wrote a net-zero image anyway.
pub fn kern_link(oldpath: &[u8], newpath: &[u8]) -> VfsResult<()> {
    reclaim_zombies();
    let mut newname = [0u8; DIRSIZ];
    // SAFETY: phases per the module doc.
    unsafe {
        DENTRY_LOCK.raw_lock();
        let r = (|| -> VfsResult<(*mut Dentry, *mut Dentry)> {
            let od = super::lookup_d_locked(oldpath)?;
            let ow = (*od).inode() as *mut VfsInode;
            if (*(*ow).xv6_ip).type_ == T_DIR {
                super::dc().dput(od);
                return Err(VfsErr::Access); // legacy EPERM shape (s2c note)
            }
            let pd = match super::parent_locked(newpath, &mut newname) {
                Ok(p) => p,
                Err(_) => {
                    super::dc().dput(od);
                    return Err(VfsErr::NoEnt);
                }
            };
            let n = name_len(&newname);
            let hit = super::dc().lookup(pd, &newname[..n]);
            if !hit.is_null() && !(*hit).is_negative() {
                super::dc().dput(od);
                super::dc().dput(pd);
                return Err(VfsErr::Io); // legacy dirlink-fail spelling
            }
            Ok((od, pd))
        })();
        DENTRY_LOCK.raw_release();
        let (od, pd) = r?;
        // Phase B: nlink bump + new dirent in one transaction (the
        // adapter replicates the sys_link sequence verbatim).
        let old_c = (*od).inode();
        let dir_c = (*pd).inode();
        let n = name_len(&newname);
        super::relieve_inode_pressure(2); // adapter iget + dirlink
        let r = ((*(*dir_c).i_op).link)(old_c, dir_c, newname.as_ptr(), n);
        if let Err(e) = r {
            super::dput_unlocked(od);
            super::dput_unlocked(pd);
            return Err(e);
        }
        // Phase C: cache the new name with a fresh wrapper.
        let ip = (*(old_c as *mut VfsInode)).xv6_ip;
        DENTRY_LOCK.raw_lock();
        match super::wrap_new(crate::fs::xv6fs::inode::idup(ip)) {
            Ok(w2) => match super::dc().insert(pd, &newname[..n], w2 as *mut InodeCommon) {
                Ok(d) => {
                    super::dc().dput(d);
                }
                Err(_) => super::release_wrapper(w2),
            },
            Err(_) => {} // wrapper pool dry: the name stays uncached
        }
        super::dc().dput(od);
        super::dc().dput(pd);
        DENTRY_LOCK.raw_release();
        Ok(())
    }
}

/// Length of a zero-padded DIRSIZ name buffer.
fn name_len(name: &[u8; DIRSIZ]) -> usize {
    name.iter().position(|&b| b == 0).unwrap_or(DIRSIZ)
}

// ---- Zombie wrappers: unlinked inodes pinned by their wrapper ref ----

const NZOMBIE: usize = 64;
static mut ZOMBIES: [*mut VfsInode; NZOMBIE] = [core::ptr::null_mut(); NZOMBIE];
static mut NZOMB: usize = 0;

/// Queue a wrapper whose inode lost its last link (DENTRY_LOCK held).
/// The final iput may truncate + sleep, so it waits for process
/// context. Beyond NZOMBIE the kernel would leak — the assert names
/// the design limit instead of corrupting silently.
pub(super) unsafe fn zombie_push(w: *mut VfsInode) {
    unsafe {
        assert!(NZOMB < NZOMBIE, "vfs_glue: zombie overflow");
        ZOMBIES[NZOMB] = w;
        NZOMB += 1;
    }
}

/// Drain zombies (process context, no DENTRY_LOCK): iput inside a
/// transaction (the truncating path needs one), then free the slots.
/// Called at the head of every kern_* mutation entry.
pub fn reclaim_zombies() {
    // SAFETY: queue drained once; iput protocol per xv6.
    unsafe {
        if NZOMB == 0 {
            return;
        }
        let n = NZOMB;
        NZOMB = 0;
        for i in 0..n {
            let w = ZOMBIES[i];
            ZOMBIES[i] = core::ptr::null_mut();
            let ip = (*w).xv6_ip;
            crate::fs::xv6fs::log::begin_op();
            crate::fs::xv6fs::inode::iput(ip);
            crate::fs::xv6fs::log::end_op();
            DENTRY_LOCK.raw_lock();
            super::wrapper_free(w);
            DENTRY_LOCK.raw_release();
        }
    }
}

// ---- Deferred dentry recycle (call_rcu) ----

pub const NRETIRE: usize = 32;
static mut RETIRE_Q: [*mut Dentry; NRETIRE] = [core::ptr::null_mut(); NRETIRE];
static mut RETIRE_N: usize = 0;
static mut RETIRE_BUSY: bool = false;
static mut RETIRE_HEAD: RcuHead = RcuHead::new(retire_cb);

/// Hand a retired dentry slot to the grace-period queue (DENTRY_LOCK
/// held). While a grace period is outstanding the slot recycles
/// directly — the common case defers, the overflow keeps correctness.
pub(super) unsafe fn retire_enqueue(d: *mut Dentry) {
    unsafe {
        if RETIRE_BUSY {
            super::dc().recycle(d);
            return;
        }
        RETIRE_Q[RETIRE_N] = d;
        RETIRE_N += 1;
        RETIRE_BUSY = true;
        // RCU_LOCK is taken under DENTRY_LOCK here — the pinned order
        // (ADR-0023); callbacks run outside RCU_LOCK and take
        // DENTRY_LOCK fresh, so no inversion exists.
        call_rcu(&raw mut RETIRE_HEAD, retire_cb);
    }
}

/// Grace-period expiry: the slots are invisible to any reader that
/// started before the detach, so they may return to the free list.
fn retire_cb(_h: *mut RcuHead) {
    // SAFETY: DENTRY_LOCK fresh; the queue is owned by this callback.
    unsafe {
        DENTRY_LOCK.raw_lock();
        for i in 0..RETIRE_N {
            super::dc().recycle(RETIRE_Q[i]);
            RETIRE_Q[i] = core::ptr::null_mut();
        }
        RETIRE_N = 0;
        RETIRE_BUSY = false;
        DENTRY_LOCK.raw_release();
    }
}

/// VfsErr → errno for the syscall surface (battery-stable folding;
/// see the module doc for the NotDir caveat).
pub fn errno_of(e: VfsErr) -> i64 {
    match e {
        VfsErr::NoEnt => abi::ENOENT,
        VfsErr::NotDir => abi::ENOENT, // legacy miss spelling, m4suite revisits
        VfsErr::IsDir => abi::EISDIR,
        VfsErr::Access => abi::EACCES,
        VfsErr::SymLoop => abi::ELOOP,
        VfsErr::NameTooLong => abi::ENAMETOOLONG,
        VfsErr::Exist => abi::EEXIST,
        VfsErr::NotEmpty => abi::ENOTEMPTY,
        VfsErr::Inval => abi::EINVAL,
        VfsErr::Busy => abi::EBUSY,
        VfsErr::Io => abi::EIO,
        VfsErr::NoSpc => abi::ENOSPC,
        VfsErr::NotSup => abi::ENOSYS,
    }
}

/// Resolve a NUL-terminated kernel-resident path (exec/suite callers).
pub fn kern_namei_c(p: *const u8) -> VfsResult<*mut Xv6Inode> {
    let mut len = 0usize;
    // SAFETY: caller guarantees NUL termination.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: len readable bytes before the NUL.
    let path: &[u8] = unsafe { core::slice::from_raw_parts(p, len) };
    super::kern_namei(path)
}
