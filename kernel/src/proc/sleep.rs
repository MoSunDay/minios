//! Sleep/wakeup channels (xv6 proc.c sleep/wakeup1/wakeup) plus the
//! timer-tick channel used by nanosleep.
//!
//! Lock protocol: sleep(chan, lk) with lk == PTABLE_LOCK keeps the lock
//! held across sched() (ownership transfers to the scheduler); with any
//! other lk the ptable lock is acquired and lk released around the sleep,
//! then handed back on wake.

use core::ptr::from_ref;

use super::proc::myproc;
use super::{ProcState, NPROC, PTABLE_LOCK};
use crate::klib::sync::Spinlock;

/// xv6 sleep(): release `lk` (if not the ptable lock), mark SLEEPING on
/// `chan`, swtch to the scheduler, then take `lk` back on wake.
///
/// # Safety
/// Caller holds `lk`; interrupts are off.
#[allow(dead_code)] // S2: wait()/pipe sleep on it
pub unsafe fn sleep(chan: usize, lk: &Spinlock<()>) {
    let p = myproc();
    if p.is_null() {
        panic!("sleep without process");
    }
    let same = from_ref(lk) == from_ref(&PTABLE_LOCK);
    if !same {
        // SAFETY: lk held; ptable lock free (not held by this context).
        unsafe { PTABLE_LOCK.raw_lock() };
        unsafe { lk.raw_release() };
    }
    // SAFETY: ptable lock held (ours or just acquired).
    unsafe {
        (*p).chan = chan;
        (*p).state = ProcState::SLEEPING;
        super::sched::sched_raw();
        (*p).chan = 0;
    }
    if !same {
        // SAFETY: ptable lock held (inherited through sched); lk free.
        unsafe { PTABLE_LOCK.raw_release() };
        unsafe { lk.raw_lock() };
    }
}

/// Wake every process sleeping on `chan` (acquires the ptable lock).
pub fn wakeup(chan: usize) {
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    unsafe { wakeup1(chan) };
}

/// xv6 wakeup1: caller holds the ptable lock.
///
/// # Safety
/// PTABLE_LOCK must be held by this context.
pub unsafe fn wakeup1(chan: usize) {
    // SAFETY: ptable lock held.
    let table = unsafe { super::ptable_mut() };
    for i in 0..NPROC {
        let p = &mut table[i] as *mut super::Proc;
        if (*p).state == ProcState::SLEEPING && (*p).chan == chan {
            // SAFETY: ptable lock held; p sleeps on this channel.
            super::sched::runnable_wake(p);
        }
    }
}

/// Timer tick channel: wake nanosleep-style sleepers waiting for `now`.
pub fn wakeup_ticks(now: u64) {
    wakeup(now as usize);
}

// ---------------------------------------------------------------------------
// Wait-queue sleep glue (21 §3): kernel-context halves of waitqueue.rs
// (that module stays pure for host tests).

use super::waitqueue::{enqueue_link, select_wake, unlink, WaitQueue};
use super::Proc;

/// Enqueue the current proc on `q` and sleep (xv6 sleep() lock protocol:
/// `lk` is released across the sleep and re-acquired after; PTABLE_LOCK
/// itself is kept across the swtch).
///
/// Interruptible waits break on kill (ERESTARTSYS at the caller via
/// `interrupted()`); exclusive selects the wake discipline (see module
/// docs).
///
/// # Safety
/// Caller holds `lk`; interrupts are off; the current proc's wait_link
/// is not queued anywhere.
pub unsafe fn wq_sleep(
    wq: *mut WaitQueue,
    lk: &Spinlock<()>,
    interruptible: bool,
    exclusive: bool,
) {
    let p = myproc();
    if p.is_null() {
        panic!("wq_sleep without process");
    }
    let same = from_ref(lk) == from_ref(&PTABLE_LOCK);
    if !same {
        // SAFETY: lk held; ptable lock free (not held by this context).
        unsafe { PTABLE_LOCK.raw_lock() };
        unsafe { lk.raw_release() };
    }
    // SAFETY: ptable lock held (ours or just acquired).
    unsafe {
        enqueue_link(&mut (*p).wait_link, &mut *wq, interruptible, exclusive);
        (*p).chan = (*wq).chan();
        (*p).state = super::ProcState::SLEEPING;
        super::sched::sched_raw();
        // Wakers unlink the links they wake; a kill-wake leaves the
        // link queued — self-dequeue is idempotent.
        unlink(&mut (*p).wait_link);
        (*p).chan = 0;
    }
    if !same {
        // SAFETY: ptable lock held (inherited through sched); lk free.
        unsafe { PTABLE_LOCK.raw_release() };
        unsafe { lk.raw_lock() };
    }
}

/// Wake waiters (acquires PTABLE_LOCK). `one` selects the exclusive
/// discipline (see `select_wake`).
pub fn wake_up(wq: &mut WaitQueue, one: bool) {
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    unsafe { wake_up_locked(wq, one) };
}

/// # Safety
/// PTABLE_LOCK must be held by this context.
pub unsafe fn wake_up_locked(wq: &mut WaitQueue, one: bool) {
    let mut set = [core::ptr::null_mut(); super::waitqueue::MAX_WAITERS];
    let n = select_wake(wq, one, &mut set);
    let off = core::mem::offset_of!(Proc, wait_link);
    for l in set.iter().take(n) {
        // SAFETY: ptable lock held; the link is queued in this context.
        unsafe {
            let p = ((*l as usize) - off) as *mut Proc;
            unlink(&mut **l);
            if (*p).state == super::ProcState::SLEEPING {
                super::sched::runnable_wake(p);
            }
        }
    }
}

/// Interrupt pending for the current proc: fatal kill today, plus
/// catchable (unblocked) signal delivery from s6 on.
pub fn interrupted() -> bool {
    let p = myproc();
    !p.is_null() && unsafe { (*p).killed != 0 }
}
