//! Kernel preemption is OFF at M3 (plans/linux26/21 §2): this module
//! makes that claim auditable instead of implicit. Critical sections
//! that must not schedule raise the UP preempt counter; sched_raw()
//! asserts it is zero before every context switch.
//!
//! Consumers land in later stages (softirq processing s5, RCU s7); the
//! counter itself and the audit hook are the M3 deliverable (ADR-0016).

use core::sync::atomic::{AtomicUsize, Ordering};

static PREEMPT_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn preempt_count() -> usize {
    PREEMPT_COUNT.load(Ordering::SeqCst)
}

/// Enter a non-preemptible critical section (pairs with preempt_enable).
#[allow(dead_code)] // s5/s7: softirq + RCU critical sections
pub fn preempt_disable() {
    PREEMPT_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Leave a non-preemptible critical section (panics on imbalance).
#[allow(dead_code)] // s5/s7: softirq + RCU critical sections
pub fn preempt_enable() {
    let prev = PREEMPT_COUNT.fetch_sub(1, Ordering::SeqCst);
    assert!(prev > 0, "preempt_enable without preempt_disable");
}

/// Audit hook: every scheduling path calls this (sched_raw).
pub fn assert_preemptible() {
    if preempt_count() != 0 {
        panic!("sched: scheduling with preemption disabled");
    }
}
