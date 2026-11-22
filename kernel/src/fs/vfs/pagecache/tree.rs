//! tree.rs: the per-mapping page index — insert / lookup / remove on
//! the engine (`PageCache` methods; the mapping is an argument, the
//! dcache `dc().lookup(parent, ..)` shape). The list stays ordered by
//! index so iteration (truncate/invalidate walk, s5) sees pages in
//! offset order.

use super::super::{VfsErr, VfsResult};
use super::page::{PageCacheStats, PageDesc};
use super::AddressSpace;

/// The page-cache engine. One per system (kernel static, host local):
/// owns the descriptor pool free list, the global idle LRU, and the
/// counters. All list surgery happens under the caller's lock (the
/// kernel glue serializes with its pcache spinlock — never taken from
/// interrupt context).
pub struct PageCache {
    pub stats: PageCacheStats,
    /// Owner callback for page frames leaving the cache (kernel:
    /// kfree). None leaks — the host default (tests leak their
    /// frames, the dcache `dispose` precedent).
    pub free_data: Option<fn(*mut u8)>,
    pub(super) free_head: *mut PageDesc,
    pub(super) lru_head: *mut PageDesc, // oldest idle
    pub(super) lru_tail: *mut PageDesc, // newest idle
}

impl PageCache {
    pub const fn new() -> PageCache {
        PageCache {
            stats: PageCacheStats::ZERO,
            free_data: None,
            free_head: core::ptr::null_mut(),
            lru_head: core::ptr::null_mut(),
            lru_tail: core::ptr::null_mut(),
        }
    }

    /// Bind a caller-owned descriptor pool (static arena in the
    /// kernel, boxed on the host). Boot/test context, no concurrency.
    ///
    /// # Safety
    /// `pool` must point to `n` contiguous PageDesc slots owned
    /// exclusively by this engine from now on.
    pub unsafe fn init(&mut self, pool: *mut PageDesc, n: usize) {
        self.free_head = core::ptr::null_mut();
        self.stats.nfree = 0;
        for i in 0..n {
            // SAFETY: caller-owned slot i of n; the free chain threads
            // t_next (tree links are dead while free).
            unsafe {
                *pool.add(i) = PageDesc::EMPTY;
                (*pool.add(i)).t_next = self.free_head;
                self.free_head = pool.add(i);
            }
            self.stats.nfree += 1;
        }
    }

    /// Take a descriptor off the free list, self-reclaiming one clean
    /// idle page when the pool is dry (pool pressure = the evict-clean
    /// driver). Err(NoSpc) when nothing is reclaimable.
    unsafe fn alloc_desc(&mut self) -> VfsResult<*mut PageDesc> {
        if self.free_head.is_null() {
            let freed = self.reclaim_clean(1);
            if freed == 0 || self.free_head.is_null() {
                return Err(VfsErr::NoSpc);
            }
        }
        // SAFETY: free_head is a live pool slot (init/put discipline).
        let d = self.free_head;
        unsafe { self.free_head = (*d).t_next };
        self.stats.nfree -= 1;
        Ok(d)
    }

    /// Count one readpage dispatch (the glue bumps this around its
    /// a_ops dispatch; the M4 gate reconciles it with misses).
    pub fn note_readpage(&mut self) {
        self.stats.readpages += 1;
    }

    /// Count readahead hints (M4 hook only; the window algorithm is
    /// Parity — 23 §1 discipline keeps the Core minimal).
    pub fn note_readahead(&mut self, n: u32) {
        self.stats.readahead += n as u64;
    }

    /// Return a descriptor to the free list. The page must already be
    /// off its tree and off the LRU.
    pub(super) unsafe fn free_desc(&mut self, d: *mut PageDesc) {
        // SAFETY: caller retired the page; blank the identity fields
        // so a stale pointer reads as free.
        unsafe {
            (*d) = PageDesc::EMPTY;
            (*d).t_next = self.free_head;
        }
        self.free_head = d;
        self.stats.nfree += 1;
    }
}

// ---- index operations (per mapping) ----

impl PageCache {
    /// Add a page frame at `index` and return it pinned (refcount 1;
    /// the caller reads/fills then put_page's). Duplicate index is a
    /// caller bug (Err(Busy) — the frame stays the caller's to free).
    /// The page starts !uptodate; readpage (the glue's dispatch)
    /// fills `data` and marks it.
    ///
    /// # Safety
    /// `data` must be a valid page frame owned by this call (it
    /// transfers to the cache for the page's residency); `m` must
    /// outlive the page.
    pub unsafe fn insert(
        &mut self,
        m: &mut AddressSpace,
        index: u64,
        data: *mut u8,
    ) -> VfsResult<*mut PageDesc> {
        let d = self.alloc_desc()?;
        // Ordered insert: walk while index < ours.
        // SAFETY: tree links are engine-owned; prev/pptr pairing is
        // the hlist discipline.
        unsafe {
            let mut pptr: *mut *mut PageDesc = &raw mut m.root;
            while !(*pptr).is_null() && (**pptr).index < index {
                pptr = &raw mut (**pptr).t_next;
            }
            if !(*pptr).is_null() && (**pptr).index == index {
                self.free_desc(d);
                return Err(VfsErr::Busy);
            }
            (*d) = PageDesc {
                t_pprev: pptr,
                t_next: *pptr,
                l_next: core::ptr::null_mut(),
                l_prev: core::ptr::null_mut(),
                on_lru: false,
                index,
                mapping: m as *mut AddressSpace,
                data,
                refcount: 1,
                dirty: false,
                uptodate: false,
                private: 0,
            };
            if !(*d).t_next.is_null() {
                (*(*d).t_next).t_pprev = &raw mut (*d).t_next;
            }
            *pptr = d;
        }
        m.nrpages += 1;
        self.stats.nrpages += 1;
        self.stats.inserts += 1;
        Ok(d)
    }

    /// find_get_page: hit returns the page pinned (caller put_page's
    /// exactly once); miss returns null. Counts hits/misses.
    ///
    /// # Safety
    /// `m` must be a live mapping bound to this engine.
    pub unsafe fn lookup(&mut self, m: &mut AddressSpace, index: u64) -> *mut PageDesc {
        // SAFETY: ordered-list walk over engine-owned links.
        let mut p = m.root;
        while !p.is_null() && unsafe { (*p).index } < index {
            p = unsafe { (*p).t_next };
        }
        if !p.is_null() && unsafe { (*p).index } == index {
            self.stats.hits += 1;
            // SAFETY: p is a resident page of m.
            unsafe { self.ref_page(p) };
            p
        } else {
            self.stats.misses += 1;
            core::ptr::null_mut()
        }
    }

    /// Explicit removal of a page the caller holds pinned (the
    /// failed-fill error path): delink from the tree and hand the
    /// frame back. Idle teardown goes through reclaim/invalidate
    /// instead.
    ///
    /// # Safety
    /// `pg` must be a resident page of `m` with refcount >= 1 and
    /// must not be used after this call.
    pub unsafe fn remove(&mut self, m: &mut AddressSpace, pg: *mut PageDesc) -> *mut u8 {
        // SAFETY: caller holds the only legitimate reference; tree
        // unlink is the pprev discipline.
        unsafe {
            debug_assert!((*pg).refcount >= 1);
            debug_assert!(!(*pg).on_lru);
            *(*pg).t_pprev = (*pg).t_next;
            if !(*pg).t_next.is_null() {
                (*(*pg).t_next).t_pprev = (*pg).t_pprev;
            }
            let data = (*pg).data;
            self.free_desc(pg);
            m.nrpages -= 1;
            self.stats.nrpages -= 1;
            data
        }
    }
}

/// The ADR-0025 fill-window gate classification of one page: Absent
/// (no page — the miss path runs), Filling (resident but !uptodate —
/// the insert-then-unlock-fill window; readers wait, never copy a
/// half-filled frame), or Ready (uptodate — bytes are complete).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Peek {
    Absent,
    /// Carries the page pointer symmetric to Ready (the gate's
    /// classifier AND its window-control hook both need it: closing
    /// the window flips a Filling page back).
    Filling(*mut PageDesc),
    /// Carries the page pointer (NO pin taken — callers copy or
    /// classify under their own lock hold).
    Ready(*mut PageDesc),
}

impl PageCache {
    /// Classify one page for the uptodate gate WITHOUT counters and
    /// WITHOUT a pin (the busy-wait loop must not pollute hit/miss
    /// stats; the caller copies under its own lock hold). This is the
    /// state machine the kernel glue's wait gate spins on.
    ///
    /// # Safety
    /// `m` must be a live mapping bound to this engine.
    pub unsafe fn peek(&mut self, m: &mut AddressSpace, index: u64) -> Peek {
        // SAFETY: ordered-list walk, read-only classification.
        let mut p = m.root;
        while !p.is_null() && unsafe { (*p).index } < index {
            p = unsafe { (*p).t_next };
        }
        if !p.is_null() && unsafe { (*p).index } == index {
            if unsafe { (*p).uptodate } {
                Peek::Ready(p)
            } else {
                Peek::Filling(p)
            }
        } else {
            Peek::Absent
        }
    }
}
