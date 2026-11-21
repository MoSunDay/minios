//! dcache/icache pure-core tests (m4r-s1): refcounts, LRU discipline,
//! negative entries, rename invalidation, leaf-ordered prune, pool
//! recycling, and the inode cache — driving the real kernel files
//! (included by path; no test-only copy to drift).

use crate::vfs::dcache::{Dcache, Dentry};
use crate::vfs::dentry::RenameOutcome;
use crate::vfs::icache::Icache;
use crate::vfs::ops::InodeCommon;
use crate::vfs::{VfsErr, S_IFDIR, S_IFREG};
use std::cell::RefCell;

const POOL: usize = 16;

/// A leaked arena of empty Dentry slots (the kernel spells this as a
/// static pool; leaking keeps the test honest about lifetime).
fn dcache() -> (Dcache, *mut Dentry) {
    let pool = Box::leak(vec![Dentry::empty(); POOL].into_boxed_slice());
    let mut dc = Dcache::new();
    // SAFETY: pool is exclusively ours for the whole test.
    unsafe { dc.init(pool.as_mut_ptr(), POOL) };
    (dc, pool.as_mut_ptr())
}

fn inode(mode: u16, ino: u32) -> *mut InodeCommon {
    let n = Box::leak(Box::new(InodeCommon::empty()));
    n.i_mode = mode;
    n.i_ino = ino;
    n.i_dev = 1;
    n
}

fn dir_ino(ino: u32) -> *mut InodeCommon {
    inode(S_IFDIR | 0o755, ino)
}

/// "/" with sub "s" (dir) and files "a", "b".
fn tree() -> Dcache {
    let (mut dc, _pool) = dcache();
    let root = dc.insert_root(dir_ino(1), core::ptr::null_mut()).unwrap();
    let s = dc.insert(root, b"s", dir_ino(2)).unwrap();
    let a = dc.insert(root, b"a", inode(S_IFREG | 0o644, 3)).unwrap();
    let b = dc.insert(s, b"b", inode(S_IFREG | 0o600, 4)).unwrap();
    dc.dput(a);
    dc.dput(b);
    dc.dput(s);
    dc
}

#[test]
fn dcache_insert_lookup_and_stats() {
    let mut dc = tree();
    let root = dc.root;
    let hit = dc.lookup(root, b"a");
    assert!(!hit.is_null());
    assert!(!unsafe { (*hit).is_negative() });
    assert!(dc.lookup(root, b"zzz").is_null());
    assert_eq!(dc.stats.hits, 1);
    // lookups: insert(root,s)=1 insert(root,a)=1 insert(s,b)=1 + 2 here + rename-checks none
    assert!(dc.stats.lookups >= 5);
}

#[test]
fn dcache_negative_entries_lifecycle() {
    let (mut dc, _p) = dcache();
    let root = dc.insert_root(dir_ino(1), core::ptr::null_mut()).unwrap();
    dc.add_negative(root, b"gone").unwrap();
    assert_eq!(dc.stats.neg_entries, 1);
    let hit = dc.lookup(root, b"gone");
    assert!(!hit.is_null() && unsafe { (*hit).is_negative() });
    assert_eq!(dc.stats.neg_hits, 1);
    // Re-caching the miss is a no-op.
    dc.add_negative(root, b"gone").unwrap();
    assert_eq!(dc.stats.neg_entries, 1);
    // A positive create converts the negative entry in place.
    let d = dc.insert(root, b"gone", inode(S_IFREG | 0o644, 7)).unwrap();
    assert_eq!(unsafe { (*d).refcount() }, 1);
    assert_eq!(dc.stats.neg_entries, 0);
    assert!(!unsafe { (*dc.lookup(root, b"gone")).is_negative() });
    // Caching a miss over an existing positive name is a caller bug.
    assert_eq!(dc.add_negative(root, b"gone").unwrap_err(), VfsErr::Exist);
    assert_eq!(
        dc.insert(root, b"gone", inode(S_IFREG, 8)).unwrap_err(),
        VfsErr::Exist
    );
}

#[test]
fn dcache_refcount_and_lru_prune() {
    let mut dc = tree();
    let root = dc.root;
    // "a" is idle (walk released it); dget pins it.
    let a0 = dc.lookup(root, b"a");
    let a = dc.dget(a0);
    assert_eq!(unsafe { (*a).refcount() }, 1);
    // Pinned "a" and the pinned root survive; the idle "s"/"b" do not.
    assert_eq!(dc.prune_lru(8), 2, "idle s and b retire, a stays pinned");
    assert!(!dc.lookup(root, b"a").is_null());
    dc.dput(a);
    // Root stays (mount pin); idle leaf "a" can go.
    assert_eq!(dc.prune_lru(8), 1);
    assert!(dc.lookup(root, b"a").is_null());
}

#[test]
fn dcache_prune_leaf_order_and_subtree() {
    let mut dc = tree();
    let root = dc.root;
    // Idle leaves "a" and "b" go first; then "s" (now childless) —
    // leaf-order means "b" must retire before its parent "s".
    let freed = dc.prune_lru(4);
    assert_eq!(freed, 3, "a, b, then childless s retired");
    assert!(dc.lookup(root, b"s").is_null());
    assert_eq!(dc.stats.live, 1, "only the pinned root remains");
    assert_eq!(dc.stats.lru_len, 0);
}

#[test]
fn dcache_shrink_busy_children_block() {
    let (mut dc, _p) = dcache();
    let root = dc.insert_root(dir_ino(1), core::ptr::null_mut()).unwrap();
    let s = dc.insert(root, b"s", dir_ino(2)).unwrap();
    let b = dc.insert(s, b"b", inode(S_IFREG, 4)).unwrap();
    dc.dput(s);
    // b stays referenced: shrink(root) frees nothing under s.
    assert_eq!(dc.shrink_subtree(root), 2); // s blocked + b referenced
    assert!(!dc.lookup(root, b"s").is_null());
    dc.dput(b);
    // Now everything under root (but root itself) retires.
    assert_eq!(dc.shrink_subtree(root), 0);
    assert!(dc.lookup(root, b"s").is_null());
}

#[test]
fn dcache_rename_moves_and_replaces() {
    let mut dc = tree();
    let root = dc.root;
    let s0 = dc.lookup(root, b"s");
    let s = dc.dget(s0);
    // Cross-directory move: s/b -> /c.
    assert!(matches!(
        dc.rename(s, b"b", root, b"c").unwrap(),
        RenameOutcome::Moved
    ));
    let c = dc.lookup(root, b"c");
    assert_eq!(unsafe { (*(*c).inode()).i_ino }, 4);
    assert!(dc.lookup(s, b"b").is_null());
    // rename a -> c displaces inode 4 and installs inode 3.
    match dc.rename(root, b"a", root, b"c").unwrap() {
        RenameOutcome::Replaced { dentry, inode } => {
            assert_eq!(unsafe { (*inode).i_ino }, 4);
            assert!(!dentry.is_null());
            dc.dput(dentry);
        }
        _ => panic!("expected Replaced"),
    }
    let c20 = dc.lookup(root, b"c");
    let c2 = dc.dget(c20);
    assert_eq!(unsafe { (*(*c2).inode()).i_ino }, 3);
    assert!(dc.lookup(root, b"a").is_null());
    dc.dput(s);
    dc.dput(c2);
}

#[test]
fn dcache_rename_semantics_edges() {
    let mut dc = tree();
    let root = dc.root;
    // Missing source.
    assert_eq!(
        dc.rename(root, b"nope", root, b"x").unwrap_err(),
        VfsErr::NoEnt
    );
    // Same dir + name is a no-op success.
    assert!(matches!(
        dc.rename(root, b"a", root, b"a").unwrap(),
        RenameOutcome::NoOp
    ));
    // Renaming a dir under its own subtree is EINVAL.
    let s0 = dc.lookup(root, b"s");
    let s = dc.dget(s0);
    assert_eq!(
        dc.rename(root, b"s", s, b"deeper").unwrap_err(),
        VfsErr::Inval
    );
    // Renaming over a negative occupant: reported with null inode.
    dc.add_negative(s, b"neg").unwrap();
    let a0 = dc.lookup(root, b"a");
    let a = dc.dget(a0);
    match dc.rename(root, b"a", s, b"neg").unwrap() {
        RenameOutcome::Replaced { dentry, inode } => {
            assert!(inode.is_null());
            dc.dput(dentry);
        }
        _ => panic!("expected Replaced over negative"),
    }
    assert!(!unsafe { (*dc.lookup(s, b"neg")).is_negative() });
    dc.dput(s);
    dc.dput(a);
}

#[test]
fn dcache_detach_is_unlink() {
    let mut dc = tree();
    let root = dc.root;
    let a0 = dc.lookup(root, b"a");
    let a = dc.dget(a0);
    dc.detach(a);
    assert!(dc.lookup(root, b"a").is_null());
    // Name free again: a negative entry can take it.
    dc.add_negative(root, b"a").unwrap();
    dc.dput(a);
}

#[test]
fn dcache_pool_exhaustion_recycles_idle() {
    let (mut dc, _p) = dcache();
    let root = dc.insert_root(dir_ino(1), core::ptr::null_mut()).unwrap();
    // POOL-1 files (root holds one slot), all released.
    for i in 0..POOL - 1 {
        let d = dc
            .insert(
                root,
                format!("f{}", i).as_bytes(),
                inode(S_IFREG, i as u32 + 10),
            )
            .unwrap();
        dc.dput(d);
    }
    assert_eq!(dc.stats.live as usize, POOL);
    // One more insert must evict an idle entry automatically.
    let d = dc.insert(root, b"extra", inode(S_IFREG, 99)).unwrap();
    assert_eq!(dc.stats.evicts, 1);
    assert_eq!(dc.stats.live as usize, POOL);
    dc.dput(d);
    // Pin f1..f14 (f0 was recycled); "extra" stays idle and recycles.
    for i in 1..POOL - 1 {
        let hit = dc.lookup(root, format!("f{}", i).as_bytes());
        assert!(!hit.is_null(), "f{} must still be cached", i);
        dc.dget(hit);
    }
    let more = dc.insert(root, b"more", inode(S_IFREG, 100)).unwrap();
    assert_eq!(dc.stats.evicts, 2);
    assert_eq!(dc.stats.live as usize, POOL);
    // Everything live is now pinned: the pool is truly exhausted.
    assert_eq!(
        dc.insert(root, b"more2", inode(S_IFREG, 101)).unwrap_err(),
        VfsErr::NoSpc
    );
    let _keep_pinned = more;
}

#[test]
fn icache_find_refs_and_prune() {
    let buckets = Box::leak(vec![core::ptr::null_mut::<InodeCommon>(); 8].into_boxed_slice());
    let mut ic = Icache::new();
    // SAFETY: bucket array is exclusively ours.
    unsafe { ic.init(buckets.as_mut_ptr(), 8) };
    let n1 = dir_ino(1);
    let n2 = inode(S_IFREG | 0o644, 2);
    // SAFETY: both inodes are live and unhashed.
    unsafe {
        (*n1).i_count = 1;
        (*n2).i_count = 1;
    }
    ic.insert(n1);
    ic.insert(n2);
    assert_eq!(ic.stats.live, 2);
    // Hit bumps the count.
    let got = ic.find(1, 2);
    assert_eq!(got, n2);
    assert_eq!(unsafe { (*got).i_count }, 2);
    // Miss is null.
    assert!(ic.find(1, 77).is_null());
    assert_eq!(ic.stats.misses, 1);
    // Release both (the find's iget needs its own put); prune evicts
    // both idle inodes via dispose.
    ic.put(n1);
    ic.put(n2);
    ic.put(n2);
    assert_eq!(ic.stats.idle, 2);
    thread_local! {
        static EVICTED: RefCell<Vec<u32>> = RefCell::new(Vec::new());
    }
    fn dispose(i: *mut InodeCommon) {
        // SAFETY: prune hands live, unhashed inodes.
        let ino = unsafe { (*i).i_ino };
        EVICTED.with(|v| v.borrow_mut().push(ino));
    }
    let n = ic.prune(2, dispose);
    assert_eq!(n, 2);
    let evicted = EVICTED.with(|v| v.borrow().clone());
    assert!(evicted.contains(&1) && evicted.contains(&2));
    assert!(ic.find(1, 1).is_null());
    assert_eq!(ic.stats.live, 0);
}

/// s2b invalidate: absent and already-negative names are Miss; an idle
/// positive leaf detaches fully (Detached) with lookup now missing and
/// the slot reusable only after recycle.
#[test]
fn dcache_invalidate_idle_leaf_detaches() {
    let mut dc = tree();
    let root = dc.root;
    assert_eq!(
        dc.invalidate(root, b"zzz"),
        crate::vfs::dcache::InvalidateOutcome::Miss
    );
    // Fresh negative caches the miss; invalidating it again stays Miss.
    dc.add_negative(root, b"neg").unwrap();
    assert_eq!(
        dc.invalidate(root, b"neg"),
        crate::vfs::dcache::InvalidateOutcome::Miss
    );
    // "a" is an idle positive leaf (tree() dput it).
    let before = dc.stats.live;
    match dc.invalidate(root, b"a") {
        crate::vfs::dcache::InvalidateOutcome::Detached(d) => {
            assert_eq!(unsafe { (*d).refcount() }, 0);
            // Deferred-free protocol: not on the free list until recycle.
            let hit = dc.lookup(root, b"a");
            assert!(hit.is_null());
            let det = d;
            dc.recycle(det);
        }
        other => panic!("expected Detached, got {:?}", other),
    }
    assert_eq!(dc.stats.live, before - 1);
    // Recycled slot is allocatable again.
    let d = dc.insert(root, b"c", inode(S_IFREG | 0o644, 9)).unwrap();
    dc.dput(d);
    assert!(!dc.lookup(root, b"c").is_null());
}

/// s2b invalidate: a referenced entry (walk/cwd holds it) negates in
/// place — name lookups miss afterwards, the holder's object stays
/// sane, and a following insert re-flips it positive.
#[test]
fn dcache_invalidate_busy_negates_in_place() {
    let mut dc = tree();
    let root = dc.root;
    let hit = dc.lookup(root, b"a");
    let a = dc.dget(hit); // simulate a cwd/open walk ref
    assert_eq!(
        dc.invalidate(root, b"a"),
        crate::vfs::dcache::InvalidateOutcome::Negated
    );
    let hit = dc.lookup(root, b"a");
    assert!(!hit.is_null() && unsafe { (*hit).is_negative() });
    assert_eq!(unsafe { (*a).refcount() }, 1); // holder unaffected
                                               // Re-create the same name: the negative flips in place, holder's
                                               // dentry is the same object it pinned.
    let a2 = dc.insert(root, b"a", inode(S_IFREG | 0o644, 3)).unwrap();
    assert_eq!(a2, a);
    dc.dput(a2);
    dc.dput(a);
    assert_eq!(unsafe { (*a).refcount() }, 0);
}

/// s2b invalidate: a busy DIRECTORY (has children) negates in place
/// even with d_ref == 0 — children pin their parent slot.
#[test]
fn dcache_invalidate_parent_with_children_negates() {
    let mut dc = tree();
    let root = dc.root;
    // "s" is idle but has child "b".
    let s = dc.lookup(root, b"s");
    assert_eq!(unsafe { (*s).refcount() }, 0);
    assert_eq!(
        dc.invalidate(root, b"s"),
        crate::vfs::dcache::InvalidateOutcome::Negated
    );
    assert!(unsafe { (*dc.lookup(root, b"s")).is_negative() });
    // Child "b" survives under the negated parent (referenced holders
    // keep a sane subtree); pruning still walks it.
    let b = dc.lookup(s, b"b");
    assert!(!b.is_null() && !unsafe { (*b).is_negative() });
}
