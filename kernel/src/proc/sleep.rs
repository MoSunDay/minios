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
            (*p).state = ProcState::RUNNABLE;
        }
    }
}

/// Timer tick channel: wake nanosleep-style sleepers waiting for `now`.
pub fn wakeup_ticks(now: u64) {
    wakeup(now as usize);
}
