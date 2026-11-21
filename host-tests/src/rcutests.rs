//! rcu_model tests (m3r-s7): the TINY grace engine's batch state
//! machine — callback FIFO chains, the QS/GP advance sequence, batch
//! boundaries across multiple grace periods, and the parked-reader
//! deferral that backs m3.rcu.reader_defer. The model drives the real
//! engine file (included by path; no test-only copy to drift).

use crate::rcu_engine::{RcuEngine, RcuHead};
use core::mem::offset_of;
use std::cell::Cell;

thread_local! {
    static SEQ: Cell<u64> = Cell::new(0);
}

/// A callback-bearing node with an invocation-order slot (container_of
/// through the embedded RcuHead, exactly like kernel callbacks).
#[repr(C)]
struct Probe {
    at: Cell<u64>,
    head: RcuHead,
}

fn probe_cb(h: *mut RcuHead) {
    // SAFETY: every queued head embeds in a Probe (queue sites pass
    // exactly these pointers).
    let pr = unsafe { &*((h as usize - offset_of!(Probe, head)) as *const Probe) };
    pr.at.set(SEQ.with(|s| {
        let v = s.get() + 1;
        s.set(v);
        v
    }));
}

fn probe() -> Probe {
    Probe {
        at: Cell::new(0),
        head: RcuHead::new(probe_cb),
    }
}

/// One RCU_SOFTIRQ round: advance the pipeline, then invoke (the
/// kernel glue splits these only to drop the lock between the two).
fn step(e: &mut RcuEngine) -> u64 {
    e.advance();
    let n = RcuEngine::invoke(e.take_done());
    e.add_invoked(n);
    n
}

#[test]
fn callback_chain_fifo_order() {
    let mut e = RcuEngine::new();
    let mut ps = [probe(), probe(), probe(), probe()];
    assert!(e.is_idle());
    // Queue with no grace period open: work pending, nothing runs.
    for p in ps.iter_mut() {
        e.queue_cb(&mut p.head);
    }
    assert!(!e.is_idle() && e.has_work() && !e.gp_active());
    assert_eq!(step(&mut e), 0); // GP opens; no QS yet
    assert!(e.gp_active());
    assert!(e.note_qs(false));
    assert_eq!(step(&mut e), 4); // FIFO within the batch
    assert!(e.is_idle());
    let order: Vec<u64> = ps.iter().map(|p| p.at.get()).collect();
    assert_eq!(order, vec![1, 2, 3, 4]);
    assert_eq!(e.stats(), (1, 4));
}

#[test]
fn gp_state_machine_sequence() {
    let mut e = RcuEngine::new();
    let mut a = probe();
    let mut b = probe();
    e.queue_cb(&mut a.head);
    // A QS seen before the GP opens is not carried into it.
    assert!(!e.note_qs(false));
    assert_eq!(step(&mut e), 0);
    assert!(e.gp_active());
    // A QS with an active reader is suppressed.
    assert!(!e.note_qs(true));
    assert_eq!(step(&mut e), 0);
    assert!(e.gp_active());
    // A clean QS completes exactly the open GP.
    assert!(e.note_qs(false));
    assert_eq!(step(&mut e), 1);
    assert!(e.is_idle());
    // A late enqueue opens a fresh GP on the next softirq round.
    e.queue_cb(&mut b.head);
    assert_eq!(step(&mut e), 0);
    assert!(e.note_qs(false));
    assert_eq!(step(&mut e), 1);
    assert_eq!(e.stats(), (2, 2));
    assert!(a.at.get() < b.at.get());
}

#[test]
fn multiple_gp_boundaries() {
    let mut e = RcuEngine::new();
    let mut ps: Vec<Probe> = (0..6).map(|_| probe()).collect();
    // Batch 1 (two callbacks) opens the first GP.
    for p in ps.iter_mut().take(2) {
        e.queue_cb(&mut p.head);
    }
    step(&mut e);
    // Batch 2 queues while GP 1 is open — it must not ride GP 1.
    for p in ps.iter_mut().skip(2).take(2) {
        e.queue_cb(&mut p.head);
    }
    assert!(e.note_qs(false));
    assert_eq!(step(&mut e), 2); // only GP 1's batch drains
                                 // Batch 3 queues after GP 1 completed; both sit in nxt and merge
    for p in ps.iter_mut().skip(4) {
        e.queue_cb(&mut p.head);
    }
    assert_eq!(step(&mut e), 0); // GP 2 opens carrying the merged nxt list
    assert!(e.note_qs(false));
    assert_eq!(step(&mut e), 4); // batches 2+3 drain as one GP-2 batch
    assert!(e.is_idle());
    assert_eq!(e.stats(), (2, 6));
    // FIFO across batches: every earlier slot ran before any later one.
    let order: Vec<u64> = ps.iter().map(|p| p.at.get()).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
}

#[test]
fn deferred_reader_blocks_completion() {
    // The parked-reader model (m3.rcu.reader_defer): every QS while a
    // reader is inside a section is suppressed; the QS observed when
    // the last reader exits releases the batch.
    let mut e = RcuEngine::new();
    let mut p = probe();
    e.queue_cb(&mut p.head);
    step(&mut e); // GP opens
    for _ in 0..3 {
        assert!(!e.note_qs(true)); // reader parked across swtches
        assert_eq!(step(&mut e), 0);
    }
    assert!(e.gp_active());
    assert!(e.note_qs(false)); // last reader out
    assert_eq!(step(&mut e), 1);
    assert!(e.is_idle());
    assert_eq!(p.at.get(), 1);
}
