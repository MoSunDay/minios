//! m3-suite RCU cases (21 §5): grace-period basics (call_rcu ->
//! quiescent swtch -> invocation, synchronize_rcu ordering),
//! reader-deferred reclamation across a voluntary park (generation
//! magic survives, the QS at unlock releases the grace period), and
//! generation-swap pressure with a parked reader (kalloc/kfree
//! balance, rcu_barrier zeroing the account). The consumer is the
//! in-test probe registry — an RCU-replaced pointer to kalloc'd
//! generation blocks (the dcache-class consumers land in M4+).

use crate::kernel::rcu::{
    active_readers, call_rcu, rcu_read_lock, rcu_read_unlock, stats, synchronize_rcu, RcuHead,
};
use crate::kernel::softirq;
use crate::proc::{kthread, sched, sleep, waitqueue, Proc, ProcState, PTABLE_LOCK};
use core::mem::offset_of;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Deterministic drain: raise + run RCU_SOFTIRQ inline (the same body
/// irq_exit / ksoftirqd run; boot context, interrupts off).
pub(super) fn drain_rcu() {
    softirq::raise_softirq(softirq::RCU_SOFTIRQ);
    softirq::do_softirq();
}

/// The system interleave, played by hand: run runnable contexts, then
/// drain RCU_SOFTIRQ, until `stop` holds (or the round budget dies).
/// Each round's pick-null pass reports the scheduler idle QS — the
/// liveness hook that keeps a fully parked system advancing.
pub(super) fn drive_rcu(stop: fn() -> bool, rounds: usize) -> bool {
    for _ in 0..rounds {
        if sched::sched_drive(stop, 64) {
            return true;
        }
        drain_rcu();
    }
    false
}

// ---- m3.rcu.basic_grace -------------------------------------------------

static CB_SLOT: [AtomicUsize; 4] = [const { AtomicUsize::new(usize::MAX) }; 4];
static mut CB_HEADS: [RcuHead; 4] = [
    RcuHead::new(seq_cb),
    RcuHead::new(seq_cb),
    RcuHead::new(seq_cb),
    RcuHead::new(seq_cb),
];
static SEQ: AtomicUsize = AtomicUsize::new(0);

fn seq_cb(h: *mut RcuHead) {
    // SAFETY: h is one of the module-static CB_HEADS.
    let base = ptr::addr_of_mut!(CB_HEADS) as usize;
    let idx = (h as usize - base) / core::mem::size_of::<RcuHead>();
    CB_SLOT[idx].store(SEQ.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
}

static QS_DONE: AtomicUsize = AtomicUsize::new(0);

extern "C" fn qs_body(_arg: usize) -> i32 {
    QS_DONE.store(1, Ordering::SeqCst);
    0
}

fn qs_done() -> bool {
    QS_DONE.load(Ordering::SeqCst) == 1
}

static SYNC_ENTERED_GP: AtomicUsize = AtomicUsize::new(usize::MAX);
static SYNC_DONE_GP: AtomicUsize = AtomicUsize::new(usize::MAX);

extern "C" fn sync_body(_arg: usize) -> i32 {
    SYNC_ENTERED_GP.store(stats().0 as usize, Ordering::SeqCst);
    synchronize_rcu();
    SYNC_DONE_GP.store(stats().0 as usize, Ordering::SeqCst);
    0
}

fn sync_entered() -> bool {
    SYNC_ENTERED_GP.load(Ordering::SeqCst) != usize::MAX
}

fn sync_done() -> bool {
    SYNC_DONE_GP.load(Ordering::SeqCst) != usize::MAX
}

/// m3.rcu.basic_grace: a callback batch opens a GP at the softirq
/// drain, a real quiescent swtch (kthread exit through sched_drive)
/// completes it FIFO, and synchronize_rcu returns strictly after
/// another full grace period.
pub(super) fn case_rcu_basic_grace() -> bool {
    for s in CB_SLOT.iter() {
        s.store(usize::MAX, Ordering::SeqCst);
    }
    SEQ.store(0, Ordering::SeqCst);
    SYNC_ENTERED_GP.store(usize::MAX, Ordering::SeqCst);
    SYNC_DONE_GP.store(usize::MAX, Ordering::SeqCst);
    let (gp0, inv0) = stats();
    // SAFETY: boot context owns the static heads until invocation;
    // pointer forms throughout (no static-mut references).
    unsafe {
        let heads = ptr::addr_of_mut!(CB_HEADS);
        for i in 0..4 {
            call_rcu(ptr::addr_of_mut!((*heads)[i]), seq_cb);
        }
    }
    if !crate::kernel::rcu::pending_work() {
        return false;
    }
    // Drain 1 (no quiescent state yet): the GP opens, nothing runs.
    drain_rcu();
    if stats() != (gp0, inv0) {
        return false;
    }
    // A real QS: a kthread exit swtch through sched_drive.
    // SAFETY: boot context owns the fresh slot; drive borrows the
    // scheduler swtch-back context.
    unsafe {
        let p = kthread::kthread_create(qs_body, 0, b"rcuqs");
        if p.is_null() {
            return false;
        }
        kthread::kthread_run(p);
        if !drive_rcu(qs_done, 4) {
            return false;
        }
        {
            let _guard = PTABLE_LOCK.lock();
            if (*p).state != ProcState::ZOMBIE {
                return false;
            }
        }
        if kthread::kthread_reap(p) != 0 {
            return false;
        }
    }
    drain_rcu();
    let (gp1, inv1) = stats();
    if gp1 != gp0 + 1 || inv1 != inv0 + 4 {
        return false;
    }
    // FIFO within the batch.
    for (i, s) in CB_SLOT.iter().enumerate() {
        if s.load(Ordering::SeqCst) != i {
            return false;
        }
    }
    // synchronize_rcu: returns strictly after another full grace
    // period (its parking swtch cannot ride its own marker's GP; the
    // idle QS + drain rounds complete it).
    // SAFETY: as above.
    unsafe {
        let sp = kthread::kthread_create(sync_body, 0, b"rcusync");
        if sp.is_null() {
            return false;
        }
        kthread::kthread_run(sp);
        if !drive_rcu(sync_entered, 4) {
            return false;
        }
        if !drive_rcu(sync_done, 8) {
            return false;
        }
        if kthread::kthread_reap(sp) != 0 {
            return false;
        }
    }
    let entered = SYNC_ENTERED_GP.load(Ordering::SeqCst);
    let done = SYNC_DONE_GP.load(Ordering::SeqCst);
    entered < done
        && done >= gp1 as usize + 1
        && !crate::kernel::rcu::pending_work()
        && active_readers() == 0
}

// ---- m3.rcu.reader_defer ------------------------------------------------

pub(super) const GEN_MAGIC: u64 = 0x5243_5547_454e_3031; // "RCUGEN01"

/// A kalloc'd generation block: the probe-registry consumer object
/// whose reclamation rides call_rcu.
#[repr(C)]
pub(super) struct GenBlock {
    pub(super) magic: u64,
    pub(super) gen: u64,
    pub(super) head: RcuHead,
}

/// Container-of through the embedded head (the free callback runs a
/// grace period after its block left GEN_PTR).
pub(super) unsafe fn block_of(h: *mut RcuHead) -> *mut GenBlock {
    (h as usize - offset_of!(GenBlock, head)) as *mut GenBlock
}

static DEFER_LOCK: crate::klib::sync::Spinlock<()> =
    crate::klib::sync::Spinlock::new("rcudefer", ());
static mut DEFER_WAITQ: waitqueue::WaitQueue = waitqueue::WaitQueue::new();
static DEFER_STATE: AtomicUsize = AtomicUsize::new(0); // 1 in-section, 2 out
static DEFER_PROC: AtomicPtr<Proc> = AtomicPtr::new(ptr::null_mut());
static DEFER_RELEASE: AtomicUsize = AtomicUsize::new(0);
static DEFER_FREED: AtomicUsize = AtomicUsize::new(0);
static DEFER_MAGIC_OK: AtomicUsize = AtomicUsize::new(0);
static DEFER_BLOCK: AtomicPtr<GenBlock> = AtomicPtr::new(ptr::null_mut());

fn defer_free_cb(h: *mut RcuHead) {
    // SAFETY: h embeds in the DEFER_BLOCK GenBlock queued by the case.
    let b = unsafe { block_of(h) };
    crate::mm::frame::kmfree(b as *mut u8);
    DEFER_FREED.fetch_add(1, Ordering::SeqCst);
}

extern "C" fn defer_body(_arg: usize) -> i32 {
    rcu_read_lock();
    // SAFETY: the case queued exactly this block and never frees it
    // by hand (reclamation rides our own callback).
    let b = DEFER_BLOCK.load(Ordering::SeqCst);
    let ok = unsafe { (*b).magic } == GEN_MAGIC;
    DEFER_STATE.store(1, Ordering::SeqCst);
    while DEFER_RELEASE.load(Ordering::Acquire) == 0 {
        let g = DEFER_LOCK.lock();
        if DEFER_RELEASE.load(Ordering::Acquire) == 0 {
            // SAFETY: defer_body proc context, guard held; static mut
            // accessed through the pointer form.
            unsafe { sleep::wq_sleep(ptr::addr_of_mut!(DEFER_WAITQ), &DEFER_LOCK, false, false) };
        }
        drop(g);
    }
    // Still inside the read section: the block must have survived the
    // whole park (deferred reclamation — an early free corrupts magic).
    if ok && unsafe { (*b).magic } == GEN_MAGIC {
        DEFER_MAGIC_OK.store(1, Ordering::SeqCst);
    }
    rcu_read_unlock(); // the deferred QS releases the grace period
    DEFER_STATE.store(2, Ordering::SeqCst);
    0
}

fn defer_in() -> bool {
    DEFER_STATE.load(Ordering::SeqCst) == 1
}

fn defer_out() -> bool {
    DEFER_STATE.load(Ordering::SeqCst) == 2
}

/// m3.rcu.reader_defer: a reader parked mid-section suppresses the
/// swtch QS (the GP stalls with the callback queued), its block stays
/// valid across the park, and the QS observed at unlock completes the
/// deferred grace period.
pub(super) fn case_rcu_reader_defer() -> bool {
    DEFER_STATE.store(0, Ordering::SeqCst);
    DEFER_RELEASE.store(0, Ordering::SeqCst);
    DEFER_FREED.store(0, Ordering::SeqCst);
    DEFER_MAGIC_OK.store(0, Ordering::SeqCst);
    let (gp0, inv0) = stats();
    // SAFETY: boot context owns the allocation; reclamation rides the
    // queued callback (never freed by hand).
    unsafe {
        let b = crate::mm::frame::kmalloc(core::mem::size_of::<GenBlock>()) as *mut GenBlock;
        if b.is_null() {
            return false;
        }
        (*b).magic = GEN_MAGIC;
        (*b).gen = 0;
        DEFER_BLOCK.store(b, Ordering::SeqCst);
        call_rcu(ptr::addr_of_mut!((*b).head), defer_free_cb);
    }
    // The GP opens before the reader parks (its swtch QS is then
    // suppressed — the kernel-side deferral path).
    drain_rcu();
    // SAFETY: boot context owns the fresh slot; drive borrows the
    // scheduler swtch-back context.
    unsafe {
        let p = kthread::kthread_create(defer_body, 0, b"rcudfr");
        if p.is_null() {
            return false;
        }
        DEFER_PROC.store(p, Ordering::SeqCst);
        kthread::kthread_run(p);
        if !drive_rcu(defer_in, 4) {
            return false;
        }
    }
    // Drain with the reader parked: suppressed QS, nothing completes.
    drain_rcu();
    if stats() != (gp0, inv0) || DEFER_FREED.load(Ordering::SeqCst) != 0 || active_readers() != 1 {
        return false;
    }
    DEFER_RELEASE.store(1, Ordering::SeqCst);
    // SAFETY: static mut accessed through the pointer form.
    sleep::wake_up(unsafe { &mut *ptr::addr_of_mut!(DEFER_WAITQ) }, false);
    // SAFETY: drive borrows the scheduler swtch-back context.
    unsafe {
        let p = DEFER_PROC.load(Ordering::SeqCst);
        if !drive_rcu(defer_out, 8) {
            return false;
        }
        {
            let _guard = PTABLE_LOCK.lock();
            if (*p).state != ProcState::ZOMBIE {
                return false;
            }
        }
        if kthread::kthread_reap(p) != 0 {
            return false;
        }
    }
    drain_rcu(); // the unlock's deferred QS completes and frees
    stats() == (gp0 + 1, inv0 + 1)
        && DEFER_FREED.load(Ordering::SeqCst) == 1
        && DEFER_MAGIC_OK.load(Ordering::SeqCst) == 1
        && active_readers() == 0
        && !crate::kernel::rcu::pending_work()
}
