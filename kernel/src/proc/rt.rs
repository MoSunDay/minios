//! SCHED_FIFO/SCHED_RR runqueues (UP; plans/linux26/21 §2): 99 priority
//! levels, FIFO within a level, RR rotating on quantum expiry; an RT
//! entity always preempts CFS. Kernel preemption stays off: a woken
//! higher-priority entity is noticed at the next timer tick (no IPIs
//! before M8; ADR-0016/0017).
//!
//! Pure policy, no locks — sched.rs holds PTABLE_LOCK around every
//! mutation; host-tests drives this module with plain link-bearing
//! structs (the `Proc` container_of lives in sched.rs).

/// Inclusive RT priority bounds (Linux MAX_RT_PRIO − 1 .. 1).
pub const RT_PRIO_MAX: i32 = 99;
pub const RT_PRIO_MIN: i32 = 1;
/// SCHED_RR quantum in PIT ticks: 100 ms at 100 Hz (Linux default).
pub const RT_RR_QUANTUM_TICKS: i32 = 10;

const NR_LEVELS: usize = RT_PRIO_MAX as usize + 1; // index 0 unused

/// Intrusive doubly-linked list link (per RT entity).
#[repr(C)]
pub struct RtLink {
    pub prev: *mut RtLink,
    pub next: *mut RtLink,
}

impl RtLink {
    pub const fn new() -> RtLink {
        RtLink {
            prev: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        }
    }
}

/// The RT runqueue: per-level head/tail lists plus a set-level bitmap.
pub struct RtRunqueue {
    heads: [*mut RtLink; NR_LEVELS],
    tails: [*mut RtLink; NR_LEVELS],
    bitmap: u128,
    nr_rt: u32,
}

impl RtRunqueue {
    pub const fn new() -> RtRunqueue {
        RtRunqueue {
            heads: [core::ptr::null_mut(); NR_LEVELS],
            tails: [core::ptr::null_mut(); NR_LEVELS],
            bitmap: 0,
            nr_rt: 0,
        }
    }

    /// Any runnable RT entity? (CFS preemption check.)
    pub fn is_empty(&self) -> bool {
        self.bitmap == 0
    }

    #[cfg(test)] // model/audit consumers (host-tests crate)
    pub fn nr_running(&self) -> u32 {
        self.nr_rt
    }
}

/// Highest non-empty level, or None.
pub fn front_prio(rq: &RtRunqueue) -> Option<i32> {
    if rq.bitmap == 0 {
        return None;
    }
    let level = 127 - rq.bitmap.leading_zeros(); // highest set bit
    Some(level as i32 + 1) // bit (prio-1)
}

/// Link at the highest non-empty level (null when empty).
pub fn front(rq: &RtRunqueue) -> *mut RtLink {
    match front_prio(rq) {
        Some(p) => rq.heads[p as usize],
        None => core::ptr::null_mut(),
    }
}

/// Strictly higher-priority runnable entity exists?
pub fn has_higher(rq: &RtRunqueue, prio: i32) -> bool {
    match front_prio(rq) {
        Some(front) => front > prio,
        None => false,
    }
}

fn set_level(rq: &mut RtRunqueue, prio: i32) {
    rq.bitmap |= 1u128 << (prio as usize - 1);
}

fn clear_level(rq: &mut RtRunqueue, prio: i32) {
    rq.bitmap &= !(1u128 << (prio as usize - 1));
}

/// Append to the tail of `prio`'s list (FIFO arrival order).
///
/// # Safety
/// `link` is exclusively owned and in no list.
pub unsafe fn push_tail(rq: &mut RtRunqueue, link: *mut RtLink, prio: i32) {
    debug_assert!((RT_PRIO_MIN..=RT_PRIO_MAX).contains(&prio));
    // SAFETY: caller owns the link; list nodes are live links.
    unsafe {
        (*link).next = core::ptr::null_mut();
        (*link).prev = rq.tails[prio as usize];
        if !rq.tails[prio as usize].is_null() {
            (*rq.tails[prio as usize]).next = link;
        } else {
            rq.heads[prio as usize] = link;
        }
        rq.tails[prio as usize] = link;
        set_level(rq, prio);
        rq.nr_rt += 1;
    }
}

/// Push to the head of `prio`'s list (a preempted FIFO keeps its turn).
///
/// # Safety
/// `link` is exclusively owned and in no list.
pub unsafe fn push_head(rq: &mut RtRunqueue, link: *mut RtLink, prio: i32) {
    debug_assert!((RT_PRIO_MIN..=RT_PRIO_MAX).contains(&prio));
    // SAFETY: caller owns the link; list nodes are live links.
    unsafe {
        (*link).prev = core::ptr::null_mut();
        (*link).next = rq.heads[prio as usize];
        if !rq.heads[prio as usize].is_null() {
            (*rq.heads[prio as usize]).prev = link;
        } else {
            rq.tails[prio as usize] = link;
        }
        rq.heads[prio as usize] = link;
        set_level(rq, prio);
        rq.nr_rt += 1;
    }
}

/// Unlink from `prio`'s list.
///
/// # Safety
/// `link` is in `prio`'s list of this runqueue.
pub unsafe fn unlink(rq: &mut RtRunqueue, link: *mut RtLink, prio: i32) {
    // SAFETY: caller contract; neighbors/tail are live.
    unsafe {
        let prev = (*link).prev;
        let next = (*link).next;
        if !prev.is_null() {
            (*prev).next = next;
        } else {
            rq.heads[prio as usize] = next;
        }
        if !next.is_null() {
            (*next).prev = prev;
        } else {
            rq.tails[prio as usize] = prev;
        }
        (*link).prev = core::ptr::null_mut();
        (*link).next = core::ptr::null_mut();
        if rq.heads[prio as usize].is_null() {
            clear_level(rq, prio);
        }
        rq.nr_rt -= 1;
    }
}
