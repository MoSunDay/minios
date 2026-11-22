//! pagecache pure-core tests (m4r-s3a): index insert/lookup/remove,
//! hit/miss/IO counters, the pin contract (ref ⇄ LRU), FIFO reclaim
//! of clean pages, dirty survival, pool-pressure self-reclaim, and
//! whole-mapping invalidate — driving the real kernel files (included
//! by path; no test-only copy to drift).

use crate::vfs::pagecache::{AddressSpace, PageCache, PageDesc, Peek};
use crate::vfs::VfsErr;
use std::sync::atomic::{AtomicUsize, Ordering};

const POOL: usize = 16;
const FRAME: usize = 4096;

/// Leaked descriptor arena (the kernel spells this as a static pool).
fn engine() -> (PageCache, *mut PageDesc) {
    let pool = Box::leak(vec![PageDesc::EMPTY; POOL].into_boxed_slice());
    let mut pc = PageCache::new();
    // SAFETY: pool is exclusively ours for the whole test.
    unsafe { pc.init(pool.as_mut_ptr(), POOL) };
    (pc, pool.as_mut_ptr())
}

/// A distinct leaked frame per call (the kernel's kalloc stand-in).
fn frame(seed: u8) -> *mut u8 {
    Box::leak(vec![seed; FRAME].into_boxed_slice()).as_mut_ptr()
}

fn mapping() -> &'static mut AddressSpace {
    Box::leak(Box::new(AddressSpace::EMPTY))
}

/// free_data callback: count + sum the handed-back pointers.
static FREED: AtomicUsize = AtomicUsize::new(0);
static FREED_SUM: AtomicUsize = AtomicUsize::new(0);
fn count_free(p: *mut u8) {
    FREED.fetch_add(1, Ordering::SeqCst);
    FREED_SUM.fetch_add(p as usize, Ordering::SeqCst);
}

fn freed() -> usize {
    FREED.load(Ordering::SeqCst)
}

#[test]
fn insert_lookup_roundtrip_and_stats() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    let frames = [frame(0), frame(1), frame(2)];
    let mut pgs = [core::ptr::null_mut(); 3];
    // Insert out of order; the tree must key by index, not sequence.
    for &(idx, f) in &[(2u64, frames[2]), (0, frames[0]), (1, frames[1])] {
        // SAFETY: distinct fresh frames, live mapping; the insert pin
        // is released right away (the resident-idle state under test).
        let pg = unsafe { pc.insert(m, idx, f).unwrap() };
        unsafe { pc.put_page(pg) };
        pgs[idx as usize] = pg;
    }
    assert_eq!(pc.stats.lru_len, 3);
    assert_eq!(m.nrpages, 3);
    assert_eq!(pc.stats.inserts, 3);
    assert_eq!(pc.stats.nrpages, 3);
    for idx in 0..3u64 {
        // SAFETY: live mapping.
        let pg = unsafe { pc.lookup(m, idx) };
        assert_eq!(pg, pgs[idx as usize]);
        assert!(!pg.is_null());
        // SAFETY: pinned by the lookup.
        unsafe {
            assert_eq!((*pg).index, idx);
            assert_eq!((*pg).mapping as usize, m as *mut _ as usize);
            assert_eq!((*pg).data, frames[idx as usize]);
            assert_eq!((*pg).refcount, 1);
            pc.put_page(pg);
        }
    }
    assert_eq!(pc.stats.hits, 3);
    assert_eq!(pc.stats.misses, 0);
}

#[test]
fn lookup_miss_counts() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 7) }.is_null());
    assert_eq!(pc.stats.misses, 1);
    assert_eq!(pc.stats.hits, 0);
}

#[test]
fn duplicate_index_refused() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: fresh frames, live mapping.
    unsafe {
        pc.insert(m, 4, frame(0x40)).unwrap();
        assert!(matches!(pc.insert(m, 4, frame(0x41)), Err(VfsErr::Busy)));
    }
    assert_eq!(m.nrpages, 1);
    assert_eq!(pc.stats.inserts, 1);
    assert_eq!(pc.stats.nfree, (POOL - 1) as u32);
}

#[test]
fn ref_contract_and_lru_transitions() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: fresh frame; pinned insert then the pin/put cycle.
    let pg = unsafe { pc.insert(m, 0, frame(1)).unwrap() };
    assert_eq!(pc.stats.lru_len, 0);
    unsafe { pc.put_page(pg) };
    assert_eq!(pc.stats.lru_len, 1);
    // SAFETY: resident page.
    unsafe {
        assert!((*pg).on_lru);
        let again = pc.lookup(m, 0);
        assert_eq!(again, pg);
        assert!(!(*pg).on_lru);
        assert_eq!((*pg).refcount, 1);
        pc.put_page(pg);
    }
    // LRU entry must be exactly one after the cycle (no duplicates).
    assert_eq!(pc.stats.lru_len, 1);
    assert_eq!(pc.stats.evicts, 0);
}

#[test]
#[should_panic(expected = "put without ref")]
fn double_put_panics() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: fresh frame, live mapping.
    let pg = unsafe { pc.insert(m, 0, frame(1)).unwrap() };
    // SAFETY: deliberately unbalanced (the contract violation).
    unsafe {
        pc.put_page(pg);
        pc.put_page(pg);
    }
}

#[test]
fn two_handles_share_one_page() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: fresh frame, live mapping.
    let f = frame(0x5a);
    let pg = unsafe { pc.insert(m, 9, f).unwrap() };
    unsafe { pc.put_page(pg) }; // resident, idle
                                // Two lookups = two handles on the same page/frame.
                                // SAFETY: live mapping.
    let h1 = unsafe { pc.lookup(m, 9) };
    let h2 = unsafe { pc.lookup(m, 9) };
    // SAFETY: both pinned.
    unsafe {
        assert_eq!(h1, h2);
        assert_eq!((*h1).data, f);
        assert_eq!((*h1).refcount, 2);
        pc.put_page(h1);
        assert_eq!((*h2).refcount, 1);
        assert!(!(*h2).on_lru); // still pinned: not reclaimable
        pc.put_page(h2);
        assert!((*h2).on_lru);
    }
    assert_eq!(pc.stats.hits, 2);
}

#[test]
fn reclaim_is_fifo_over_clean_idle() {
    let (mut pc, _pool) = engine();
    pc.free_data = Some(count_free);
    let m = mapping();
    let mut pgs = [core::ptr::null_mut(); 3];
    for (i, slot) in pgs.iter_mut().enumerate() {
        // SAFETY: fresh frames, live mapping.
        let pg = unsafe { pc.insert(m, i as u64, frame(i as u8)).unwrap() };
        unsafe { pc.put_page(pg) };
        *slot = pg;
    }
    let before = freed();
    // Ref/unref the middle page: it re-parks at the MRU end.
    // SAFETY: resident page.
    unsafe {
        pc.ref_page(pgs[1]);
        pc.put_page(pgs[1]);
    }
    assert_eq!(pc.reclaim_clean(2), 2);
    assert_eq!(freed(), before + 2);
    assert_eq!(pc.stats.evicts, 2);
    // FIFO despite the middle touch: pages 0 then 2 went.
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 0) }.is_null());
    // SAFETY: live mapping.
    let survivor = unsafe { pc.lookup(m, 1) };
    assert!(!survivor.is_null());
    // SAFETY: pinned by the lookup.
    unsafe { pc.put_page(survivor) };
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 2) }.is_null());
    assert_eq!(m.nrpages, 1);
}

#[test]
fn dirty_pages_survive_reclaim() {
    let (mut pc, _pool) = engine();
    pc.free_data = Some(count_free);
    let m = mapping();
    // SAFETY: fresh frames, live mapping.
    let pg = unsafe { pc.insert(m, 0, frame(9)).unwrap() };
    unsafe {
        (*pg).set_dirty();
        pc.put_page(pg);
    }
    let before = freed();
    assert_eq!(pc.reclaim_clean(5), 0);
    assert_eq!(freed(), before);
    assert_eq!(pc.stats.evicts, 0);
    // SAFETY: live mapping.
    let still = unsafe { pc.lookup(m, 0) };
    assert_eq!(still, pg);
    // SAFETY: pinned by the lookup; cleaning makes it reclaimable
    // again (the s5 writeback end-state).
    unsafe {
        (*still).clear_dirty();
        pc.put_page(still);
    }
    assert_eq!(pc.reclaim_clean(1), 1);
}

#[test]
fn pool_pressure_self_reclaims_oldest() {
    let (mut pc, _pool) = engine();
    pc.free_data = Some(count_free);
    let m = mapping();
    for i in 0..POOL + 4 {
        // SAFETY: fresh frames, live mapping.
        let pg = unsafe { pc.insert(m, i as u64, frame(i as u8)).unwrap() };
        unsafe { pc.put_page(pg) };
    }
    // The four oldest indices were evicted to make room; the newest
    // four stayed; inserts all succeeded.
    assert_eq!(pc.stats.evicts, 4);
    assert_eq!(m.nrpages, POOL as u32);
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 0) }.is_null());
    // SAFETY: live mapping.
    let fresh = unsafe { pc.lookup(m, (POOL + 3) as u64) };
    assert!(!fresh.is_null());
    // SAFETY: pinned by the lookup.
    unsafe { pc.put_page(fresh) };
}

#[test]
fn pool_exhaustion_with_dirty_tail_is_nospc() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    for i in 0..POOL {
        // SAFETY: fresh frames, live mapping.
        let pg = unsafe { pc.insert(m, i as u64, frame(i as u8)).unwrap() };
        // SAFETY: pinned; every page dirty before releasing.
        unsafe {
            (*pg).set_dirty();
            pc.put_page(pg);
        }
    }
    // No clean idle page exists: the (POOL+1)-th insert must refuse
    // rather than drop data (23 §2: reclaim never loses data).
    // SAFETY: fresh frame, live mapping.
    let f = frame(0xff);
    assert!(matches!(
        unsafe { pc.insert(m, POOL as u64, f) },
        Err(VfsErr::NoSpc)
    ));
    assert_eq!(pc.stats.evicts, 0);
    assert_eq!(m.nrpages, POOL as u32);
}

#[test]
fn remove_returns_the_frame() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    let f = frame(0x77);
    // SAFETY: fresh frame, live mapping.
    let pg = unsafe { pc.insert(m, 3, f).unwrap() };
    // SAFETY: caller holds the pin (insert contract).
    let back = unsafe { pc.remove(m, pg) };
    assert_eq!(back, f);
    assert_eq!(m.nrpages, 0);
    assert_eq!(pc.stats.nrpages, 0);
    assert_eq!(pc.stats.nfree, POOL as u32);
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 3) }.is_null());
}

#[test]
fn invalidate_mapping_drops_idle_keeps_pinned() {
    let (mut pc, _pool) = engine();
    pc.free_data = Some(count_free);
    let m = mapping();
    let mut pinned = core::ptr::null_mut();
    for i in 0..4u64 {
        // SAFETY: fresh frames, live mapping.
        let pg = unsafe { pc.insert(m, i, frame(i as u8)).unwrap() };
        if i == 2 {
            pinned = pg; // stays referenced across the invalidate
        } else {
            unsafe { pc.put_page(pg) };
        }
    }
    let before = freed();
    // SAFETY: live mapping being retired.
    let dropped = unsafe { pc.invalidate_mapping(m) };
    assert_eq!(dropped, 3);
    assert_eq!(freed(), before + 3);
    assert_eq!(m.nrpages, 1);
    // SAFETY: still resident and pinned.
    unsafe {
        assert!((*pinned).refcount == 1);
        assert!(!(*pinned).on_lru);
        pc.put_page(pinned); // now idle
    }
    // A second invalidate (the survivor went idle) drains it.
    // SAFETY: live mapping.
    assert_eq!(unsafe { pc.invalidate_mapping(m) }, 1);
    assert_eq!(m.nrpages, 0);
}

#[test]
fn readpage_and_readahead_counter_hooks() {
    let (mut pc, _pool) = engine();
    pc.note_readpage();
    pc.note_readpage();
    pc.note_readpage();
    pc.note_readahead(2);
    assert_eq!(pc.stats.readpages, 3);
    assert_eq!(pc.stats.readahead, 2);
}

#[test]
fn full_cycle_counter_reconciliation() {
    let (mut pc, _pool) = engine();
    pc.free_data = Some(count_free);
    let m = mapping();
    // One miss/fill/read/evict/refill cycle for a single index: the
    // gate-style arithmetic must close exactly.
    // SAFETY: fresh frames, live mapping.
    let f1 = frame(1);
    let pg = unsafe { pc.insert(m, 5, f1).unwrap() };
    pc.note_readpage();
    unsafe { pc.put_page(pg) };
    assert_eq!(pc.reclaim_clean(1), 1);
    // SAFETY: live mapping.
    assert!(unsafe { pc.lookup(m, 5) }.is_null());
    let f2 = frame(2);
    // SAFETY: fresh frame, live mapping.
    let pg2 = unsafe { pc.insert(m, 5, f2).unwrap() };
    pc.note_readpage();
    unsafe { pc.put_page(pg2) };
    // SAFETY: live mapping.
    let hit = unsafe { pc.lookup(m, 5) };
    assert!(!hit.is_null());
    // SAFETY: pinned.
    unsafe { pc.put_page(hit) };
    assert_eq!(pc.stats.misses, 1);
    assert_eq!(pc.stats.hits, 1);
    assert_eq!(pc.stats.inserts, 2);
    assert_eq!(pc.stats.evicts, 1);
    assert_eq!(pc.stats.readpages, 2);
    assert_eq!(pc.stats.nrpages, 1);
}

/// The ADR-0025 fill-window gate states: a fresh insert classifies as
/// Filling (!uptodate — the insert-then-unlock-fill window), flips to
/// Ready carrying the page once filled, and Absent after removal —
/// and none of it moves the hit/miss counters.
#[test]
fn peek_gate_states_and_counter_freedom() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    assert_eq!(unsafe { pc.peek(m, 0) }, Peek::Absent);
    // SAFETY: fresh frame, live mapping; insert pin held by the test.
    let f = frame(9);
    let pg = unsafe { pc.insert(m, 0, f).unwrap() };
    assert!(matches!(unsafe { pc.peek(m, 0) }, Peek::Filling(p) if p == pg));
    let (h0, m0) = (pc.stats.hits, pc.stats.misses);
    for _ in 0..3 {
        assert!(matches!(unsafe { pc.peek(m, 0) }, Peek::Filling(p) if p == pg));
    }
    // SAFETY: pinned page; the filler's completion mark.
    unsafe { (*pg).uptodate = true };
    match unsafe { pc.peek(m, 0) } {
        Peek::Ready(p) => assert_eq!(p, pg),
        other => panic!("expected Ready, got {:?}", other),
    }
    // Gate peeks are counter-free.
    assert_eq!((pc.stats.hits, pc.stats.misses), (h0, m0));
    // SAFETY: caller-held pin; removal hands the frame back.
    let back = unsafe { pc.remove(m, pg) };
    assert_eq!(back, f);
    assert_eq!(unsafe { pc.peek(m, 0) }, Peek::Absent);
}

/// The gate classifies per-index: neighbors stay Absent while one
/// page sits mid-fill.
#[test]
fn peek_gate_is_per_index() {
    let (mut pc, _pool) = engine();
    let m = mapping();
    // SAFETY: fresh frame, live mapping; held pin.
    let pg = unsafe { pc.insert(m, 7, frame(1)).unwrap() };
    assert_eq!(unsafe { pc.peek(m, 6) }, Peek::Absent);
    assert!(matches!(unsafe { pc.peek(m, 7) }, Peek::Filling(p) if p == pg));
    assert_eq!(unsafe { pc.peek(m, 8) }, Peek::Absent);
    // SAFETY: held pin.
    unsafe { pc.put_page(pg) };
    // Idle-but-filling pages are still reclaimable (clean + idle):
    // the window never blocks teardown.
    assert_eq!(pc.reclaim_clean(1), 1);
    assert_eq!(unsafe { pc.peek(m, 7) }, Peek::Absent);
}
