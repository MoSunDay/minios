//! icache.rs: the inode cache, pure core (M4 s1, 23 §1).
//!
//! Intrusive hash of `InodeCommon` headers keyed (dev, ino) over a
//! caller-supplied bucket array, plus an idle LRU for inodes whose
//! i_count fell to zero (they stay hashed so a re-iget finds them,
//! exactly like Linux until the inode is evicted).
//!
//! Ownership discipline: the fs owns inode storage; the cache only
//! links headers. Eviction (`prune`) hands each victim to a dispose
//! callback — the kernel glue points that at the fs's destroy_inode +
//! slab free; deferred (RCU) release is the s2 glue's concern.

use super::ops::InodeCommon;

/// Power-of-two bucket count the caller must provide (hash masks with
/// it minus one).
pub struct Icache {
    buckets: *mut *mut InodeCommon,
    mask: usize,
    lru_head: *mut InodeCommon,
    lru_tail: *mut InodeCommon,
    pub stats: IcacheStats,
}

#[derive(Clone, Copy)]
pub struct IcacheStats {
    pub inserts: u64,
    pub hits: u64,
    pub misses: u64,
    pub evicts: u64,
    pub live: u32,
    pub idle: u32,
}

impl IcacheStats {
    /// Const zero (Icache::new is a const fn).
    pub const ZERO: IcacheStats = IcacheStats {
        inserts: 0,
        hits: 0,
        misses: 0,
        evicts: 0,
        live: 0,
        idle: 0,
    };
}

impl Icache {
    pub const fn new() -> Icache {
        Icache {
            buckets: core::ptr::null_mut(),
            mask: 0,
            lru_head: core::ptr::null_mut(),
            lru_tail: core::ptr::null_mut(),
            stats: IcacheStats::ZERO,
        }
    }

    /// Bind a zeroed bucket array of `nbuckets` (power of two).
    ///
    /// SAFETY: `buckets` must reference `nbuckets` writable,
    /// zero-initialized slots that stay alive and exclusive until the
    /// next init on this cache.
    pub unsafe fn init(&mut self, buckets: *mut *mut InodeCommon, nbuckets: usize) {
        self.buckets = buckets;
        self.mask = nbuckets - 1;
        self.lru_head = core::ptr::null_mut();
        self.lru_tail = core::ptr::null_mut();
        self.stats = IcacheStats::ZERO;
        for i in 0..nbuckets {
            // SAFETY: caller owns the array during init.
            unsafe { *buckets.add(i) = core::ptr::null_mut() };
        }
    }

    /// Insert a fresh inode (i_count managed by the caller; typically
    /// 1). Duplicate (dev, ino) is a caller bug and panics in debug —
    /// two fs reads of one inode must have converged via find().
    pub fn insert(&mut self, inode: *mut InodeCommon) {
        let key = Self::key(inode);
        let b = self.bucket(key);
        // SAFETY: inode is a live header the caller handed over.
        unsafe {
            (*inode).i_hash_next = *b;
            if !(*inode).i_hash_next.is_null() {
                (*(*inode).i_hash_next).i_hash_pprev = &mut (*inode).i_hash_next;
            }
            *b = inode;
            (*inode).i_hash_pprev = b;
        }
        self.stats.inserts += 1;
        self.stats.live += 1;
    }

    /// iget half: hash lookup; a hit bumps i_count and takes the inode
    /// off the idle LRU. Returns null on miss (caller reads the fs and
    /// inserts).
    pub fn find(&mut self, dev: u32, ino: u32) -> *mut InodeCommon {
        let key = (dev as u64) << 32 | ino as u64;
        // SAFETY: init bound a live bucket array.
        let mut cur = unsafe { *self.bucket(key) };
        self.stats.misses += 1;
        while !cur.is_null() {
            // SAFETY: hash members are live inodes.
            let n = unsafe { &mut *cur };
            if n.i_dev == dev && n.i_ino == ino {
                self.stats.misses -= 1;
                self.stats.hits += 1;
                if n.i_count == 0 {
                    self.lru_unlink(cur);
                }
                n.i_count += 1;
                return cur;
            }
            cur = n.i_hash_next;
        }
        core::ptr::null_mut()
    }

    /// iput half: drop a reference; zero parks the inode on the idle
    /// LRU (still hashed — evict happens via prune or fs dispose).
    pub fn put(&mut self, inode: *mut InodeCommon) {
        // SAFETY: caller holds the inode and one reference.
        let n = unsafe { &mut *inode };
        debug_assert!(n.i_count > 0, "iput underflow");
        n.i_count -= 1;
        if n.i_count == 0 {
            self.lru_push_tail(inode);
        }
    }

    /// Evict up to `n` idle inodes, oldest first, handing each to
    /// `dispose` (unhash happens here). Returns the number evicted.
    pub fn prune(&mut self, n: usize, dispose: fn(*mut InodeCommon)) -> usize {
        let mut freed = 0usize;
        let mut skips = 0u32;
        while freed < n {
            if skips > self.stats.idle {
                break;
            }
            let cand = self.lru_head;
            if cand.is_null() {
                break;
            }
            // SAFETY: LRU members are live inodes.
            let idle = unsafe { (*cand).i_count == 0 };
            if idle {
                self.lru_unlink(cand);
                self.unhash(cand);
                self.stats.live -= 1;
                self.stats.evicts += 1;
                dispose(cand);
                freed += 1;
                skips = 0;
            } else {
                self.lru_unlink(cand);
                self.lru_push_tail(cand);
                skips += 1;
            }
        }
        freed
    }

    #[inline]
    fn key(inode: *mut InodeCommon) -> u64 {
        // SAFETY: caller passed a live inode.
        unsafe { ((*inode).i_dev as u64) << 32 | (*inode).i_ino as u64 }
    }

    #[inline]
    fn bucket(&self, key: u64) -> *mut *mut InodeCommon {
        // SAFETY: init bound a live bucket array.
        unsafe { self.buckets.add((key as usize >> 5) & self.mask) }
    }

    fn unhash(&mut self, inode: *mut InodeCommon) {
        // SAFETY: inode is hashed.
        unsafe {
            let n = &mut *inode;
            if !n.i_hash_next.is_null() {
                (*n.i_hash_next).i_hash_pprev = n.i_hash_pprev;
            }
            *n.i_hash_pprev = n.i_hash_next;
            n.i_hash_next = core::ptr::null_mut();
            n.i_hash_pprev = core::ptr::null_mut();
        }
    }

    fn lru_push_tail(&mut self, inode: *mut InodeCommon) {
        // SAFETY: inode is live and off the LRU.
        unsafe {
            let n = &mut *inode;
            n.i_lru_prev = self.lru_tail;
            n.i_lru_next = core::ptr::null_mut();
            if !self.lru_tail.is_null() {
                (*self.lru_tail).i_lru_next = inode;
            } else {
                self.lru_head = inode;
            }
            self.lru_tail = inode;
        }
        self.stats.idle += 1;
    }

    fn lru_unlink(&mut self, inode: *mut InodeCommon) {
        // SAFETY: inode is on the LRU.
        unsafe {
            let n = &mut *inode;
            if !n.i_lru_prev.is_null() {
                (*n.i_lru_prev).i_lru_next = n.i_lru_next;
            } else {
                self.lru_head = n.i_lru_next;
            }
            if !n.i_lru_next.is_null() {
                (*n.i_lru_next).i_lru_prev = n.i_lru_prev;
            } else {
                self.lru_tail = n.i_lru_prev;
            }
            n.i_lru_next = core::ptr::null_mut();
            n.i_lru_prev = core::ptr::null_mut();
        }
        self.stats.idle -= 1;
    }
}
