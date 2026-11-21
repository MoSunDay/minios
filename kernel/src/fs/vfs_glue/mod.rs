//! vfs_glue (m4r-s2b): the kernel-side VFS layer over the s1 pure
//! core. Owns the dcache arena, the root mount and the walk; the xv6fs
//! adapter (xv6_adapt.rs) fills the ops vtables without touching the
//! legacy inode/namei code. Namespace mutations run in three phases —
//! resolve under DENTRY_LOCK (pin dentries), fs-op with the lock
//! dropped (ilock/begin_op sleep), cohere under DENTRY_LOCK again —
//! because UP still sleeps between processes.
//!
//! Lock discipline (ADR-0023 material): the walk/mutate family uses
//! raw_lock/raw_release hand-off (a Spinlock guard cannot survive the
//! mid-critical-section release); the shims (dget/dput_unlocked/root_dget) keep
//! guards because they never cycle. Order: DENTRY_LOCK → xv6
//! ICACHE_LOCK (leaf) and DENTRY_LOCK → RCU_LOCK (call_rcu defers
//! dentry recycle); RCU callbacks run lock-free and take DENTRY_LOCK
//! fresh. The s1 Icache stays unconsumed this milestone — xv6fs
//! already keeps a legacy inode cache whose lifetime the VFS layer
//! cannot observe, so per-name wrappers (1:1 with their dentry) are
//! the honest binding; tmpfs (s6) becomes the icache's first consumer.
//!
//! Errno folding keeps the battery byte-stable: NotDir maps to ENOENT
//! (the legacy walker reported plain misses); ENOTDIR surfaces with
//! the m4suite walk cases at VfsErr level.

pub mod mutate;
mod wrapper;
pub mod xv6_adapt;
pub mod xv6_obj;

use crate::fs::vfs::dcache::Dcache;
use crate::fs::vfs::dentry::Dentry;
use crate::fs::vfs::namei::{may_search, Cred, PathCursor, MAY_EXEC};
use crate::fs::vfs::ops::{check_permission, InodeCommon};
use crate::fs::vfs::{VfsErr, VfsResult};
use crate::fs::xv6fs::inode::Inode as Xv6Inode;
use crate::klib::sync::Spinlock;

pub(crate) use wrapper::{release_wrapper, wrap_new, wrapper_free, VfsInode};

pub use mutate::{errno_of, kern_create, kern_link, kern_namei_c, kern_unlink, CreateKind};

/// Global dcache serialization (UP). Public for proc cwd refs and the
/// deferred-recycle callback.
pub static DENTRY_LOCK: Spinlock<()> = Spinlock::new("dcache", ());

const NDENTRY: usize = 256;

static mut DCACHE: Dcache = Dcache::new();
static mut DENTRY_POOL: [Dentry; NDENTRY] = [const { Dentry::empty() }; NDENTRY];

/// Exclusive Dcache borrow (UP discipline: every use sits under
/// DENTRY_LOCK or boot; the raw-pointer reborrow is the kernel's
/// established arena-access pattern).
pub(super) unsafe fn dc() -> &'static mut Dcache {
    unsafe { &mut *core::ptr::addr_of_mut!(DCACHE) }
}

/// The root mount's superblock (single mount this milestone).
static mut ROOT_SB: crate::fs::vfs::ops::SuperBlock = crate::fs::vfs::ops::SuperBlock {
    s_dev: 0,
    s_op: core::ptr::null(),
    s_root: core::ptr::null_mut(),
    s_private: core::ptr::null_mut(),
    s_flags: 0,
};

/// Mount the root filesystem and prime the dcache. Boot context, no
/// concurrency. Called from init_m1r_fs() after the xv6fs layers.
pub unsafe fn init() {
    unsafe {
        dc().init(
            core::ptr::addr_of_mut!(DENTRY_POOL).cast::<Dentry>(),
            NDENTRY,
        );
        dc().dispose = glue_dispose;
        wrapper::init();
        ROOT_SB.s_op = &xv6_adapt::XV6_SUPER_OPS;
        ROOT_SB.s_dev = crate::fs::xv6fs::layout::ROOTDEV;
        ROOT_SB.s_root = core::ptr::null_mut();
        let ip = crate::fs::xv6fs::inode::iget(
            crate::fs::xv6fs::layout::ROOTDEV,
            crate::fs::xv6fs::layout::ROOTINO,
        );
        crate::fs::xv6fs::inode::ilock(ip);
        let w = wrap_new(ip).ok().expect("vfs_glue: root wrapper");
        crate::fs::xv6fs::inode::iunlock(ip);
        let root = dc()
            .insert_root(&raw mut (*w).c, &raw mut ROOT_SB)
            .expect("vfs_glue: root dentry");
        ROOT_SB.s_root = root;
    }
}

/// Dispose hook: a pruned positive dentry releases its wrapper (the
/// ref discipline is identical to kern_unlink's phase C — nlink == 0
/// goes to the zombie list, else plain iput; neither sleeps).
fn glue_dispose(d: *mut Dentry) {
    // SAFETY: called under DENTRY_LOCK from the s1 retire paths.
    unsafe {
        let w = (*d).inode() as *mut VfsInode;
        if !w.is_null() {
            release_wrapper(w);
        }
    }
}

/// Held reference on the root dentry (proc cwd bookkeeping).
pub fn root_dget() -> *mut Dentry {
    // SAFETY: guard discipline; no lock cycling in this shim.
    unsafe {
        let _g = DENTRY_LOCK.lock();
        dc().dget(dc().root)
    }
}

/// dget/dput shims for proc.rs (fork/exit/chdir cwd refs).
pub fn dget(d: *mut Dentry) -> *mut Dentry {
    // SAFETY: caller owns a live reference on d.
    unsafe {
        let _g = DENTRY_LOCK.lock();
        dc().dget(d)
    }
}

/// Drop a dentry ref from UNLOCKED context (takes DENTRY_LOCK). Under
/// DENTRY_LOCK the only legal spelling is `dc().dput(d)` — calling this
/// shim there self-deadlocks (the spinlock is not reentrant).
pub fn dput_unlocked(d: *mut Dentry) {
    // SAFETY: caller owns the reference being dropped.
    unsafe {
        let _g = DENTRY_LOCK.lock();
        dc().dput(d);
    }
}

/// Resolve `path` to a legacy inode (unlocked, iget'd) with the xv6
/// namei contract: absolute walks start at the root, relative at the
/// caller's cwd dentry; misses return Err (the syscall layer folds to
/// errno). Positives and negatives land in the dcache along the way.
pub fn kern_namei(path: &[u8]) -> VfsResult<*mut Xv6Inode> {
    match kern_namei_d(path) {
        Ok((ip, d)) => {
            // The ref kern_namei_d returns is ALREADY idup'd (its
            // chdir contract parks it in proc state) — pass it
            // through; iduping again here leaked one ref per lookup
            // (m1r-user createdelete exhausted the 50-slot table).
            dput_unlocked(d);
            Ok(ip)
        }
        Err(e) => Err(e),
    }
}

/// kern_namei keeping the dentry reference too (chdir swaps both).
pub fn kern_namei_d(path: &[u8]) -> VfsResult<(*mut Xv6Inode, *mut Dentry)> {
    // SAFETY: raw lock discipline (the walk cycles the lock).
    unsafe {
        DENTRY_LOCK.raw_lock();
        // The raw xv6_ip belongs to the wrapper's iget ref; callers
        // (chdir) park it in proc.cwd, so it needs its own reference
        // (idup) — balanced by exit's iput. Without it the proc and
        // the wrapper would double-iput one ref (ilock ref<1 panic).
        let r = lookup_d_locked(path).map(|d| {
            let ip = crate::fs::xv6fs::inode::idup((*((*d).inode() as *mut VfsInode)).xv6_ip);
            (ip, d)
        });
        DENTRY_LOCK.raw_release();
        r
    }
}

/// Walk to a held dentry (DENTRY_LOCK held by the caller).
pub(super) unsafe fn lookup_d_locked(path: &[u8]) -> VfsResult<*mut Dentry> {
    unsafe {
        if dc().root.is_null() {
            return Err(VfsErr::NoEnt); // glue not mounted (defensive)
        }
        let start = walk_start(path);
        if start.is_null() {
            return Err(VfsErr::NoEnt);
        }
        walk_locked(dc().dget(start), path)
    }
}

/// Start dentry for a walk (DENTRY_LOCK held): root for absolute
/// paths, else the current process's cwd dentry.
unsafe fn walk_start(path: &[u8]) -> *mut Dentry {
    if PathCursor::is_absolute(path) {
        return dc().root;
    }
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return core::ptr::null_mut();
    }
    (*p).cwd_d
}

/// Locked walk (DENTRY_LOCK held; one owned reference on `cur` comes
/// in and one on the result goes out, dropped on error). Cache hits
/// stay under the lock; a miss drops the lock for the (sleeping) fs
/// lookup and re-checks before inserting, so no fs op ever runs under
/// DENTRY_LOCK. The lock is held again on every return.
unsafe fn walk_locked(mut cur: *mut Dentry, path: &[u8]) -> VfsResult<*mut Dentry> {
    unsafe {
        let mut cursor = match PathCursor::new(path) {
            Ok(c) => c,
            Err(e) => {
                dc().dput(cur);
                return Err(e);
            }
        };
        loop {
            if (*cur).is_negative() {
                dc().dput(cur);
                return Err(VfsErr::NoEnt);
            }
            let comp = match cursor.next() {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(cur),
                Err(e) => {
                    dc().dput(cur);
                    return Err(e);
                }
            };
            // Search gate: intermediate holders must be directories
            // the cred may cross (modes are rwxrwxrwx here, so only
            // the NotDir shape can ever fire — propagate it verbatim:
            // errno_of folds NotDir to ENOENT, the legacy walker's
            // plain-miss spelling, and the m4suite walk cases pin the
            // pre-fold ENOTDIR). Every failure below releases `cur` —
            // a leaked pin would strand the dentry (and its wrapper's
            // inode-table slot) forever.
            if let Err(e) = may_search(&*(*cur).inode(), &Cred::ROOT) {
                dc().dput(cur);
                return Err(e);
            }
            let parent = cur;
            if comp == b"." {
                continue;
            }
            if comp == b".." {
                let up = (*parent).parent();
                if !up.is_null() && up != parent {
                    cur = dc().dget(up);
                    dc().dput(parent);
                }
                continue;
            }
            match step_locked(parent, comp) {
                Ok(Some(next)) => {
                    dc().dput(parent);
                    cur = next;
                }
                Ok(None) => continue, // lock cycled under us; retry
                Err(e) => {
                    dc().dput(cur);
                    return Err(e);
                }
            }
        }
    }
}

/// Reclaim idle dentries until the legacy inode table has `need` free
/// slots. Wrapper-held iget refs are what pin table slots, and evicting
/// an idle dentry disposes its wrapper (iput drops the ref), so LRU
/// prune is the relief valve between the 256-slot dcache and the
/// 50-slot inode table. Called UNLOCKED before fs calls that may iget
/// or ialloc; dispose paths are ref-drop only (nlink 0 defers to the
/// zombie list), so the DENTRY_LOCK section here never sleeps.
pub(super) fn relieve_inode_pressure(need: usize) {
    // SAFETY: prune under DENTRY_LOCK; free_slots takes the table lock
    // (nested spin, never inverted: xv6fs never takes DENTRY_LOCK).
    unsafe {
        DENTRY_LOCK.raw_lock();
        while crate::fs::xv6fs::inode::free_slots() < need {
            if dc().prune_lru(1) == 0 {
                break; // LRU dry: only pinned entries remain
            }
        }
        DENTRY_LOCK.raw_release();
    }
}

/// One component resolution (DENTRY_LOCK held on entry and exit).
/// Ok(Some(d)) hands a new owned ref; Ok(None) asks the caller to
/// retry the component (the lock was cycled for an fs lookup and the
/// tree may have changed underneath).
unsafe fn step_locked(parent: *mut Dentry, comp: &[u8]) -> VfsResult<Option<*mut Dentry>> {
    // Legacy namecmp compares only DIRSIZ bytes per component.
    let comp = &comp[..core::cmp::min(comp.len(), crate::fs::xv6fs::layout::DIRSIZ)];
    unsafe {
        let hit = dc().lookup(parent, comp);
        if !hit.is_null() {
            if (*hit).is_negative() {
                return Err(VfsErr::NoEnt);
            }
            return Ok(Some(dc().dget(hit)));
        }
        // Miss: run the fs lookup unlocked (ilock can sleep). The
        // parent is pinned by the walk's own reference.
        let dir_c = (*parent).inode();
        check_permission(&*dir_c, MAY_EXEC, &Cred::ROOT)?;
        let lookup = (*(*dir_c).i_op).lookup;
        DENTRY_LOCK.raw_release();
        relieve_inode_pressure(2); // adapter iget may need a table slot
        let r = lookup(dir_c, comp.as_ptr(), comp.len());
        DENTRY_LOCK.raw_lock();
        // Re-check: a sleeper may have raced the same name in.
        let again = dc().lookup(parent, comp);
        if !again.is_null() {
            if let Ok(w) = r {
                // The freshly made wrapper is redundant: release it.
                release_wrapper(w as *mut VfsInode);
            }
            if (*again).is_negative() {
                return Err(VfsErr::NoEnt);
            }
            return Ok(Some(dc().dget(again)));
        }
        match r {
            Ok(w) => match dc().insert(parent, comp, w as *mut InodeCommon) {
                Ok(d) => Ok(Some(d)),
                Err(e) => {
                    // Dcache exhausted: the name exists on disk but
                    // cannot be cached — drop the wrapper (and its
                    // iget ref) before failing; leaking refs would
                    // pin the 50-slot inode table forever.
                    release_wrapper(w as *mut VfsInode);
                    Err(e)
                }
            },
            Err(VfsErr::NoEnt) => {
                let _ = dc().add_negative(parent, comp);
                Err(VfsErr::NoEnt)
            }
            Err(e) => Err(e),
        }
    }
}

/// Resolve the parent directory of the final component (the
/// nameiparent contract: the final name itself is NOT resolved).
/// Returns the held parent dentry; the final name lands in `name`
/// (DIRSIZ-capped, zero-padded — the legacy caller contract). The
/// DENTRY_LOCK is held throughout (the last hop needs no fs op).
pub(super) unsafe fn parent_locked(path: &[u8], name: &mut [u8]) -> VfsResult<*mut Dentry> {
    unsafe {
        if dc().root.is_null() {
            return Err(VfsErr::NoEnt);
        }
        let start = walk_start(path);
        if start.is_null() {
            return Err(VfsErr::NoEnt);
        }
        // Split at the last slash; the prefix walks normally. Trailing
        // slashes come off the whole path first (the legacy skipelem
        // discipline: nameiparent("foo/") yields name "foo", and a
        // path of only slashes is a miss).
        let path = trim_slashes(path);
        if path.is_empty() {
            return Err(VfsErr::NoEnt);
        }
        let (prefix, last) = match path.iter().rposition(|&b| b == b'/') {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => (b"".as_slice(), path),
        };
        // Legacy skipelem/namecmp: components act on their first
        // DIRSIZ bytes (xv6 silently truncates; usertests `fourteen`
        // pins the semantics — a 15-char name creates the 14-char one).
        let last = &last[..core::cmp::min(last.len(), crate::fs::xv6fs::layout::DIRSIZ)];
        name[..last.len()].copy_from_slice(last);
        for b in &mut name[last.len()..] {
            *b = 0;
        }
        if prefix.is_empty() {
            return Ok(dc().dget(start));
        }
        walk_locked(dc().dget(start), prefix)
    }
}

fn trim_slashes(b: &[u8]) -> &[u8] {
    let mut s = b;
    while let Some((&f, rest)) = s.split_first() {
        if f == b'/' {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((&l, rest)) = s.split_last() {
        if l == b'/' {
            s = rest;
        } else {
            break;
        }
    }
    s
}
