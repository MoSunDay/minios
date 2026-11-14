//! sleep.rs: xv6 proc.c sleep/wakeup1/wakeup — atomically release a lock
//! and sleep on a channel, then reacquire.
//!
//! Two lock flavors are supported:
//!  * `sleep(chan, &RawLock)`     — the xv6-faithful port (ptable, pipe,
//!    ticks locks; manual acquire/release, needed for the scheduler's
//!    cross-context ptable handoff).
//!  * `sleep_guard(chan, lock, g)` — adapter for lib::sync::Spinlock users
//!    (Agent C's fs layer): drops the guard (releases the lock), sleeps,
//!    then re-locks `lock` and rewrites the guard slot in place. Agents
//!    that wrote `sleep(chan, &mut g)` should call this instead (see the
//!    contract block in crate::fs).

use core::ptr::addr_of_mut;
use core::ptr::eq as ptr_eq;

use crate::kernel::sched::{
    acquire, myproc, release, sched, ProcState, RawLock, PTABLE, PTABLE_LOCK,
};
use crate::lib::sync::{Spinlock, SpinlockGuard};

/// Atomically release lock and sleep on chan.
///
/// SAFETY: `lk` must be held by the caller (xv6 lock discipline); `chan` is
/// an address token (e.g. address of a pipe counter or of TICKS).
pub unsafe fn sleep(chan: usize, lk: &RawLock) {
    let proc = myproc();
    if proc.is_null() {
        panic!("sleep");
    }
    if !ptr_eq(lk, &PTABLE_LOCK) {
        //DOC: sleeplock0
        acquire(&PTABLE_LOCK);
        release(lk);
    }
    // SAFETY: ptable lock held; state transition is what scheduler sees.
    unsafe {
        // Go to sleep.
        (*proc).chan = chan;
        (*proc).state = ProcState::SLEEPING;
        sched();

        // Tidy up.
        (*proc).chan = 0;
    }
    // Reacquire original lock.
    if !ptr_eq(lk, &PTABLE_LOCK) {
        //DOC: sleeplock1
        release(&PTABLE_LOCK);
        acquire(lk);
    }
}

/// Spinlock-guard flavor: xv6 `sleep(chan, lk)` where lk is a
/// `lib::sync::Spinlock` currently held via `guard` (obtained from
/// `lock.lock()`). Releases it around the sleep and hands back a live
/// guard in the same slot.
///
/// SAFETY: `guard` must be a live guard for `lock`; after return the slot
/// holds a fresh guard for the same lock.
#[allow(dead_code)] // adapter for lib::sync::Spinlock sleepers; no caller yet
pub unsafe fn sleep_guard<'a, T>(
    chan: usize,
    lock: &'a Spinlock<T>,
    guard: &mut SpinlockGuard<'a, T>,
) {
    acquire(&PTABLE_LOCK);
    // SAFETY: guard owns the lock; dropping it in place is release(lk).
    // The slot is re-initialized below before any use.
    unsafe { core::ptr::drop_in_place(guard as *mut SpinlockGuard<'_, T>) };

    let proc = myproc();
    // SAFETY: ptable lock held.
    unsafe {
        (*proc).chan = chan;
        (*proc).state = ProcState::SLEEPING;
        sched();
        (*proc).chan = 0;
    }
    release(&PTABLE_LOCK);
    // SAFETY: re-initialize the dropped slot with a fresh guard; the
    // assignment move-initializes it (xv6 acquire(lk)).
    *guard = lock.lock();
}

/// Wake up all processes sleeping on chan. The ptable lock must be held.
///
/// SAFETY: caller must hold PTABLE_LOCK.
pub unsafe fn wakeup1(chan: usize) {
    for i in 0..crate::kernel::sched::NPROC {
        // SAFETY: ptable lock held by discipline.
        let p = unsafe { addr_of_mut!(PTABLE[i]) };
        unsafe {
            if (*p).state == ProcState::SLEEPING && (*p).chan == chan {
                (*p).state = ProcState::RUNNABLE;
            }
        }
    }
}

/// Wake up all processes sleeping on chan.
pub unsafe fn wakeup(chan: usize) {
    acquire(&PTABLE_LOCK);
    unsafe { wakeup1(chan) };
    release(&PTABLE_LOCK);
}
