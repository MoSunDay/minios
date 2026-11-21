//! Softirqs (21 §3, UP): 2.6-style bottom halves — a small vector of
//! statically registered handlers plus a pending bitmask. Hardirq
//! paths only raise; traps_dispatch runs pending work at IRQ exit
//! (irq_exit) with a bounded restart budget, and budget-exhausted work
//! is handed to the ksoftirqd kthread (spawned on demand), which
//! drains pending bits in process context. Tasklets ride the
//! TASKLET/HI softirqs with per-tasklet SCHED de-duplication.
//! Divergences (ADR-0020): do_softirq keeps IRQs masked (Linux re-enables
//! local IRQs while processing), raise_softirq never wakes ksoftirqd
//! directly from hardirq context (only budget exhaustion or an explicit
//! process-context raise does), and disabled tasklets are dropped.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::klib::sync::Spinlock;
use crate::proc::preempt::{in_softirq, local_bh_disable, local_bh_enable};
use crate::proc::{kthread, sleep, waitqueue, Proc, ProcState};

/// Softirq indices (include/linux/interrupt.h order). The table is
/// compacted (no BLOCK/SCHED/HRTIRQ slots — 2.6.39.4 numbers TASKLET
/// 6 and RCU 9); RCU rides slot 6 right behind TASKLET (m3r-s7) while
/// TX/RX keep their Linux numbers.
pub const HI_SOFTIRQ: usize = 0;
pub const TIMER_SOFTIRQ: usize = 1;
pub const NET_TX_SOFTIRQ: usize = 2;
pub const NET_RX_SOFTIRQ: usize = 3;
pub const TASKLET_SOFTIRQ: usize = 5;
pub const RCU_SOFTIRQ: usize = 6;
pub const NR_SOFTIRQS: usize = 7;

/// Max do_softirq restart loops before deferring to ksoftirqd.
pub const MAX_SOFTIRQ_RESTART: usize = 10;

pub type SoftirqHandler = fn(usize);

static mut SOFTIRQ_VEC: [Option<SoftirqHandler>; NR_SOFTIRQS] = [None; NR_SOFTIRQS];
static mut SOFTIRQ_DATA: [usize; NR_SOFTIRQS] = [0; NR_SOFTIRQS];
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Register a handler (Linux open_softirq; one handler per index).
///
/// # Safety
/// Boot or quiet context; never re-registered while pending.
pub unsafe fn open_softirq(nr: usize, handler: SoftirqHandler, data: usize) {
    assert!(nr < NR_SOFTIRQS, "open_softirq: bad index");
    unsafe {
        *core::ptr::addr_of_mut!(SOFTIRQ_VEC[nr]) = Some(handler);
        *core::ptr::addr_of_mut!(SOFTIRQ_DATA[nr]) = data;
    }
}

/// Pending bitmask snapshot.
pub fn pending() -> usize {
    PENDING.load(Ordering::SeqCst)
}

/// Mark a softirq pending (hardirq-safe: a plain atomic or).
pub fn raise_softirq(nr: usize) {
    assert!(nr < NR_SOFTIRQS, "raise_softirq: bad index");
    PENDING.fetch_or(1 << nr, Ordering::SeqCst);
}

/// Run pending softirqs (IRQ-exit and ksoftirqd entry). No-op when
/// idle or already servicing (in_softirq guard). Handlers run in
/// ascending index order with BH depth held; a self-raising storm
/// burns the restart budget and the leftover mask wakes ksoftirqd.
pub fn do_softirq() {
    if pending() == 0 || in_softirq() {
        return;
    }
    local_bh_disable();
    let mut restarts = 0usize;
    while pending() != 0 {
        if restarts >= MAX_SOFTIRQ_RESTART {
            break;
        }
        let work = PENDING.swap(0, Ordering::SeqCst);
        // SAFETY: bh-disabled, IF-masked (irq exit) or process context.
        unsafe { run_mask(work) };
        restarts += 1;
    }
    if pending() != 0 {
        wakeup_ksoftirqd();
    }
    local_bh_enable();
}

/// Dispatch one pending snapshot in ascending index order.
///
/// # Safety
/// Called with bh disabled (do_softirq) or from ksoftirqd's drain loop.
unsafe fn run_mask(work: usize) {
    let vec = core::ptr::addr_of!(SOFTIRQ_VEC) as *const [Option<SoftirqHandler>; NR_SOFTIRQS];
    let data = core::ptr::addr_of!(SOFTIRQ_DATA) as *const [usize; NR_SOFTIRQS];
    let mut bit = 0usize;
    while work >> bit != 0 {
        if work & (1 << bit) != 0 {
            if let Some(h) = unsafe { (*vec)[bit] } {
                h(unsafe { (*data)[bit] });
            }
        }
        bit += 1;
    }
}

/// IRQ-exit hook (traps_dispatch, vector >= IRQ_BASE).
pub fn irq_exit() {
    do_softirq();
}

// ---- tasklets (simplified tasklet_struct) ----

pub const TASKLET_STATE_SCHED: usize = 1;

/// A deferrable function scheduled onto the TASKLET/HI softirqs.
pub struct TaskletStruct {
    pub state: AtomicUsize,
    /// Disable count: a tasklet runs only at count 0 (simplified:
    /// disabled tasklets are dropped, not re-queued — ADR-0020).
    pub count: AtomicUsize,
    pub func: fn(usize),
    pub data: usize,
    next: *mut TaskletStruct,
}

impl TaskletStruct {
    pub const fn new(func: fn(usize), data: usize) -> Self {
        TaskletStruct {
            state: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            func,
            data,
            next: core::ptr::null_mut(),
        }
    }
}

static TASKLET_LOCK: Spinlock<()> = Spinlock::new("tasklet", ());
/// Lists indexed by softirq: [TASKLET_SOFTIRQ, HI_SOFTIRQ].
static mut TASKLET_LISTS: [*mut TaskletStruct; 2] = [core::ptr::null_mut(), core::ptr::null_mut()];

/// Schedule a tasklet onto the normal list (false if already SCHED'd).
///
/// # Safety
/// `t` must outlive the run (static or owned until func fires).
pub unsafe fn schedule_tasklet(t: *mut TaskletStruct) -> bool {
    unsafe { tasklet_schedule(t, 0) }
}

/// Schedule a tasklet onto the high-priority (HI_SOFTIRQ) list.
///
/// # Safety
/// Same contract as schedule_tasklet.
pub unsafe fn schedule_tasklet_hi(t: *mut TaskletStruct) -> bool {
    unsafe { tasklet_schedule(t, 1) }
}

unsafe fn tasklet_schedule(t: *mut TaskletStruct, list: usize) -> bool {
    // Test-and-set SCHED outside the lock (single enqueue site).
    let prev = (*t).state.fetch_or(TASKLET_STATE_SCHED, Ordering::SeqCst);
    if prev & TASKLET_STATE_SCHED != 0 {
        return false;
    }
    (*t).next = core::ptr::null_mut();
    let _guard = TASKLET_LOCK.lock();
    // SAFETY: tasklet lock held; static list access.
    unsafe {
        let lists = core::ptr::addr_of_mut!(TASKLET_LISTS) as *mut [*mut TaskletStruct; 2];
        let head = (*lists)[list];
        if head.is_null() {
            (*lists)[list] = t;
        } else {
            let mut cur = head;
            while !(*cur).next.is_null() {
                cur = (*cur).next;
            }
            (*cur).next = t;
        }
    }
    let nr = if list == 0 {
        TASKLET_SOFTIRQ
    } else {
        HI_SOFTIRQ
    };
    raise_softirq(nr);
    true
}

/// TASKLET/HI softirq body: steal the whole list, run each runnable
/// tasklet once (SCHED cleared first — self-reschedule re-queues).
fn tasklet_action(list: usize) {
    let stolen;
    {
        let _guard = TASKLET_LOCK.lock();
        // SAFETY: tasklet lock held.
        unsafe {
            let lists = core::ptr::addr_of_mut!(TASKLET_LISTS) as *mut [*mut TaskletStruct; 2];
            stolen = (*lists)[list];
            (*lists)[list] = core::ptr::null_mut();
        }
    }
    let mut cur = stolen;
    while !cur.is_null() {
        // SAFETY: dequeued tasklet owned by this run.
        unsafe {
            let t = cur;
            cur = (*t).next;
            if (*t).count.load(Ordering::SeqCst) == 0 {
                (*t).state.fetch_and(!TASKLET_STATE_SCHED, Ordering::SeqCst);
                ((*t).func)((*t).data);
            }
        }
    }
}

// ---- ksoftirqd ----

static KSOFTIRQD_LOCK: Spinlock<()> = Spinlock::new("ksoftirqd", ());
static mut KSOFTIRQD_WAITQ: waitqueue::WaitQueue = waitqueue::WaitQueue::new();
static mut KSOFTIRQD_PROC: *mut Proc = core::ptr::null_mut();

extern "C" fn ksoftirqd_main(_arg: usize) -> i32 {
    loop {
        while pending() != 0 {
            do_softirq();
        }
        if sleep::interrupted() {
            return 0;
        }
        // The lock only satisfies the wq_sleep protocol (condition is
        // the atomic pending mask; wakers never take this lock).
        let _guard = KSOFTIRQD_LOCK.lock();
        if pending() == 0 {
            // SAFETY: ksoftirqd proc context, guard held; static mut
            // accessed through the pointer form.
            unsafe {
                let wq = core::ptr::addr_of_mut!(KSOFTIRQD_WAITQ);
                sleep::wq_sleep(wq, &KSOFTIRQD_LOCK, false, true)
            };
        }
    }
}

/// Wake the ksoftirqd kthread (budget exhaustion / process-context
/// raise). Safe from hardirq context: a plain ptable-locked wake.
fn wakeup_ksoftirqd() {
    // SAFETY: static access without a reference.
    let p = unsafe { *core::ptr::addr_of!(KSOFTIRQD_PROC) };
    if p.is_null() {
        return;
    }
    let _guard = crate::proc::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    unsafe {
        if (*p).state == ProcState::SLEEPING {
            crate::proc::sched::runnable_wake(p);
        }
    }
}

/// Spawn the ksoftirqd kthread (boot/process context; idempotent).
pub fn spawn_ksoftirqd() -> bool {
    // SAFETY: static access without a reference.
    if !unsafe { *core::ptr::addr_of!(KSOFTIRQD_PROC) }.is_null() {
        return true;
    }
    // SAFETY: quiet context, slot owned until kthread_run.
    let p = unsafe { kthread::kthread_create(ksoftirqd_main, 0, b"ksoftirqd") };
    if p.is_null() {
        return false;
    }
    // SAFETY: EMBRYO from kthread_create.
    unsafe { kthread::kthread_run(p) };
    // SAFETY: static access without a reference.
    unsafe { *core::ptr::addr_of_mut!(KSOFTIRQD_PROC) = p };
    true
}

/// The ksoftirqd Proc (m3-suite state probes); null before spawn.
pub fn ksoftirqd_proc() -> *mut Proc {
    // SAFETY: static access without a reference.
    unsafe { *core::ptr::addr_of!(KSOFTIRQD_PROC) }
}

/// ksoftirqd's current state under the ptable lock (drive-loop probe).
pub fn ksoftirqd_state() -> ProcState {
    // SAFETY: static access without a reference.
    let p = unsafe { *core::ptr::addr_of!(KSOFTIRQD_PROC) };
    if p.is_null() {
        return ProcState::UNUSED;
    }
    let _guard = crate::proc::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    unsafe { (*p).state }
}

/// Register the built-in tasklet softirqs (m1r_boot common init).
pub fn init() {
    // SAFETY: boot init, nothing pending.
    unsafe {
        open_softirq(TASKLET_SOFTIRQ, tasklet_softirq, 0);
        open_softirq(HI_SOFTIRQ, tasklet_softirq, 1);
    }
}

fn tasklet_softirq(list: usize) {
    tasklet_action(list);
}
