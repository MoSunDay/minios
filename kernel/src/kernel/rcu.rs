//! RCU kernel glue (UP, single "classic" tiny flavor; 21 §5, M3 s7):
//! read-side markers over a global reader count, quiescent-state
//! reports from the voluntary-swtch / user-return / user-tick sites,
//! callback plumbing on RCU_SOFTIRQ and the synchronous services.
//! The grace-period state machine itself is rcu_engine.rs (pure,
//! host-testable); this file is kernel-only.
//!
//! Read-side contract (audited in debug builds): a critical section
//! must not sleep (`sleep`/`wq_sleep` debug-assert the per-proc
//! nesting) and must not hold the ptable lock sleepably. Unlike stock
//! TINY_RCU — which relies on UP readers never being switched out —
//! QS reporting is suppressed while ANY context is inside a read
//! section, so a reader that (against the rules) parks mid-section
//! merely delays reclamation until it exits (divergence list in
//! ADR-0022).
//!
//! Lock order: PTABLE_LOCK -> RCU_LOCK (the swtch QS-report site);
//! callbacks always run OUTSIDE RCU_LOCK (func is the kfree family or
//! a wait-queue wake taking PTABLE_LOCK — the reverse nesting never
//! happens). `call_rcu` is safe from any context (short internal
//! spin); `synchronize_rcu`/`rcu_barrier` sleep and must not be
//! called with the ptable lock held.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::rcu_engine::RcuEngine;
pub use super::rcu_engine::RcuHead;
use crate::klib::sync::Spinlock;
use crate::proc::{self, waitqueue, Proc};

/// Contexts inside an RCU read section (any context: a parked reader
/// keeps this nonzero and defers the grace period).
static ACTIVE_READERS: AtomicUsize = AtomicUsize::new(0);

/// Serializes engine/list mutation (short leaf sections; ordered
/// after PTABLE_LOCK — see the module doc).
static RCU_LOCK: Spinlock<()> = Spinlock::new("rcu", ());

static mut ENGINE: RcuEngine = RcuEngine::new();

/// synchronize_rcu / rcu_barrier sleepers park here.
#[allow(dead_code)] // s7b: m3 RCU cases
static mut RCU_WAITQ: waitqueue::WaitQueue = waitqueue::WaitQueue::new();

fn engine_mut() -> &'static mut RcuEngine {
    // SAFETY: static access without a reference; RCU_LOCK held.
    unsafe { &mut *ptr::addr_of_mut!(ENGINE) }
}

fn raise_rcu_softirq() {
    crate::kernel::softirq::raise_softirq(crate::kernel::softirq::RCU_SOFTIRQ);
}

/// Reader-side dereference: read-once + acquire — a reader that starts
/// after the writer's `rcu_assign!` publication sees the new pointer.
#[macro_export]
macro_rules! rcu_deref {
    ($p:expr) => {
        ($p).load(core::sync::atomic::Ordering::Acquire)
    };
}

/// Writer-side publication: release store — all prior writes to the
/// new generation are visible before the pointer itself flips.
#[macro_export]
macro_rules! rcu_assign {
    ($p:expr, $v:expr) => {
        ($p).store($v, core::sync::atomic::Ordering::Release)
    };
}

/// Enter an RCU read-side critical section (UP non-preemptible: a
/// barrier + the audit marker; rules in the module doc). Never blocks,
/// never sleeps; safe from any context.
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn rcu_read_lock() {
    ACTIVE_READERS.fetch_add(1, Ordering::SeqCst);
    let p = proc::proc::myproc();
    if !p.is_null() {
        // SAFETY: the running (or interrupted-on-this-CPU) process.
        unsafe { (*p).rcu_read_nesting += 1 };
    }
}

/// Leave the read side. The last reader out reports the quiescent
/// state the switch-out could not (deferred QS), unblocking the GP.
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn rcu_read_unlock() {
    let p = proc::proc::myproc();
    if !p.is_null() {
        // SAFETY: as in rcu_read_lock.
        unsafe {
            debug_assert!((*p).rcu_read_nesting > 0, "unbalanced rcu_read_unlock");
            (*p).rcu_read_nesting -= 1;
        }
    }
    if ACTIVE_READERS.fetch_sub(1, Ordering::SeqCst) == 1 {
        note_qs();
    }
}

/// A quiescent-state report (swtch / user return / user tick / last
/// reader out). Raises RCU_SOFTIRQ when a GP may now complete.
fn note_qs() {
    let may_complete = {
        let _g = RCU_LOCK.lock();
        let readers = ACTIVE_READERS.load(Ordering::SeqCst) > 0;
        engine_mut().note_qs(readers)
    };
    if may_complete {
        raise_rcu_softirq();
    }
}

/// sched_raw hook: the outgoing context voluntarily swtches (a
/// quiescent state unless a reader is parked mid-section).
pub fn note_context_switch() {
    note_qs();
}

/// scheduler()/sched_drive() idle hook: when nothing is runnable the
/// CPU runs the scheduler context, which holds no read sections — a
/// quiescent state. Without this report a GP that opened after the
/// last swtch would stall forever while every proc blocks (idle
/// liveness; the mirror of Linux reporting QS from idle). No-op when
/// no GP is open or a reader is parked mid-section.
pub fn note_idle() {
    note_qs();
}

/// User-return hook (userret_deliver head): user mode holds no read
/// sections; still checked against parked readers.
pub fn user_return() {
    note_qs();
}

/// Tick hook (Linux rcu_check_callbacks): a user-mode tick is a QS; a
/// kernel-mode tick just opportunistically raises the softirq.
pub fn check_callbacks(user: bool) {
    if user {
        note_qs();
        return;
    }
    let work = {
        let _g = RCU_LOCK.lock();
        engine_mut().has_work()
    };
    if work {
        raise_rcu_softirq();
    }
}

/// Queue `func(head)` for invocation after a full grace period. Safe
/// from any context (short internal spin section).
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn call_rcu(head: *mut RcuHead, func: fn(*mut RcuHead)) {
    // SAFETY: caller-owned node until invocation.
    unsafe { (*head).func = func };
    {
        let _g = RCU_LOCK.lock();
        engine_mut().queue_cb(head);
    }
    raise_rcu_softirq();
}

/// RCU_SOFTIRQ body: advance the batch pipeline, then invoke
/// completed callbacks OUTSIDE RCU_LOCK (kfree family / wake-ups).
fn rcu_process_callbacks(_data: usize) {
    let done = {
        let _g = RCU_LOCK.lock();
        let eng = engine_mut();
        eng.advance();
        eng.take_done()
    };
    let n = RcuEngine::invoke(done);
    if n > 0 {
        let _g = RCU_LOCK.lock();
        engine_mut().add_invoked(n);
    }
}

/// Register the RCU softirq (m1r_boot common init; nothing pending).
pub fn init() {
    // SAFETY: boot init, quiet context.
    unsafe {
        crate::kernel::softirq::open_softirq(
            crate::kernel::softirq::RCU_SOFTIRQ,
            rcu_process_callbacks,
            0,
        )
    };
}

// ---- synchronous services (process context; never under ptable) ----

/// A stack-resident marker: its callback (a grace period later) sets
/// `done` and wakes RCU_WAITQ. The sleeper outlives the callback by
/// construction — it cannot return before its own wake-up.
#[repr(C)]
#[allow(dead_code)] // s7b: m3 RCU cases
struct SyncDone {
    done: AtomicBool,
    head: RcuHead,
}

#[allow(dead_code)] // s7b: m3 RCU cases
fn sync_done_cb(h: *mut RcuHead) {
    let sd = ((h as usize) - core::mem::offset_of!(SyncDone, head)) as *mut SyncDone;
    // SAFETY: the sleeper owns `sd` until it observes `done`.
    unsafe {
        (*sd).done.store(true, Ordering::Release);
        // SAFETY: static wait queue; bh context (ptable under us).
        crate::proc::sleep::wake_up(&mut *ptr::addr_of_mut!(RCU_WAITQ), false);
    }
}

/// Shared body of synchronize_rcu / rcu_barrier: queue a marker
/// callback, park on RCU_WAITQ until it fires. FIFO batches make the
/// marker's invocation imply every earlier callback's invocation.
#[allow(dead_code)] // s7b: m3 RCU cases
fn wait_for_callbacks() {
    let mut sd = SyncDone {
        done: AtomicBool::new(false),
        head: RcuHead::new(sync_done_cb),
    };
    call_rcu(&mut sd.head, sync_done_cb);
    while !sd.done.load(Ordering::Acquire) {
        let g = RCU_LOCK.lock();
        if sd.done.load(Ordering::Acquire) {
            drop(g);
            break;
        }
        // SAFETY: RCU_LOCK held (guard) — released across the sleep
        // and re-acquired before wq_sleep returns; our wait link is
        // unqueued; interrupts stay masked through the handoff dance.
        unsafe { proc::sleep::wq_sleep(ptr::addr_of_mut!(RCU_WAITQ), &RCU_LOCK, false, false) };
        drop(g);
    }
}

/// Wait for a full grace period (every reader active at call time —
/// and any earlier one — has left its read section).
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn synchronize_rcu() {
    wait_for_callbacks();
}

/// Wait until every callback queued before this call has been invoked
/// (pressure cases close with a zeroed account).
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn rcu_barrier() {
    wait_for_callbacks();
}

// ---- probes (m3-suite state checks) ----

/// (grace periods completed, callbacks invoked) under RCU_LOCK.
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn stats() -> (u64, u64) {
    let _g = RCU_LOCK.lock();
    engine_mut().stats()
}

/// Callbacks still waiting for (or inside) a grace period.
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn pending_work() -> bool {
    let _g = RCU_LOCK.lock();
    engine_mut().has_work()
}

/// Contexts currently inside RCU read sections.
#[allow(dead_code)] // s7b: m3 RCU cases
pub fn active_readers() -> usize {
    ACTIVE_READERS.load(Ordering::SeqCst)
}

/// Debug audit hook for the sleep paths: the current process must not
/// sleep inside an RCU read-side critical section (module-doc rules).
pub fn debug_assert_no_reader(p: *mut Proc) {
    // SAFETY: caller passes the current proc (myproc).
    unsafe {
        debug_assert!((*p).rcu_read_nesting == 0, "sleep inside RCU read section");
    }
}
