//! M3 kernel-mode suite: CFS policy (pick order, tick fairness), RT
//! FIFO/RR level semantics, scheduler wiring invariants, the s4 time
//! layer, and the s5 kthread/workqueue/softirq machinery. Runs in
//! the boot context before the scheduler starts (policy-level battery;
//! process-level dynamics are the s4 guest-time counters). Kernel-only
//! (no root disk): boot cmdline `m3-suite` -> CASE lines -> merged
//! verdict, exit 0x10 -> QEMU 33 (0x11 on any failure).

use crate::arch::x86_64::io;
use crate::console;
use crate::kernel::softirq::{self, TaskletStruct};
use crate::kernel::time::{clockevents, clocksource, hrtimer, wheel};
use crate::kernel::workqueue::{self, WorkStruct};
use crate::proc::kthread;
use crate::proc::preempt::in_softirq;
use crate::proc::sched;
use crate::proc::{cfs, rt, waitqueue, ProcState, SchedPolicy, PTABLE_LOCK};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Link-bearing stand-in for a Proc (both policies are link-agnostic).
struct FakeEnt {
    se: cfs::SchedEntity,
    link: rt::RtLink,
    id: u32,
}

impl FakeEnt {
    const fn new(id: u32) -> FakeEnt {
        FakeEnt {
            se: cfs::SchedEntity::new(),
            link: rt::RtLink::new(),
            id,
        }
    }
}

/// Container recovery from a member pointer — the same `offset_of!`
/// idiom sched.rs uses for `Proc`. Per-build offsets keep this correct
/// under any `#[repr(Rust)]` field reordering (a blind offset-0 cast
/// broke differently in debug vs release; see the s2 battery).
fn ent_of_se(se: *mut cfs::SchedEntity) -> *mut FakeEnt {
    ((se as usize) - core::mem::offset_of!(FakeEnt, se)) as *mut FakeEnt
}

/// `ent_of_se` for the RT link member.
fn ent_of_link(l: *mut rt::RtLink) -> *mut FakeEnt {
    ((l as usize) - core::mem::offset_of!(FakeEnt, link)) as *mut FakeEnt
}

/// m3.cfs.pick_order: vruntime order with FIFO tie-break.
fn case_cfs_pick_order() -> bool {
    let mut ents = [
        FakeEnt::new(0),
        FakeEnt::new(1),
        FakeEnt::new(2),
        FakeEnt::new(3),
    ];
    ents[0].se.vruntime = 30;
    ents[1].se.vruntime = 10;
    ents[2].se.vruntime = 20;
    ents[3].se.vruntime = 10;
    let mut rq = cfs::Runqueue::new();
    for e in ents.iter_mut() {
        cfs::enqueue_entity(&mut rq, &mut e.se);
    }
    // Pick order: vruntime 10 (insertion order 1 then 3), 20, 30.
    let want = [1u32, 3, 2, 0];
    for w in want {
        let se = cfs::pick_next_entity(&mut rq);
        let e = ent_of_se(se);
        // SAFETY: entity_of round-trips into our array elements.
        unsafe {
            if (*e).id != w {
                return false;
            }
        }
    }
    rq.is_empty()
}

/// m3.cfs.tick_fairness: two equal entities share the CPU within two
/// ticks (the xv6.ipc.preempt property at policy level).
fn case_cfs_tick_fairness() -> bool {
    const TICK_NS: u64 = 10_000_000;
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut rq = cfs::Runqueue::new();
    cfs::place_new_entity(&rq, &mut a.se);
    cfs::place_new_entity(&rq, &mut b.se);
    cfs::enqueue_entity(&mut rq, &mut b.se);
    let mut curr = &mut a.se as *mut cfs::SchedEntity;
    // SAFETY: both entities are ours; the loop only calls pure policy.
    unsafe {
        (*curr).exec_start = 0;
        let mut now = 0u64;
        for _ in 0..24 {
            now += TICK_NS;
            cfs::update_curr(&mut rq, &mut *curr, now);
            if cfs::preempt_needed(&rq, &*curr) {
                cfs::enqueue_entity(&mut rq, &mut *curr);
                curr = cfs::pick_next_entity(&mut rq);
                (*curr).exec_start = now;
            }
        }
    }
    let d = (a.se.sum_exec_ns as i64 - b.se.sum_exec_ns as i64).unsigned_abs();
    a.se.sum_exec_ns > 0 && b.se.sum_exec_ns > 0 && d <= 2 * TICK_NS
}

/// m3.rt.prio_order: highest non-empty level wins.
fn case_rt_prio_order() -> bool {
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut c = FakeEnt::new(2);
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links, not in any list.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 5);
        rt::push_tail(&mut rq, &mut b.link, 50);
        rt::push_tail(&mut rq, &mut c.link, 10);
    }
    let mut order = [0u32; 3];
    for slot in order.iter_mut() {
        // SAFETY: front/unlink on owned links.
        unsafe {
            let l = rt::front(&mut rq);
            if l.is_null() {
                return false;
            }
            let prio = rt::front_prio(&rq).unwrap();
            rt::unlink(&mut rq, l, prio);
            *slot = (*ent_of_link(l)).id;
        }
    }
    rq.is_empty() && order == [1, 2, 0]
}

/// m3.rt.fifo_head: FIFO within a level; a preempted FIFO requeues at
/// the head (keeps its turn).
fn case_rt_fifo_head() -> bool {
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut c = FakeEnt::new(2);
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 10);
        rt::push_tail(&mut rq, &mut b.link, 10);
        if rt::front_prio(&rq) != Some(10) {
            return false;
        }
        let front = rt::front(&mut rq);
        if (*ent_of_link(front)).id != 0 {
            return false; // FIFO arrival order
        }
        rt::push_head(&mut rq, &mut c.link, 10); // preempted FIFO returns
        let front = rt::front(&mut rq);
        (*ent_of_link(front)).id == 2
    }
}

/// m3.rt.rr_rotate: quantum rotation moves the front to the tail —
/// strict round-robin within a level.
fn case_rt_rr_rotate() -> bool {
    let mut ents = [FakeEnt::new(0), FakeEnt::new(1), FakeEnt::new(2)];
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links, same level.
    unsafe {
        for e in ents.iter_mut() {
            rt::push_tail(&mut rq, &mut e.link, 7);
        }
        // Three rotations (sched.rs RR-expiry path shape): front -> tail.
        for _ in 0..3 {
            let l = rt::front(&mut rq);
            rt::unlink(&mut rq, l, 7);
            rt::push_tail(&mut rq, l, 7);
        }
        // Full cycles later the order is back to 0, 1, 2.
        let mut seen = [0u32; 3];
        for slot in seen.iter_mut() {
            let l = rt::front(&mut rq);
            rt::unlink(&mut rq, l, 7);
            *slot = (*ent_of_link(l)).id;
        }
        seen == [0, 1, 2] && rq.is_empty()
    }
}

/// Link-bearing stand-in for a waiter (recovered via `offset_of!`,
/// not blind offset-0 casts — see the s2 battery note above).
struct FakeWqEnt {
    link: waitqueue::WaitLink,
    id: u32,
}

impl FakeWqEnt {
    const fn new(id: u32) -> FakeWqEnt {
        FakeWqEnt {
            link: waitqueue::WaitLink::new(),
            id,
        }
    }
}

fn ent_of_wqlink(l: *mut waitqueue::WaitLink) -> *mut FakeWqEnt {
    ((l as usize) - core::mem::offset_of!(FakeWqEnt, link)) as *mut FakeWqEnt
}

/// m3.wq.wake_policy: Linux wake discipline — non-exclusive entries
/// queue at the head, exclusive at the tail; wake-one takes every
/// non-exclusive waiter plus the first exclusive one; wake-all drains.
fn case_wq_wake_policy() -> bool {
    let mut a = FakeWqEnt::new(0);
    let mut b = FakeWqEnt::new(1);
    let mut c = FakeWqEnt::new(2);
    let mut d = FakeWqEnt::new(3);
    let mut q = waitqueue::WaitQueue::new();
    // SAFETY: owned links, not queued anywhere.
    unsafe {
        waitqueue::enqueue_link(&mut a.link, &mut q, true, false); // [a]
        waitqueue::enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        waitqueue::enqueue_link(&mut c.link, &mut q, true, true); // [b a c]
        waitqueue::enqueue_link(&mut d.link, &mut q, true, true); // [b a c d]
    }
    let mut set = [core::ptr::null_mut(); waitqueue::MAX_WAITERS];
    let n = waitqueue::select_wake(&q, true, &mut set);
    let mut ok = n == 3; // b a (non-exclusive) + c (first exclusive); d stays
    let want = [1u32, 0, 2];
    for (i, l) in set.iter().take(n).enumerate() {
        // SAFETY: links from our own ents.
        unsafe {
            ok = ok && (*ent_of_wqlink(*l)).id == want[i];
        }
    }
    let n_all = waitqueue::select_wake(&q, false, &mut set);
    ok = ok && n_all == 4;
    // SAFETY: owned links.
    unsafe {
        waitqueue::unlink(&mut a.link);
        waitqueue::unlink(&mut b.link);
        waitqueue::unlink(&mut c.link);
        waitqueue::unlink(&mut d.link);
    }
    ok && q.is_empty()
}

/// m3.wq.unlink_middle: unlinking a middle waiter keeps the list walk
/// exact, and a second unlink of the same link is a no-op.
fn case_wq_unlink_middle() -> bool {
    let mut a = FakeWqEnt::new(0);
    let mut b = FakeWqEnt::new(1);
    let mut c = FakeWqEnt::new(2);
    let mut q = waitqueue::WaitQueue::new();
    // SAFETY: owned links.
    unsafe {
        waitqueue::enqueue_link(&mut a.link, &mut q, true, false); // [a]
        waitqueue::enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        waitqueue::enqueue_link(&mut c.link, &mut q, true, false); // [c b a]
        waitqueue::unlink(&mut b.link); // [c a]
        waitqueue::unlink(&mut b.link); // idempotent
    }
    let mut set = [core::ptr::null_mut(); waitqueue::MAX_WAITERS];
    let n = waitqueue::select_wake(&q, false, &mut set);
    let mut ok = n == 2;
    let want = [2u32, 0];
    for (i, l) in set.iter().take(n).enumerate() {
        // SAFETY: links from our own ents.
        unsafe {
            ok = ok && (*ent_of_wqlink(*l)).id == want[i];
        }
    }
    // SAFETY: owned links.
    unsafe {
        waitqueue::unlink(&mut a.link);
        waitqueue::unlink(&mut c.link);
    }
    ok && q.is_empty()
}

/// m3.sched.policy_wiring: Linux-visible numbering and bounds the
/// syscalls and pick order rely on.
fn case_sched_policy_wiring() -> bool {
    let ok_policy = SchedPolicy::from_i64(0) == Some(SchedPolicy::Normal)
        && SchedPolicy::from_i64(1) == Some(SchedPolicy::Fifo)
        && SchedPolicy::from_i64(2) == Some(SchedPolicy::Rr)
        && SchedPolicy::from_i64(3).is_none()
        && SchedPolicy::from_i64(-1).is_none();
    let ok_bounds = rt::RT_PRIO_MIN == 1
        && rt::RT_PRIO_MAX == 99
        && rt::RT_RR_QUANTUM_TICKS == 10
        && (rt::RT_PRIO_MIN..=rt::RT_PRIO_MAX).contains(&50);
    // The RT runqueue empties exactly with its bitmap.
    let rq = rt::RtRunqueue::new();
    let ok_empty = rq.is_empty();
    let ok_gran = cfs::SCHED_LATENCY_NS == 20_000_000
        && cfs::MIN_GRANULARITY_NS == 4_000_000
        && cfs::NICE_0_LOAD == 1024;
    ok_policy && ok_bounds && ok_empty && ok_gran
}

// ---- time layer (21 §4): clocksource/clockevents wiring, hrtimer
// order/cancel, and a real wheel cascade driven by the live tick. ----

/// m3.time.clock_layer_wiring: rated clocksource switch (TSC 300 won at
/// calibration), monotonic progress, PIT event device in periodic mode.
fn case_time_clock_layer_wiring() -> bool {
    let dev = clockevents::active();
    let ok_dev = dev.name == "pit"
        && dev.hz == 100
        && dev.features & clockevents::CLOCK_EVT_FEAT_PERIODIC != 0
        && dev.rating == 100;
    let a = clocksource::monotonic_ns();
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    let b = clocksource::monotonic_ns();
    clocksource::active_rating() == clocksource::TSC_RATING
        && clocksource::TSC_RATING > clocksource::JIFFIES_RATING
        && a > 0
        && b > a
        && clocksource::wall_ns() >= b
        && ok_dev
}

/// Timers for the hrtimer order/cancel case (stable addresses: callbacks
/// receive raw pointers; accessed only through addr_of, never &mut).
static mut HR_T1: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut HR_T2: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut HR_T3: hrtimer::HrTimer = hrtimer::HrTimer::new();
/// Bitmask of fired timers (bit0 T1, bit1 T2, bit2 T3).
static HR_ORDER: AtomicUsize = AtomicUsize::new(0);

/// Record which timer fired into HR_ORDER.
fn hr_probe(t: *mut hrtimer::HrTimer) {
    let bit = if t as usize == core::ptr::addr_of!(HR_T1) as usize {
        1
    } else if t as usize == core::ptr::addr_of!(HR_T2) as usize {
        2
    } else {
        4
    };
    HR_ORDER.fetch_or(bit, Ordering::SeqCst);
}

/// m3.time.hrtimer_order_cancel: sorted insert regardless of insertion
/// order, batched expiry in expiry order, and cancel-wins-the-race
/// semantics (a cancelled timer's callback never runs).
fn case_time_hrtimer_order_cancel() -> bool {
    const BASE: u64 = 1_000_000;
    HR_ORDER.store(0, Ordering::SeqCst);
    // SAFETY: boot context; the timers are module statics that outlive
    // every armed window (fired or cancelled within this case).
    unsafe {
        let t1 = core::ptr::addr_of_mut!(HR_T1);
        let t2 = core::ptr::addr_of_mut!(HR_T2);
        let t3 = core::ptr::addr_of_mut!(HR_T3);
        // Deliberately out of expiry order: T1 latest, T2 earliest.
        hrtimer::hrtimer_start(t1, BASE + 30_000_000, hrtimer::HrMode::Abs, hr_probe);
        hrtimer::hrtimer_start(t2, BASE + 10_000_000, hrtimer::HrMode::Abs, hr_probe);
        hrtimer::hrtimer_start(t3, BASE + 20_000_000, hrtimer::HrMode::Abs, hr_probe);
        // Sorted insert: the head is the earliest expiry, not the last
        // started timer.
        if hrtimer::next_expiry_ns() != BASE + 10_000_000 {
            return false;
        }
        // Cancel before any expiry: the callback must never run.
        if !hrtimer::hrtimer_cancel(t3) {
            return false;
        }
        // First batch at T2's expiry: only T2 ran, T1 still pending.
        let n1 = hrtimer::run_expired(BASE + 15_000_000);
        if n1 != 1 || HR_ORDER.load(Ordering::SeqCst) != 2 {
            return false;
        }
        if hrtimer::pending() != 1 || hrtimer::next_expiry_ns() != BASE + 30_000_000 {
            return false;
        }
        // Second batch past T1's expiry: T3's bit stays clear (the
        // cancel won the race), T1 fired.
        let n2 = hrtimer::run_expired(BASE + 40_000_000);
        if n2 != 1 || HR_ORDER.load(Ordering::SeqCst) != 1 | 2 {
            return false;
        }
        // Cancel after fire loses the race; the fired flag proves the
        // callback ran while the cancelled timer never did.
        if !(!hrtimer::hrtimer_cancel(t1)
            && hrtimer::pending() == 0
            && (*t1).fired()
            && !(*t3).fired())
        {
            return false;
        }
        // Rel anchoring: a zero offset arms at the current monotonic
        // read, so the timer is already due by the next expire pass
        // (monotonic strictly increases between the two reads). T3's
        // order bit is still clear (its cancel won the race above), so
        // the re-fire completes the mask.
        hrtimer::hrtimer_start(t3, 0, hrtimer::HrMode::Rel, hr_probe);
        hrtimer::next_expiry_ns() != 0
            && hrtimer::run_expired(clocksource::monotonic_ns()) == 1
            && HR_ORDER.load(Ordering::SeqCst) == 1 | 2 | 4
            && (*t3).fired()
            && hrtimer::pending() == 0
    }
}

/// Jiffy at which the wheel probe ran (usize::MAX = never fired).
static WHEEL_FIRED_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Timer for the wheel cascade case (stable address for the callback;
/// accessed only through addr_of, never &mut).
static mut WHEEL_T: wheel::LrTimer = wheel::LrTimer::new();

/// Record the jiffy the wheel probe ran at.
fn wheel_probe(_t: *mut wheel::LrTimer) {
    WHEEL_FIRED_AT.store(crate::kernel::time::ticks() as usize, Ordering::SeqCst);
}

/// m3.time.wheel_cascade: a tv2-placed timer (t0+300) that only reaches
/// tv1 after the tv1 index wraps — the real cascade path, driven by the
/// live tick interrupt. Fires exactly on its expiry jiffy.
fn case_time_wheel_cascade() -> bool {
    let baseline = wheel::pending();
    let t0 = crate::kernel::time::ticks();
    let expires = t0 + 300; // level 1 by the wheel placement math
    WHEEL_FIRED_AT.store(usize::MAX, Ordering::SeqCst);
    let wt = core::ptr::addr_of_mut!(WHEEL_T);
    // SAFETY: boot context; WHEEL_T is a module static that outlives
    // the armed window (fired or dequeued before this case returns).
    unsafe {
        wheel::add_timer(wt, expires, wheel_probe);
    }
    // SAFETY: plain queued-flag read of the static timer.
    let armed = unsafe { (*wt).queued() };
    if !(wheel::pending() == baseline + 1 && armed) {
        // SAFETY: boot context; dequeue the un-fired timer.
        unsafe {
            wheel::del_timer(wt);
        }
        return false;
    }
    // Boot context runs with interrupts off; the tick interrupt drives
    // wheel_tick, so enable interrupts around the wait (suite.rs idiom)
    // and restore the boot state after.
    io::sti();
    while crate::kernel::time::ticks() < expires + 2
        && WHEEL_FIRED_AT.load(Ordering::SeqCst) == usize::MAX
    {
        io::hlt();
    }
    io::cli();
    // SAFETY: plain queued-flag read of the static timer.
    let ok = WHEEL_FIRED_AT.load(Ordering::SeqCst) == expires as usize
        && !unsafe { (*wt).queued() }
        && wheel::pending() == baseline;
    if !ok && armed {
        // SAFETY: boot context; dequeue the un-fired timer.
        unsafe {
            wheel::del_timer(wt);
        }
    }
    ok
}

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

fn case_kthread_run_exit() -> bool {
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

fn case_workq_dedup_run() -> bool {
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

fn case_softirq_order_bh_mask() -> bool {
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

fn case_softirq_tasklet_sched() -> bool {
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

fn case_softirq_ksoftirqd_pressure() -> bool {
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

pub fn run() -> ! {
    ksymtab_placeholder();
    // Resident bottom-half pressure valve for the softirq cases.
    softirq::spawn_ksoftirqd();
    let checks: [(&str, fn() -> bool); 16] = [
        ("m3.cfs.pick_order", case_cfs_pick_order),
        ("m3.cfs.tick_fairness", case_cfs_tick_fairness),
        ("m3.rt.prio_order", case_rt_prio_order),
        ("m3.rt.fifo_head", case_rt_fifo_head),
        ("m3.rt.rr_rotate", case_rt_rr_rotate),
        ("m3.sched.policy_wiring", case_sched_policy_wiring),
        ("m3.wq.wake_policy", case_wq_wake_policy),
        ("m3.wq.unlink_middle", case_wq_unlink_middle),
        ("m3.time.clock_layer_wiring", case_time_clock_layer_wiring),
        (
            "m3.time.hrtimer_order_cancel",
            case_time_hrtimer_order_cancel,
        ),
        ("m3.time.wheel_cascade", case_time_wheel_cascade),
        ("m3.kthread.run_exit", case_kthread_run_exit),
        ("m3.workq.dedup_run", case_workq_dedup_run),
        ("m3.softirq.order_bh_mask", case_softirq_order_bh_mask),
        ("m3.softirq.tasklet_sched", case_softirq_tasklet_sched),
        (
            "m3.softirq.ksoftirqd_pressure",
            case_softirq_ksoftirqd_pressure,
        ),
    ];
    let mut total = 0usize;
    let mut passed = 0usize;
    for (case_id, check) in checks {
        total += 1;
        let ok = check();
        console::write_str("CASE ");
        console::write_str(case_id);
        console::write_str(if ok { " PASS\n" } else { " FAIL\n" });
        if ok {
            passed += 1;
        }
    }
    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m3 run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m3 run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x10 } else { 0x11 });
    loop {
        io::hlt();
    }
}

/// m2suite names its symbol prefix via ksymtab; m3 cases are policy-only
/// (no injected faults), so just arm the suite name for the log.
fn ksymtab_placeholder() {
    super::ksymtab::set_active_suite("m3");
}

fn write_decimal(mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &buf[i..] {
        crate::arch::x86_64::console::write_byte(d);
    }
}
