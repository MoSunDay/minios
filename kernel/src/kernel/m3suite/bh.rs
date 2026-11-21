//! M3 bottom-half cases (21 §3): a kthread runs to completion and
//! reaps cleanly on the inline scheduler drive, workqueue `queue_work`
//! de-duplicates via the pending bit and re-queues finished items
//! through the events worker, and softirq covers ordering under the BH
//! mask, tasklet SCHED-bit scheduling, and ksoftirqd storm pressure.

use crate::kernel::softirq::{self, TaskletStruct};
use crate::kernel::workqueue::{self, WorkStruct};
use crate::proc::kthread;
use crate::proc::preempt::in_softirq;
use crate::proc::sched;
use crate::proc::{ProcState, PTABLE_LOCK};
use core::sync::atomic::{AtomicUsize, Ordering};

// ---- s5: kthread / workqueue / softirq (21 §3) ----

/// m3.kthread.run_exit: a kthread runs to completion on the inline
/// scheduler drive, exits through the zombie path, and reaps cleanly.
static KTH_DONE: AtomicUsize = AtomicUsize::new(0);

extern "C" fn kth_body(arg: usize) -> i32 {
    KTH_DONE.store(arg + 1, Ordering::SeqCst);
    42
}

fn kth_done() -> bool {
    KTH_DONE.load(Ordering::SeqCst) == 8
}

pub(super) fn case_kthread_run_exit() -> bool {
    // SAFETY: boot context owns the fresh slot; sched_drive borrows the
    // scheduler swtch-back context.
    unsafe {
        let p = kthread::kthread_create(kth_body, 7, b"kthprobe");
        if p.is_null() {
            return false;
        }
        kthread::kthread_run(p);
        if !sched::sched_drive(kth_done, 64) {
            return false;
        }
        {
            let _guard = PTABLE_LOCK.lock();
            if (*p).state != ProcState::ZOMBIE || (*p).xstate != 42 || (*p).mm.is_some() {
                return false;
            }
        }
        if kthread::kthread_reap(p) != 42 {
            return false;
        }
        let _guard = PTABLE_LOCK.lock();
        (*p).state == ProcState::UNUSED
    }
}

/// m3.workq.dedup_run: queue_work de-duplicates via the pending bit,
/// the events worker executes each queuing exactly once (sleepable
/// deferred work, in contrast with softirqs), and a finished item
/// re-queues.
static WORK_RAN: AtomicUsize = AtomicUsize::new(0);
static mut WORK_ITEM: WorkStruct = WorkStruct::new(probe_work);

fn probe_work(_w: *mut WorkStruct) {
    WORK_RAN.fetch_add(1, Ordering::SeqCst);
}

fn work_ran_once() -> bool {
    WORK_RAN.load(Ordering::SeqCst) == 1
}

fn work_ran_twice() -> bool {
    WORK_RAN.load(Ordering::SeqCst) == 2
}

pub(super) fn case_workq_dedup_run() -> bool {
    // SAFETY: boot context; the work item is a module static the worker
    // only touches between queue and run.
    unsafe {
        *core::ptr::addr_of_mut!(WORK_ITEM) = WorkStruct::new(probe_work);
        let ev = core::ptr::addr_of_mut!(workqueue::EVENTS);
        if !workqueue::queue_work(ev, core::ptr::addr_of_mut!(WORK_ITEM)) {
            return false;
        }
        if workqueue::queue_work(ev, core::ptr::addr_of_mut!(WORK_ITEM)) {
            return false; // second queuing must de-duplicate
        }
        if !sched::sched_drive(work_ran_once, 64) {
            return false;
        }
        // Pending cleared after the run: the same item queues again.
        if !workqueue::queue_work(ev, core::ptr::addr_of_mut!(WORK_ITEM)) {
            return false;
        }
        sched::sched_drive(work_ran_twice, 64)
    }
}

/// m3.softirq.order_bh_mask: raised bits run in ascending index order
/// with the BH depth held inside every handler.
static SOFT_ORDER: AtomicUsize = AtomicUsize::new(0);
static SOFT_RX_AT: AtomicUsize = AtomicUsize::new(0);
static SOFT_TX_AT: AtomicUsize = AtomicUsize::new(0);
static SOFT_IN_BH: AtomicUsize = AtomicUsize::new(0);

fn softirq_probe_rx(_d: usize) {
    if in_softirq() {
        SOFT_IN_BH.fetch_add(1, Ordering::SeqCst);
    }
    SOFT_RX_AT.store(SOFT_ORDER.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
}

fn softirq_probe_tx(_d: usize) {
    if in_softirq() {
        SOFT_IN_BH.fetch_add(1, Ordering::SeqCst);
    }
    SOFT_TX_AT.store(SOFT_ORDER.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
}

pub(super) fn case_softirq_order_bh_mask() -> bool {
    const UNSET: usize = usize::MAX;
    // SAFETY: quiet boot context; NET_TX/NET_RX are unused until M4
    // net. Probing TX(2)+RX(3) leaves HI(0) and TASKLET(5) owned by
    // softirq::init, so the later tasklet case sees its handlers.
    unsafe {
        softirq::open_softirq(softirq::NET_TX_SOFTIRQ, softirq_probe_tx, 0);
        softirq::open_softirq(softirq::NET_RX_SOFTIRQ, softirq_probe_rx, 0);
    }
    SOFT_ORDER.store(0, Ordering::SeqCst);
    SOFT_RX_AT.store(UNSET, Ordering::SeqCst);
    SOFT_TX_AT.store(UNSET, Ordering::SeqCst);
    SOFT_IN_BH.store(0, Ordering::SeqCst);
    softirq::raise_softirq(softirq::NET_TX_SOFTIRQ);
    softirq::raise_softirq(softirq::NET_RX_SOFTIRQ);
    let want = (1 << softirq::NET_TX_SOFTIRQ) | (1 << softirq::NET_RX_SOFTIRQ);
    if softirq::pending() != want {
        return false;
    }
    softirq::do_softirq();
    softirq::pending() == 0
        && SOFT_TX_AT.load(Ordering::SeqCst) == 0
        && SOFT_RX_AT.load(Ordering::SeqCst) == 1
        && SOFT_IN_BH.load(Ordering::SeqCst) == 2
}

/// m3.softirq.tasklet_sched: SCHED-bit de-dup, single run per schedule,
/// and re-scheduling after completion (normal + HI lists).
static TASKLET_RAN: AtomicUsize = AtomicUsize::new(0);
static TASKLET_DATA: AtomicUsize = AtomicUsize::new(0);
static mut TL_ITEM: TaskletStruct = TaskletStruct::new(tl_fn, 5);

fn tl_fn(d: usize) {
    TASKLET_DATA.store(d, Ordering::SeqCst);
    TASKLET_RAN.fetch_add(1, Ordering::SeqCst);
}

pub(super) fn case_softirq_tasklet_sched() -> bool {
    // SAFETY: module static; run only between schedule and drain.
    unsafe {
        *core::ptr::addr_of_mut!(TL_ITEM) = TaskletStruct::new(tl_fn, 5);
        let t = core::ptr::addr_of_mut!(TL_ITEM);
        if !softirq::schedule_tasklet(t) {
            return false;
        }
        if softirq::schedule_tasklet(t) {
            return false; // SCHED bit de-duplicates
        }
        softirq::do_softirq();
        if TASKLET_RAN.load(Ordering::SeqCst) != 1 || TASKLET_DATA.load(Ordering::SeqCst) != 5 {
            return false;
        }
        if !softirq::schedule_tasklet(t) {
            return false; // SCHED cleared by the run: schedulable again
        }
        softirq::do_softirq();
        if TASKLET_RAN.load(Ordering::SeqCst) != 2 {
            return false;
        }
        if !softirq::schedule_tasklet_hi(t) {
            return false; // the HI list rides the same discipline
        }
        softirq::do_softirq();
        TASKLET_RAN.load(Ordering::SeqCst) == 3 && softirq::pending() == 0
    }
}

/// m3.softirq.ksoftirqd_pressure: a self-raising storm exhausts the
/// do_softirq budget, the leftover mask wakes ksoftirqd (RUNNABLE),
/// and once the storm stops the thread drains the backlog and sleeps.
static STORM_ON: AtomicUsize = AtomicUsize::new(0);
static STORM_LOOPS: AtomicUsize = AtomicUsize::new(0);

fn softirq_probe_storm(_d: usize) {
    if STORM_ON.load(Ordering::SeqCst) == 1 {
        softirq::raise_softirq(softirq::NET_RX_SOFTIRQ);
    }
    STORM_LOOPS.fetch_add(1, Ordering::SeqCst);
}

fn pressure_drained() -> bool {
    softirq::pending() == 0 && softirq::ksoftirqd_state() == ProcState::SLEEPING
}

pub(super) fn case_softirq_ksoftirqd_pressure() -> bool {
    // SAFETY: quiet boot context; NET_RX is unused until M4 net.
    unsafe {
        softirq::open_softirq(softirq::NET_RX_SOFTIRQ, softirq_probe_storm, 0);
    }
    let ks = softirq::ksoftirqd_proc();
    if ks.is_null() {
        return false;
    }
    {
        let _guard = PTABLE_LOCK.lock();
        // SAFETY: ptable lock held.
        unsafe {
            if (*ks).state != ProcState::SLEEPING {
                return false;
            }
        }
    }
    STORM_ON.store(1, Ordering::SeqCst);
    STORM_LOOPS.store(0, Ordering::SeqCst);
    softirq::raise_softirq(softirq::NET_RX_SOFTIRQ);
    softirq::do_softirq();
    // Budget exhausted with a leftover mask, and ksoftirqd was woken.
    if softirq::pending() == 0 {
        return false;
    }
    if STORM_LOOPS.load(Ordering::SeqCst) < softirq::MAX_SOFTIRQ_RESTART {
        return false;
    }
    {
        let _guard = PTABLE_LOCK.lock();
        // SAFETY: ptable lock held.
        unsafe {
            if (*ks).state != ProcState::RUNNABLE {
                return false;
            }
        }
    }
    // Storm off: ksoftirqd drains the backlog and parks again.
    STORM_ON.store(0, Ordering::SeqCst);
    sched::sched_drive(pressure_drained, 4096)
}
