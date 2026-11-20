//! Linux-style wait queue (UP): an intrusive list of sleepers with
//! interruptible/exclusive flags (21 §3). Pure list policy — the sleep
//! glue (wq_sleep/wake_up, PTABLE_LOCK, myproc) lives in sleep.rs so
//! this module stays host-testable like rt.rs/cfs.rs.
//!
//! Wake discipline (Linux `__wake_up`): non-exclusive entries queue at
//! the head and wake in every wake-up; exclusive entries queue at the
//! tail and `wake_up_one` stops after the first — thundering-herd
//! avoidance for capacity-style waits (pipe writers).

/// Intrusive sleeper link (one per Proc: a UP process waits on at most
/// one queue at a time).
pub struct WaitLink {
    next: *mut WaitLink,
    prev: *mut WaitLink,
    queue: *mut WaitQueue,
    exclusive: bool,
    interruptible: bool,
}

impl WaitLink {
    pub const fn new() -> WaitLink {
        WaitLink {
            next: core::ptr::null_mut(),
            prev: core::ptr::null_mut(),
            queue: core::ptr::null_mut(),
            exclusive: false,
            interruptible: false,
        }
    }

    /// Whether an interrupt (fatal kill today; pending signal at s6)
    /// may break this wait.
    #[allow(dead_code)] // s6: signal wake-up filters on this
    pub fn interruptible(&self) -> bool {
        self.interruptible
    }
}

/// FIFO-ordered wait queue head. All mutation happens under PTABLE_LOCK
/// (the resource lock — pipe, ptable — is held on entry and ordered
/// before it, exactly like the chan sleep protocol).
pub struct WaitQueue {
    first: *mut WaitLink,
    last: *mut WaitLink,
}

impl WaitQueue {
    pub const fn new() -> WaitQueue {
        WaitQueue {
            first: core::ptr::null_mut(),
            last: core::ptr::null_mut(),
        }
    }

    /// The chan value sleepers block on (the queue address).
    pub fn chan(&self) -> usize {
        self as *const WaitQueue as usize
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_null()
    }
}

/// Pure wake-set policy (host tests + m3 suite drive this directly):
/// fill `out` with the links a wake-up would target, FIFO first.
/// `one` = all non-exclusive entries plus the first exclusive one;
/// otherwise every entry. Returns the wake-set size.
pub const MAX_WAITERS: usize = 64; // NPROC (kernel proc config)

pub fn select_wake(q: &WaitQueue, one: bool, out: &mut [*mut WaitLink; MAX_WAITERS]) -> usize {
    let mut n = 0usize;
    let mut l = q.first;
    while !l.is_null() && n < MAX_WAITERS {
        // SAFETY: list links are live (queue invariants hold under the
        // ptable lock; pure reads here).
        let link = unsafe { &*l };
        out[n] = l;
        n += 1;
        if one && link.exclusive {
            break;
        }
        l = link.next;
    }
    n
}

/// Enqueue a link (ptable lock held): exclusive at the tail,
/// non-exclusive at the head (Linux `__add_wait_queue{,_tail}`).
/// Link-level so host tests and the m3 suite drive the pure list.
///
/// # Safety
/// PTABLE_LOCK held; the link is not queued.
pub unsafe fn enqueue_link(
    l: &mut WaitLink,
    q: &mut WaitQueue,
    interruptible: bool,
    exclusive: bool,
) {
    // SAFETY: ptable lock held; we own the link.
    unsafe {
        (*l).interruptible = interruptible;
        (*l).exclusive = exclusive;
        (*l).queue = q as *mut WaitQueue;
        if exclusive {
            let old = q.last;
            (*l).prev = old;
            (*l).next = core::ptr::null_mut();
            if old.is_null() {
                q.first = l;
            } else {
                (*old).next = l;
            }
            q.last = l;
        } else {
            let old = q.first;
            (*l).next = old;
            (*l).prev = core::ptr::null_mut();
            if old.is_null() {
                q.last = l;
            } else {
                (*old).prev = l;
            }
            q.first = l;
        }
    }
}

/// Idempotent unlink (ptable lock held).
///
/// # Safety
/// PTABLE_LOCK held; the link's queue pointers are consistent.
pub unsafe fn unlink(l: &mut WaitLink) {
    // SAFETY: caller contract.
    unsafe {
        if l.queue.is_null() {
            return;
        }
        let q = l.queue;
        if !l.prev.is_null() {
            (*l.prev).next = l.next;
        } else {
            (*q).first = l.next;
        }
        if !l.next.is_null() {
            (*l.next).prev = l.prev;
        } else {
            (*q).last = l.prev;
        }
        l.next = core::ptr::null_mut();
        l.prev = core::ptr::null_mut();
        l.queue = core::ptr::null_mut();
        l.exclusive = false;
        l.interruptible = false;
    }
}
