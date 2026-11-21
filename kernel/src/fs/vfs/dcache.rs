//! dcache.rs: the dentry cache engine, pure core (M4 s1, 23 §1).
//!
//! Arena-backed intrusive tree: the caller supplies a `Dentry` pool
//! (static in the kernel, boxed on the host) and the cache owns the
//! free list, per-parent child lists, and the unreferenced-entry LRU.
//! Child lookups are linear scans of the parent's list — honest at the
//! UP M4 scale, an explicit divergence from Linux's per-superblock
//! hash that ADR-0023 records with the lock hierarchy.
//!
//! Negative dentries: a lookup miss may be cached (`add_negative`) so
//! repeated ENOENT hits skip the fs; converting one to positive on
//! create is in place. In the kernel, negative-entry eviction is
//! RCU-deferred by the s2 glue (the first real call_rcu consumer,
//! ADR-0022); this core is synchronous and the glue wraps it.
//!
//! Invariant: every entry with `d_ref == 0` is on the LRU (except
//! transiently inside one call); `d_on_lru` keeps single-entry lists
//! and detached idles honest. Rename/shrink/prune policy lives in
//! devict.rs (same module).

use super::dentry::{DcacheStats, D_NEGATIVE, NAME_MAX};
use super::ops::{InodeCommon, SuperBlock};
use super::{VfsErr, VfsResult};

pub use super::dentry::Dentry;

/// Result of a coherence invalidation (unlink/rmdir/rename-over).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InvalidateOutcome {
    /// No positive entry under the name (absent or already negative).
    Miss,
    /// Entry referenced (walk/cwd/mount) or a parent: it stays in the
    /// tree but turns negative, so later lookups miss while the old
    /// reference holders keep a sane object.
    Negated,
    /// Idle leaf fully retired here (off tree, off LRU, counters done)
    /// EXCEPT the free-list return: the caller owns the slot and must
    /// hand it back via `recycle` — the kernel glue defers that through
    /// call_rcu so lockless readers drain before reuse (s2b).
    Detached(*mut Dentry),
}

pub struct Dcache {
    /// Owner callback for a retiring POSITIVE entry (the kernel glue
    /// releases the wrapper its d_inode names). No-op until set.
    pub dispose: fn(*mut Dentry),
    pub(super) pool: *mut Dentry,
    pub(super) cap: usize,
    pub(super) free: *mut Dentry,
    pub root: *mut Dentry,
    pub(super) lru_head: *mut Dentry,
    pub(super) lru_tail: *mut Dentry,
    pub stats: DcacheStats,
}

impl Dcache {
    pub const fn new() -> Dcache {
        Dcache {
            dispose: |_| {},
            pool: core::ptr::null_mut(),
            cap: 0,
            free: core::ptr::null_mut(),
            root: core::ptr::null_mut(),
            lru_head: core::ptr::null_mut(),
            lru_tail: core::ptr::null_mut(),
            stats: DcacheStats::ZERO,
        }
    }

    /// Bind the arena and rebuild the free list (fresh-boot spelling).
    ///
    /// SAFETY: `pool` must reference `cap` writable Dentry slots that
    /// stay alive and exclusive until the next init on this cache.
    pub unsafe fn init(&mut self, pool: *mut Dentry, cap: usize) {
        self.pool = pool;
        self.cap = cap;
        self.root = core::ptr::null_mut();
        self.lru_head = core::ptr::null_mut();
        self.lru_tail = core::ptr::null_mut();
        self.stats = DcacheStats::ZERO;
        self.free = core::ptr::null_mut();
        for i in (0..cap).rev() {
            // SAFETY: caller owns the arena slots during init.
            let d = unsafe { pool.add(i) };
            // SAFETY: slot is exclusively ours here.
            unsafe {
                *d = Dentry::empty();
                (*d).d_next = self.free;
            }
            self.free = d;
        }
    }

    /// Install a mount root dentry; the mount itself holds the pin
    /// (ref 1), so it never rides the LRU while mounted.
    pub fn insert_root(
        &mut self,
        inode: *mut InodeCommon,
        sb: *mut SuperBlock,
    ) -> VfsResult<*mut Dentry> {
        let d = self.alloc()?;
        // SAFETY: alloc returned an unattached pool slot we own.
        unsafe {
            (*d).d_parent = core::ptr::null_mut();
            (*d).d_inode = inode;
            (*d).d_sb = sb;
            (*d).d_ref = 1;
        }
        self.root = d;
        self.stats.live += 1;
        self.stats.inserts += 1;
        Ok(d)
    }

    /// Take a reference; an idle entry leaves the LRU.
    pub fn dget(&mut self, d: *mut Dentry) -> *mut Dentry {
        // SAFETY: caller holds the pointer valid for the call.
        let e = unsafe { &mut *d };
        if e.d_ref == 0 {
            debug_assert!(e.d_on_lru);
            self.lru_unlink(d);
        }
        e.d_ref += 1;
        d
    }

    /// Drop a reference; the last one parks the entry on the LRU.
    pub fn dput(&mut self, d: *mut Dentry) {
        // SAFETY: caller holds the pointer and one reference.
        let e = unsafe { &mut *d };
        debug_assert!(e.d_ref > 0, "dput underflow");
        e.d_ref -= 1;
        if e.d_ref == 0 {
            self.lru_push_tail(d);
        }
    }

    /// Find a child by name (linear scan of the parent's child list).
    /// Returns null on miss; hits may be negative — `is_negative()`.
    pub fn lookup(&mut self, parent: *mut Dentry, name: &[u8]) -> *mut Dentry {
        debug_assert!(name.len() <= NAME_MAX);
        self.stats.lookups += 1;
        let mut cur = self.first_child(parent);
        while !cur.is_null() {
            // SAFETY: list members are live children of parent.
            let c = unsafe { &*cur };
            if c.name() == name {
                if c.is_negative() {
                    self.stats.neg_hits += 1;
                } else {
                    self.stats.hits += 1;
                }
                return cur;
            }
            cur = c.d_next;
        }
        core::ptr::null_mut()
    }

    /// Attach a positive child (create/link/mkdir spelling). A
    /// positive occupant is `Exist`; a cached negative entry converts
    /// in place. The returned dentry carries ref 1 for the caller.
    pub fn insert(
        &mut self,
        parent: *mut Dentry,
        name: &[u8],
        inode: *mut InodeCommon,
    ) -> VfsResult<*mut Dentry> {
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(VfsErr::NameTooLong);
        }
        let hit = self.lookup(parent, name);
        if !hit.is_null() {
            // SAFETY: hit is a child of parent.
            if unsafe { (*hit).is_negative() } {
                // SAFETY: exclusive access for the conversion.
                let e = unsafe { &mut *hit };
                e.d_flags &= !D_NEGATIVE;
                e.d_inode = inode;
                self.stats.neg_entries -= 1;
                return Ok(self.dget(hit));
            }
            return Err(VfsErr::Exist);
        }
        let d = self.alloc()?;
        // SAFETY: fresh slot we own.
        unsafe {
            let e = &mut *d;
            e.d_name[..name.len()].copy_from_slice(name);
            e.d_name[name.len()] = 0;
            e.d_len = name.len() as u16;
            e.d_inode = inode;
            e.d_sb = (*parent).d_sb;
            e.d_ref = 1;
        }
        self.child_link(parent, d);
        self.stats.live += 1;
        self.stats.inserts += 1;
        Ok(d)
    }

    /// Cache a lookup miss. No-op when already cached; `Exist` when a
    /// positive entry holds the name (the fs just said NoEnt — bug).
    pub fn add_negative(&mut self, parent: *mut Dentry, name: &[u8]) -> VfsResult<()> {
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(VfsErr::NameTooLong);
        }
        let hit = self.lookup(parent, name);
        if !hit.is_null() {
            // SAFETY: child of parent.
            if unsafe { (*hit).is_negative() } {
                return Ok(());
            }
            return Err(VfsErr::Exist);
        }
        let d = self.alloc()?;
        // SAFETY: fresh slot we own.
        unsafe {
            let e = &mut *d;
            e.d_name[..name.len()].copy_from_slice(name);
            e.d_name[name.len()] = 0;
            e.d_len = name.len() as u16;
            e.d_flags = D_NEGATIVE;
            e.d_sb = (*parent).d_sb;
        }
        self.child_link(parent, d);
        self.lru_push_tail(d); // idle from birth
        self.stats.live += 1;
        self.stats.neg_entries += 1;
        Ok(())
    }

    /// Unhash a name (unlink/rmdir spelling): the dentry leaves the
    /// tree but keeps its refs; the last dput parks it on the LRU.
    pub fn detach(&mut self, d: *mut Dentry) {
        self.child_unlink(d);
    }

    /// Invalidate one child name (namespace coherence for
    /// unlink/rmdir/rename-over): an idle positive leaf is retired up
    /// to (but not including) the free-list push — `Detached` hands
    /// the slot to the caller for a deferred `recycle`; a referenced
    /// or child-bearing entry converts to negative in place; absent or
    /// already-negative names are `Miss`.
    pub fn invalidate(&mut self, parent: *mut Dentry, name: &[u8]) -> InvalidateOutcome {
        let hit = self.lookup(parent, name);
        if hit.is_null() {
            return InvalidateOutcome::Miss;
        }
        // SAFETY: hit is a live child of parent.
        let e = unsafe { &mut *hit };
        if e.is_negative() {
            return InvalidateOutcome::Miss;
        }
        if e.d_ref == 0 && e.d_first_child.is_null() {
            self.child_unlink(hit);
            self.lru_unlink(hit); // idle ⇔ on LRU (invariant)
            self.stats.live -= 1;
            self.stats.evicts += 1;
            InvalidateOutcome::Detached(hit)
        } else {
            e.d_flags |= super::dentry::D_NEGATIVE;
            e.d_inode = core::ptr::null_mut();
            self.stats.neg_entries += 1;
            InvalidateOutcome::Negated
        }
    }

    /// Return a `Detached` slot to the free list. Must not be called
    /// before the deferred-free grace period the caller requires.
    pub fn recycle(&mut self, d: *mut Dentry) {
        debug_assert_eq!(unsafe { (*d).d_ref }, 0);
        debug_assert!(unsafe { (*d).d_pprev.is_null() }, "recycle while attached");
        // SAFETY: caller owns the retired slot exclusively.
        unsafe {
            (*d).d_next = self.free;
        }
        self.free = d;
    }

    /// True when `maybe_desc` is `anc` itself or below it in the tree.
    pub(super) fn is_ancestor_or_self(&self, anc: *mut Dentry, maybe_desc: *mut Dentry) -> bool {
        let mut cur = maybe_desc;
        while !cur.is_null() {
            if cur == anc {
                return true;
            }
            // SAFETY: tree nodes carry valid parent pointers (root: 0).
            cur = unsafe { (*cur).d_parent };
        }
        false
    }

    #[inline]
    pub(super) fn first_child(&self, d: *mut Dentry) -> *mut Dentry {
        // SAFETY: caller passes a live node.
        unsafe { (*d).d_first_child }
    }

    /// Allocate from the free list; when empty, evict one idle entry.
    pub(super) fn alloc(&mut self) -> VfsResult<*mut Dentry> {
        if self.free.is_null() && self.prune_lru(1) == 0 {
            return Err(VfsErr::NoSpc); // dcache exhausted, all pinned
        }
        let d = self.free;
        // SAFETY: free-list members are exclusively owned slots.
        unsafe {
            self.free = (*d).d_next;
            let e = &mut *d;
            e.d_next = core::ptr::null_mut();
            e.d_pprev = core::ptr::null_mut();
            e.d_first_child = core::ptr::null_mut();
            e.d_flags = 0;
            e.d_inode = core::ptr::null_mut();
            e.d_len = 0;
            e.d_parent = core::ptr::null_mut();
            e.d_on_lru = false;
        }
        Ok(d)
    }

    /// Free an idle leaf (still attached): unlink from parent + LRU.
    pub(super) fn retire_entry(&mut self, d: *mut Dentry) {
        debug_assert!(unsafe { (*d).d_ref } == 0);
        debug_assert!(
            unsafe { (*d).d_first_child.is_null() },
            "retire with children"
        );
        self.child_unlink(d); // attached-or-detached both fine here
        self.retire_detached(d);
    }

    /// Free an idle detached entry (LRU + counters + free list).
    pub(super) fn retire_detached(&mut self, d: *mut Dentry) {
        // SAFETY: d is idle and detached.
        let (neg, on_lru) = unsafe { ((*d).is_negative(), (*d).d_on_lru) };
        debug_assert_eq!(unsafe { (*d).d_ref }, 0);
        debug_assert!(unsafe { (*d).d_pprev.is_null() }, "retire while attached");
        if neg {
            self.stats.neg_entries -= 1;
        } else {
            (self.dispose)(d); // release the wrapper before reuse
        }
        if on_lru {
            self.lru_unlink(d);
        }
        self.stats.live -= 1;
        self.stats.evicts += 1;
        // SAFETY: slot returns to our exclusive free list.
        unsafe {
            (*d).d_next = self.free;
        }
        self.free = d;
    }

    pub(super) fn child_link(&mut self, parent: *mut Dentry, d: *mut Dentry) {
        // SAFETY: both live; d is unattached.
        unsafe {
            (*d).d_parent = parent;
            (*d).d_pprev = &mut (*parent).d_first_child;
            (*d).d_next = (*parent).d_first_child;
            if !(*d).d_next.is_null() {
                (*(*d).d_next).d_pprev = &mut (*d).d_next;
            }
            (*parent).d_first_child = d;
        }
    }

    pub(super) fn child_unlink(&mut self, d: *mut Dentry) {
        // SAFETY: d carries valid hlist links or is already detached.
        unsafe {
            let e = &mut *d;
            if e.d_pprev.is_null() {
                return; // already detached
            }
            if !e.d_next.is_null() {
                (*e.d_next).d_pprev = e.d_pprev;
            }
            *e.d_pprev = e.d_next;
            e.d_next = core::ptr::null_mut();
            e.d_pprev = core::ptr::null_mut();
        }
    }

    pub(super) fn lru_push_tail(&mut self, d: *mut Dentry) {
        // SAFETY: d is live and not currently on the LRU.
        unsafe {
            let e = &mut *d;
            debug_assert!(!e.d_on_lru);
            e.d_lru_prev = self.lru_tail;
            e.d_lru_next = core::ptr::null_mut();
            e.d_on_lru = true;
            if !self.lru_tail.is_null() {
                (*self.lru_tail).d_lru_next = d;
            } else {
                self.lru_head = d;
            }
            self.lru_tail = d;
        }
        self.stats.lru_len += 1;
    }

    pub(super) fn lru_unlink(&mut self, d: *mut Dentry) {
        // SAFETY: d is on the LRU.
        unsafe {
            let e = &mut *d;
            debug_assert!(e.d_on_lru);
            if !e.d_lru_prev.is_null() {
                (*e.d_lru_prev).d_lru_next = e.d_lru_next;
            } else {
                self.lru_head = e.d_lru_next;
            }
            if !e.d_lru_next.is_null() {
                (*e.d_lru_next).d_lru_prev = e.d_lru_prev;
            } else {
                self.lru_tail = e.d_lru_prev;
            }
            e.d_lru_next = core::ptr::null_mut();
            e.d_lru_prev = core::ptr::null_mut();
            e.d_on_lru = false;
        }
        self.stats.lru_len -= 1;
    }
}
