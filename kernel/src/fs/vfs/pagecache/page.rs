//! page.rs: the cached-page descriptor and the observable counters.
//!
//! A PageDesc is the VFS view of one page-frame in the cache: the
//! (mapping, index) key identifies it, `data` is the page frame
//! (PGSIZE bytes — the size itself lives with the frame layer, the
//! core stays size-agnostic), `refcount` is the pin count ABOVE
//! residency (0 == idle, LRU-eligible), and `dirty`/`uptodate` carry
//! the fill/writeback state. The descriptor is intrusive (tree links
//! + LRU links live inside it, the dcache discipline), so a caller
//! pool (static arena in the kernel, boxed on the host) owns the
//! storage and the engine threads the lists.
//!
//! Ref contract (Linux find_get_page shape): every lookup returns a
//! pinned page (refcount >= 1, off the LRU); every pin has exactly one
//! put_page. Idle pages (ref 0) sit on the global inactive LRU and are
//! reclaim candidates — dirty pages are NEVER reclaimable until
//! writeback cleans them (23 §2: no data loss in the name of
//! reclaim; writeback itself is s5).

/// One cached page. All links are owned by the engine; a free
/// descriptor has mapping == null and data == null.
#[derive(Clone)]
#[repr(C)]
pub struct PageDesc {
    /// Mapping-local tree links (index-ordered list; pprev unlink
    /// discipline).
    pub t_next: *mut PageDesc,
    pub t_pprev: *mut *mut PageDesc,
    /// Global idle-LRU links (only meaningful while refcount == 0).
    pub l_next: *mut PageDesc,
    pub l_prev: *mut PageDesc,
    /// Guards single-entry/detached LRU states (the d_on_lru trick).
    pub on_lru: bool,
    /// Page index within the mapping (byte offset = index * page size).
    pub index: u64,
    /// Owning address_space; null while the descriptor is free.
    pub mapping: *mut super::AddressSpace,
    /// The page frame's data pointer; null while free. Owned by the
    /// cache for resident pages (freed through the free_data callback).
    pub data: *mut u8,
    /// Pins above residency; 0 == idle (on the LRU).
    pub refcount: u32,
    /// Modified but not yet written back (s5 flips this; until then a
    /// set bit pins the page against reclaim, never a drop).
    pub dirty: bool,
    /// Filled from the backing store (readpage completed).
    pub uptodate: bool,
    /// Fs-private tag (buffer_head binding lands with s5 writeback).
    pub private: usize,
}

impl PageDesc {
    pub const EMPTY: PageDesc = PageDesc {
        t_next: core::ptr::null_mut(),
        t_pprev: core::ptr::null_mut(),
        l_next: core::ptr::null_mut(),
        l_prev: core::ptr::null_mut(),
        on_lru: false,
        index: 0,
        mapping: core::ptr::null_mut(),
        data: core::ptr::null_mut(),
        refcount: 0,
        dirty: false,
        uptodate: false,
        private: 0,
    };

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[inline]
    pub fn set_dirty(&mut self) {
        self.dirty = true;
    }

    #[inline]
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    #[inline]
    pub fn is_idle(&self) -> bool {
        self.refcount == 0
    }
}

/// Observable counters (rules/01: suite cases and the M4 gate anchor
/// on these numbers — hit/miss/readpage reconciliation — never on
/// timing windows).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PageCacheStats {
    /// Tree lookups that found the page (a returned page is pinned).
    pub hits: u64,
    /// Tree lookups that missed.
    pub misses: u64,
    /// Pages added to a mapping (fill path).
    pub inserts: u64,
    /// Idle clean pages dropped by reclaim (mapping + frame returned).
    pub evicts: u64,
    /// readpage dispatches (real backing-store reads; the gate's IO
    /// count together with misses).
    pub readpages: u64,
    /// Readahead dispatch hints (counter hook only in M4 — the window
    /// algorithm is Parity, 23 §1 discipline).
    pub readahead: u64,
    /// Pages currently resident across all mappings.
    pub nrpages: u32,
    /// Idle pages on the inactive LRU.
    pub lru_len: u32,
    /// Free descriptors in the pool.
    pub nfree: u32,
}

impl PageCacheStats {
    /// Const zero (PageCache::new is a const fn, the DcacheStats::ZERO
    /// precedent).
    pub const ZERO: PageCacheStats = PageCacheStats {
        hits: 0,
        misses: 0,
        inserts: 0,
        evicts: 0,
        readpages: 0,
        readahead: 0,
        nrpages: 0,
        lru_len: 0,
        nfree: 0,
    };
}
