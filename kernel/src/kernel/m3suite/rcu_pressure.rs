//! m3-suite RCU pressure case (21 §5): generation-swap pressure with
//! a parked reader — the probe-registry pointer swaps N generations
//! while the reader holds an old block across each swap (magic stays
//! valid; reclamation defers), and rcu_barrier drains every queued
//! free (kalloc/kfree balance, account zeroed).

use super::rcu::{block_of, drive_rcu, GenBlock, GEN_MAGIC};
use crate::kernel::rcu::{call_rcu, rcu_barrier, rcu_read_lock, rcu_read_unlock, stats};
use crate::kernel::rcu_engine::RcuHead;
use crate::proc::{kthread, sleep, waitqueue, Proc, ProcState, PTABLE_LOCK};
use crate::{rcu_assign, rcu_deref};
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Swap iterations (each: one held generation, one swap, one free).
const SWAPS: usize = 8;

static GEN_PTR: AtomicPtr<GenBlock> = AtomicPtr::new(ptr::null_mut());
static ALLOCED: AtomicUsize = AtomicUsize::new(0);
static FREED: AtomicUsize = AtomicUsize::new(0);
static READER_OK: AtomicUsize = AtomicUsize::new(0);
static READER_BAD: AtomicUsize = AtomicUsize::new(0);
static READER_GEN: AtomicUsize = AtomicUsize::new(0);
static READER_PHASE: AtomicUsize = AtomicUsize::new(0); // 1 in-section
static REVALIDATED: AtomicUsize = AtomicUsize::new(0);
static SWAP_STEP: AtomicUsize = AtomicUsize::new(0);
static SWAP_DONE: AtomicUsize = AtomicUsize::new(0);

static SWAP_LOCK: crate::klib::sync::Spinlock<()> = crate::klib::sync::Spinlock::new("rcuswap", ());
static mut SWAP_WAITQ: waitqueue::WaitQueue = waitqueue::WaitQueue::new();
static READER_PROC: AtomicPtr<Proc> = AtomicPtr::new(ptr::null_mut());

fn swap_free_cb(h: *mut RcuHead) {
    // SAFETY: h embeds in a GenBlock the case allocated and swapped
    // out (never freed by hand).
    let b = unsafe { block_of(h) };
    crate::mm::frame::kmfree(b as *mut u8);
    FREED.fetch_add(1, Ordering::SeqCst);
}

extern "C" fn swap_reader(_arg: usize) -> i32 {
    loop {
        rcu_read_lock();
        // SAFETY: GEN_PTR always points at a live-or-deferred block
        // (the writer only frees through swap_free_cb, a GP later).
        let b = rcu_deref!(GEN_PTR);
        let gen = unsafe { (*b).gen } as usize;
        if unsafe { (*b).magic } == GEN_MAGIC {
            READER_OK.fetch_add(1, Ordering::SeqCst);
        } else {
            READER_BAD.fetch_add(1, Ordering::SeqCst);
        }
        READER_GEN.store(gen, Ordering::SeqCst);
        READER_PHASE.store(1, Ordering::SeqCst);
        // Park inside the read section until the writer's swap (or the
        // done flag); still holding generation `gen`.
        while SWAP_STEP.load(Ordering::Acquire) <= gen && SWAP_DONE.load(Ordering::Acquire) == 0 {
            let g = SWAP_LOCK.lock();
            if SWAP_STEP.load(Ordering::Acquire) <= gen && SWAP_DONE.load(Ordering::Acquire) == 0 {
                // SAFETY: swap_reader proc context, guard held; static
                // mut accessed through the pointer form.
                unsafe { sleep::wq_sleep(ptr::addr_of_mut!(SWAP_WAITQ), &SWAP_LOCK, false, false) };
            }
            drop(g);
        }
        // Still inside the section: the held block survived the swap.
        if unsafe { (*b).magic } == GEN_MAGIC {
            READER_OK.fetch_add(1, Ordering::SeqCst);
        } else {
            READER_BAD.fetch_add(1, Ordering::SeqCst);
        }
        rcu_read_unlock();
        REVALIDATED.fetch_add(1, Ordering::SeqCst);
        if SWAP_DONE.load(Ordering::Acquire) != 0 {
            break;
        }
    }
    0
}

fn reader_holds_target() -> bool {
    READER_PHASE.load(Ordering::SeqCst) == 1
        && READER_GEN.load(Ordering::SeqCst) == TARGET.load(Ordering::SeqCst)
}

fn revalidated_target() -> bool {
    REVALIDATED.load(Ordering::SeqCst) == TARGET.load(Ordering::SeqCst) + 1
}

static TARGET: AtomicUsize = AtomicUsize::new(0);

static BARRIER_DONE: AtomicUsize = AtomicUsize::new(0);

extern "C" fn barrier_body(_arg: usize) -> i32 {
    rcu_barrier();
    BARRIER_DONE.store(1, Ordering::SeqCst);
    0
}

fn barrier_done() -> bool {
    BARRIER_DONE.load(Ordering::SeqCst) == 1
}

fn reader_revalidated_all() -> bool {
    REVALIDATED.load(Ordering::SeqCst) == SWAPS + 1
}

/// m3.rcu.gen_swap_pressure: N generation swaps under a parked reader
/// (each old block defers until the reader moves on, magic intact),
/// then rcu_barrier drains the final batch — kalloc/kfree balanced,
/// no validation ever failed.
pub(super) fn case_rcu_gen_swap_pressure() -> bool {
    ALLOCED.store(0, Ordering::SeqCst);
    FREED.store(0, Ordering::SeqCst);
    READER_OK.store(0, Ordering::SeqCst);
    READER_BAD.store(0, Ordering::SeqCst);
    READER_PHASE.store(0, Ordering::SeqCst);
    REVALIDATED.store(0, Ordering::SeqCst);
    SWAP_STEP.store(0, Ordering::SeqCst);
    SWAP_DONE.store(0, Ordering::SeqCst);
    TARGET.store(0, Ordering::SeqCst);
    BARRIER_DONE.store(0, Ordering::SeqCst);
    let (gp0, inv0) = stats();
    // Generation 0: the initial registry pointer.
    // SAFETY: boot context owns the allocation; reclamation rides the
    // queued callback (never freed by hand).
    unsafe {
        let b0 = crate::mm::frame::kmalloc(core::mem::size_of::<GenBlock>()) as *mut GenBlock;
        if b0.is_null() {
            return false;
        }
        (*b0).magic = GEN_MAGIC;
        (*b0).gen = 0;
        GEN_PTR.store(b0, Ordering::SeqCst);
        ALLOCED.fetch_add(1, Ordering::SeqCst);
        let p = kthread::kthread_create(swap_reader, 0, b"rcurd");
        if p.is_null() {
            return false;
        }
        READER_PROC.store(p, Ordering::SeqCst);
        kthread::kthread_run(p);
    }
    for i in 0..SWAPS {
        TARGET.store(i, Ordering::SeqCst);
        // The reader parks holding generation i.
        if !drive_rcu(reader_holds_target, 8) {
            return false;
        }
        // Deferral bound: at most the i retired generations have been
        // reclaimed — generation i itself is still reachable and held
        // (timer IRQs may have drained an earlier GP at irq_exit, so
        // the exact lag is not pinned; the held-block safety proof is
        // the reader's post-swap magic revalidation below).
        if FREED.load(Ordering::SeqCst) > i {
            return false;
        }
        // Swap: publish generation i+1, retire generation i.
        // SAFETY: as above; the old block stays readable until its
        // callback runs (a GP after the reader moves on).
        unsafe {
            let n = crate::mm::frame::kmalloc(core::mem::size_of::<GenBlock>()) as *mut GenBlock;
            if n.is_null() {
                return false;
            }
            (*n).magic = GEN_MAGIC;
            (*n).gen = (i + 1) as u64;
            ALLOCED.fetch_add(1, Ordering::SeqCst);
            let old = rcu_deref!(GEN_PTR);
            rcu_assign!(GEN_PTR, n);
            call_rcu(&mut (*old).head, swap_free_cb);
        }
        drain_and_bump();
        TARGET.store(i, Ordering::SeqCst);
        // The reader revalidates the held block, unlocks (releasing
        // the deferred GP), and parks on generation i+1.
        if !drive_rcu(revalidated_target, 8) {
            return false;
        }
    }
    // Release the reader from its final park (holding generation
    // SWAPS): one revalidation, then exit.
    SWAP_DONE.store(1, Ordering::SeqCst);
    TARGET.store(SWAPS - 1, Ordering::SeqCst);
    // SAFETY: static mut accessed through the pointer form.
    sleep::wake_up(unsafe { &mut *ptr::addr_of_mut!(SWAP_WAITQ) }, false);
    // SAFETY: drive borrows the scheduler swtch-back context.
    unsafe {
        let p = READER_PROC.load(Ordering::SeqCst);
        if !drive_rcu(reader_revalidated_all, 8) {
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
    let alloced = ALLOCED.load(Ordering::SeqCst);
    // Retire the final generation, then drain EVERYTHING through
    // rcu_barrier (its sleeper exercises the idle-QS liveness chain).
    // SAFETY: as above.
    unsafe {
        let last = rcu_deref!(GEN_PTR);
        call_rcu(&mut (*last).head, swap_free_cb);
        let bp = kthread::kthread_create(barrier_body, 0, b"rcubar");
        if bp.is_null() {
            return false;
        }
        kthread::kthread_run(bp);
        if !drive_rcu(barrier_done, 8) {
            return false;
        }
        if kthread::kthread_reap(bp) != 0 {
            return false;
        }
    }
    let (gp1, inv1) = stats();
    // Batch/GP count is scheduling-dependent (irq-exit drains and the
    // unlock-QS windows merge adjacent retires into one nxt batch —
    // the host-tested engine semantics); the deterministic anchors are
    // the callback account and the reader's validation record.
    gp1 >= gp0 + 2
        && inv1 == inv0 + SWAPS as u64 + 2
        && FREED.load(Ordering::SeqCst) == alloced
        && READER_BAD.load(Ordering::SeqCst) == 0
        && READER_OK.load(Ordering::SeqCst) >= 2 * SWAPS
        && !crate::kernel::rcu::pending_work()
}

/// One softirq drain, then hand the reader its wake (swap i done).
fn drain_and_bump() {
    super::rcu::drain_rcu();
    SWAP_STEP.fetch_add(1, Ordering::SeqCst);
    // SAFETY: static mut accessed through the pointer form.
    sleep::wake_up(unsafe { &mut *ptr::addr_of_mut!(SWAP_WAITQ) }, false);
}
