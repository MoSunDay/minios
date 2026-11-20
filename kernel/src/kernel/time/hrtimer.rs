//! Hrtimers (21 §4): absolute/relative expiry over the monotonic
//! clocksource with Linux-style cancel semantics. UP shape: an
//! intrusive list kept sorted by absolute expiry (Linux uses an rbtree;
//! the list is exact for our population and host-testable). Expiry
//! runs from the periodic tick — sub-tick resolution waits for the M8
//! LAPIC one-shot (ADR-0019).

use crate::klib::sync::Spinlock;

/// Expiry basis.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HrMode {
    Abs,
    Rel,
}

/// Timer callback receives the timer pointer.
pub type HrCallback = fn(*mut HrTimer);

/// One hrtimer. Storage is owned by the user (kernel stack for
/// nanosleep-style waits, statics/embedded structs for permanent ones);
/// the queue owns the link while armed.
pub struct HrTimer {
    expires_ns: u64,
    next: *mut HrTimer,
    prev: *mut HrTimer,
    armed: bool,
    fired: bool,
    callback: Option<HrCallback>,
}

impl HrTimer {
    pub const fn new() -> HrTimer {
        HrTimer {
            expires_ns: 0,
            next: core::ptr::null_mut(),
            prev: core::ptr::null_mut(),
            armed: false,
            fired: false,
            callback: None,
        }
    }

    pub fn fired(&self) -> bool {
        self.fired
    }
}

/// Sorted queue state (all fields under HRTIMER_LOCK).
struct HrQueue {
    first: *mut HrTimer,
    count: usize,
}

static HRTIMER_LOCK: Spinlock<()> = Spinlock::new("hrtimer", ());
static mut QUEUE: HrQueue = HrQueue {
    first: core::ptr::null_mut(),
    count: 0,
};

/// Arm (or re-arm) `t` to expire at `ns` (mode-relative to the
/// monotonic clock). Re-arming an armed timer repositions it.
///
/// # Safety
/// `t` must outlive its armed period (cancel before dropping).
pub unsafe fn hrtimer_start(t: *mut HrTimer, ns: u64, mode: HrMode, cb: HrCallback) {
    let expires = match mode {
        HrMode::Abs => ns,
        HrMode::Rel => super::clocksource::monotonic_ns().saturating_add(ns),
    };
    let _guard = HRTIMER_LOCK.lock();
    // SAFETY: ptable-free queue; we exclusively own `t`.
    unsafe {
        let q = core::ptr::addr_of_mut!(QUEUE);
        hrtimer_unlink_locked(&mut *q, t);
        (*t).expires_ns = expires;
        (*t).fired = false;
        (*t).callback = Some(cb);
        (*t).armed = true;
        // Sorted insert (ascending expiry; FIFO among equals).
        let mut prev: *mut HrTimer = core::ptr::null_mut();
        let mut cur = (*q).first;
        while !cur.is_null() && (*cur).expires_ns <= expires {
            prev = cur;
            cur = (*cur).next;
        }
        (*t).prev = prev;
        (*t).next = cur;
        if prev.is_null() {
            (*q).first = t;
        } else {
            (*prev).next = t;
        }
        if !cur.is_null() {
            (*cur).prev = t;
        }
        (*q).count += 1;
    }
}

/// Cancel `t`: returns true when the timer was armed and had not yet
/// fired (the cancellation raced the expiry and won — the callback will
/// never run), false when it had already fired or was not armed.
///
/// # Safety
/// Caller owns `t` (it stays valid; only queue membership ends).
pub unsafe fn hrtimer_cancel(t: *mut HrTimer) -> bool {
    let _guard = HRTIMER_LOCK.lock();
    // SAFETY: queue lock held.
    unsafe {
        let q = core::ptr::addr_of_mut!(QUEUE);
        let was_armed = (*t).armed;
        hrtimer_unlink_locked(&mut *q, t);
        was_armed && !(*t).fired
    }
}

/// Unlink + disarm (queue lock held).
///
/// # Safety
/// HRTIMER_LOCK held.
unsafe fn hrtimer_unlink_locked(q: &mut HrQueue, t: *mut HrTimer) {
    // SAFETY: caller contract; `t` is either queued or already clean.
    unsafe {
        if !(*t).armed {
            return;
        }
        if !(*t).prev.is_null() {
            (*(*t).prev).next = (*t).next;
        } else if (*q).first == t {
            (*q).first = (*t).next;
        }
        if !(*t).next.is_null() {
            (*(*t).next).prev = (*t).prev;
        }
        (*t).next = core::ptr::null_mut();
        (*t).prev = core::ptr::null_mut();
        (*t).armed = false;
        (*q).count = (*q).count.saturating_sub(1);
    }
}

/// Earliest pending expiry (for tick-driven dispatch; 0 when empty).
pub fn next_expiry_ns() -> u64 {
    let _guard = HRTIMER_LOCK.lock();
    // SAFETY: queue lock held; read-only walk of the head.
    unsafe {
        let q = core::ptr::addr_of!(QUEUE);
        if (*q).first.is_null() {
            0
        } else {
            (*(*q).first).expires_ns
        }
    }
}

/// Pending timer count (suite/tests).
pub fn pending() -> usize {
    let _guard = HRTIMER_LOCK.lock();
    // SAFETY: queue lock held.
    unsafe { (*core::ptr::addr_of!(QUEUE)).count }
}

/// Expire everything due at `now`: timers are unlinked first and their
/// callbacks run outside the queue lock (callbacks may re-arm). Runs
/// from the timer interrupt; a callback waking sleepers must not
/// acquire HRTIMER_LOCK itself.
///
/// # Safety
/// Timer-interrupt or boot context (callback contracts apply).
pub unsafe fn run_expired(now: u64) -> usize {
    const BATCH: usize = 16;
    let mut fired = 0usize;
    loop {
        let mut batch: [*mut HrTimer; BATCH] = [core::ptr::null_mut(); BATCH];
        let n = {
            let _guard = HRTIMER_LOCK.lock();
            // SAFETY: queue lock held; collect due timers.
            unsafe {
                let q = core::ptr::addr_of_mut!(QUEUE);
                let mut n = 0usize;
                while n < BATCH && !(*q).first.is_null() && (*(*q).first).expires_ns <= now {
                    batch[n] = (*q).first;
                    hrtimer_unlink_locked(&mut *q, batch[n]);
                    (*batch[n]).fired = true;
                    n += 1;
                }
                n
            }
        };
        if n == 0 {
            break;
        }
        fired += n;
        for i in 0..n {
            // SAFETY: timer was owned by the queue; the armer keeps it
            // alive until cancel/expiry observes completion.
            unsafe {
                if let Some(cb) = (*batch[i]).callback {
                    cb(batch[i]);
                }
            }
        }
    }
    fired
}
