//! lru.rs: the global inactive list and clean-page reclaim — the
//! idle-page lifecycle (ref 0 ⇄ LRU) plus the two teardown paths:
//! reclaim (pool pressure, oldest-clean-first) and whole-mapping
//! invalidate (wrapper death in the kernel glue).

use super::page::PageDesc;
use super::tree::PageCache;
use super::AddressSpace;

impl PageCache {
    /// Pin a resident page: ref 0→1 takes it off the LRU (the dcache
    /// dget shape). Every lookup returns through here.
    ///
    /// # Safety
    /// `pg` must be a resident page of this engine.
    pub unsafe fn ref_page(&mut self, pg: *mut PageDesc) {
        // SAFETY: resident page; LRU links are engine-owned.
        unsafe {
            if (*pg).refcount == 0 {
                debug_assert!((*pg).on_lru);
                self.lru_unlink(pg);
            }
            (*pg).refcount += 1;
        }
    }

    /// Drop one pin: ref 1→0 parks the page on the LRU tail (MRU
    /// end). Underflow is a caller bug and panics (fail loud — a
    /// lost page cannot be reclaimed).
    ///
    /// # Safety
    /// `pg` must be a resident page with at least one pin held.
    pub unsafe fn put_page(&mut self, pg: *mut PageDesc) {
        // SAFETY: resident page under the caller's pin.
        unsafe {
            assert!((*pg).refcount >= 1, "pagecache: put without ref");
            (*pg).refcount -= 1;
            if (*pg).refcount == 0 {
                self.lru_push_tail(pg);
            }
        }
    }

    /// Oldest idle page (reclaim scan start; diagnostics/tests).
    pub fn lru_head(&self) -> *mut PageDesc {
        self.lru_head
    }

    unsafe fn lru_unlink(&mut self, pg: *mut PageDesc) {
        // SAFETY: pg is on the LRU (on_lru guards the call).
        unsafe {
            debug_assert!((*pg).on_lru);
            let prev = (*pg).l_prev;
            let next = (*pg).l_next;
            if !prev.is_null() {
                (*prev).l_next = next;
            } else {
                self.lru_head = next;
            }
            if !next.is_null() {
                (*next).l_prev = prev;
            } else {
                self.lru_tail = prev;
            }
            (*pg).l_next = core::ptr::null_mut();
            (*pg).l_prev = core::ptr::null_mut();
            (*pg).on_lru = false;
            self.stats.lru_len -= 1;
        }
    }

    unsafe fn lru_push_tail(&mut self, pg: *mut PageDesc) {
        // SAFETY: pg is off-LRU with clean links.
        unsafe {
            debug_assert!(!(*pg).on_lru);
            (*pg).l_prev = self.lru_tail;
            (*pg).l_next = core::ptr::null_mut();
            if !self.lru_tail.is_null() {
                (*self.lru_tail).l_next = pg;
            } else {
                self.lru_head = pg;
            }
            self.lru_tail = pg;
            (*pg).on_lru = true;
            self.stats.lru_len += 1;
        }
    }

    /// Reclaim up to `need` pages, oldest idle-clean first: the page
    /// leaves its mapping and the LRU, the frame goes back through
    /// the free_data callback, the descriptor returns to the pool.
    /// Dirty pages are skipped (never dropped — writeback is s5);
    /// returns the number actually reclaimed.
    pub fn reclaim_clean(&mut self, need: usize) -> usize {
        let mut freed = 0usize;
        while freed < need {
            // Oldest reclaimable candidate (scan skips dirty head
            // entries without unlinking them).
            let mut p = self.lru_head;
            let mut victim: *mut PageDesc = core::ptr::null_mut();
            while !p.is_null() {
                // SAFETY: LRU walk under the caller's lock.
                if !unsafe { (*p).dirty } {
                    victim = p;
                    break;
                }
                p = unsafe { (*p).l_next };
            }
            if victim.is_null() {
                break;
            }
            // SAFETY: victim is idle (LRU invariant), clean, resident.
            unsafe {
                let m = (*victim).mapping;
                debug_assert!((*victim).refcount == 0);
                self.lru_unlink(victim);
                *(*victim).t_pprev = (*victim).t_next;
                if !(*victim).t_next.is_null() {
                    (*(*victim).t_next).t_pprev = (*victim).t_pprev;
                }
                (*m).nrpages -= 1;
                if let Some(free_data) = self.free_data {
                    free_data((*victim).data);
                }
                self.free_desc(victim);
            }
            self.stats.nrpages -= 1;
            self.stats.evicts += 1;
            freed += 1;
        }
        freed
    }

    /// Drop every idle clean page of `m` (the kernel wrapper-teardown
    /// path: the mapping dies with its dentry, 1:1 — ADR-0023/0024).
    /// Pinned or dirty survivors stay in the tree (the caller decides
    /// whether that is legal; the s3 kernel glue cannot produce them
    /// and asserts the mapping drained). Returns pages dropped.
    ///
    /// # Safety
    /// `m` must be a live mapping of this engine and must not be
    /// reused (except a fresh init) afterwards.
    pub unsafe fn invalidate_mapping(&mut self, m: &mut AddressSpace) -> u32 {
        let mut dropped = 0u32;
        // SAFETY: ordered walk; removals only touch the current head,
        // so the cursor advance is always safe.
        unsafe {
            let mut p = m.root;
            while !p.is_null() {
                let next = (*p).t_next;
                if (*p).refcount == 0 && !(*p).dirty {
                    *(*p).t_pprev = (*p).t_next;
                    if !(*p).t_next.is_null() {
                        (*(*p).t_next).t_pprev = (*p).t_pprev;
                    }
                    self.lru_unlink(p);
                    if let Some(free_data) = self.free_data {
                        free_data((*p).data);
                    }
                    self.free_desc(p);
                    dropped += 1;
                }
                p = next;
            }
        }
        m.nrpages -= dropped;
        self.stats.nrpages -= dropped;
        self.stats.evicts += dropped as u64;
        dropped
    }
}
