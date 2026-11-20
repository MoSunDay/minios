//! Jiffies timer wheel (21 §4, Linux 2.6 shape): the low-resolution
//! compat interface — 256 first-level buckets cascading into four 64-
//! bucket levels (horizon 2^26 jiffies; beyond that clamps to tv5).
//! Timers are intrusive links owned by their armer; expiry runs from
//! the periodic tick. Pure placement math (level_of/bucket_of) is
//! host-testable against the Linux idx/expires formulas.

use crate::klib::sync::Spinlock;

pub const TVR_BITS: usize = 8;
pub const TVN_BITS: usize = 6;
pub const TVR_SIZE: usize = 1 << TVR_BITS;
pub const TVN_SIZE: usize = 1 << TVN_BITS;
/// tv5 horizon: 1 << (TVR_BITS + 3*TVN_BITS) buckets * 64 span.
pub const WHEEL_HORIZON: u64 = 1u64 << (TVR_BITS + 3 * TVN_BITS);

/// Low-resolution timer (struct timer_list shape).
pub struct LrTimer {
    expires: u64,
    next: *mut LrTimer,
    prev: *mut LrTimer,
    queued: bool,
    callback: Option<fn(*mut LrTimer)>,
}

impl LrTimer {
    pub const fn new() -> LrTimer {
        LrTimer {
            expires: 0,
            next: core::ptr::null_mut(),
            prev: core::ptr::null_mut(),
            queued: false,
            callback: None,
        }
    }

    pub fn queued(&self) -> bool {
        self.queued
    }

    /// Absolute jiffy deadline (sys_alarm remaining-time arithmetic).
    pub fn expires(&self) -> u64 {
        self.expires
    }
}

struct Wheel {
    /// Absolute jiffies position the wheel has advanced to.
    jiffies: u64,
    tv1: [*mut LrTimer; TVR_SIZE],
    tv2: [*mut LrTimer; TVN_SIZE],
    tv3: [*mut LrTimer; TVN_SIZE],
    tv4: [*mut LrTimer; TVN_SIZE],
    tv5: [*mut LrTimer; TVN_SIZE],
}

static WHEEL_LOCK: Spinlock<()> = Spinlock::new("twheel", ());
static mut WHEEL: Wheel = Wheel {
    jiffies: 0,
    tv1: [core::ptr::null_mut(); TVR_SIZE],
    tv2: [core::ptr::null_mut(); TVN_SIZE],
    tv3: [core::ptr::null_mut(); TVN_SIZE],
    tv4: [core::ptr::null_mut(); TVN_SIZE],
    tv5: [core::ptr::null_mut(); TVN_SIZE],
};

/// Linux internal_add_timer placement: level by idx distance, bucket by
/// shifted expires bits (pure; host tests check the thresholds).
pub fn level_of(idx: u64) -> usize {
    if idx < TVR_SIZE as u64 {
        0
    } else if idx < 1 << (TVR_BITS + TVN_BITS) {
        1
    } else if idx < 1 << (TVR_BITS + 2 * TVN_BITS) {
        2
    } else if idx < WHEEL_HORIZON {
        3
    } else {
        4
    }
}

/// Bucket index within `lvl` for absolute `expires` (pure).
pub fn bucket_of(lvl: usize, expires: u64) -> usize {
    match lvl {
        0 => (expires & (TVR_SIZE as u64 - 1)) as usize,
        1 => ((expires >> TVR_BITS) & (TVN_SIZE as u64 - 1)) as usize,
        2 => ((expires >> (TVR_BITS + TVN_BITS)) & (TVN_SIZE as u64 - 1)) as usize,
        3 => ((expires >> (TVR_BITS + 2 * TVN_BITS)) & (TVN_SIZE as u64 - 1)) as usize,
        _ => ((expires >> (TVR_BITS + 3 * TVN_BITS)) & (TVN_SIZE as u64 - 1)) as usize,
    }
}

/// Head-insert into level `lvl` bucket `b` (wheel lock held).
///
/// # Safety
/// WHEEL_LOCK held; timer not queued.
unsafe fn level_push(w: &mut Wheel, lvl: usize, b: usize, t: *mut LrTimer) {
    // SAFETY: caller contract.
    unsafe {
        let head = *level_head_mut(w, lvl, b);
        (*t).prev = core::ptr::null_mut();
        (*t).next = head;
        if !head.is_null() {
            (*head).prev = t;
        }
        *level_head_mut(w, lvl, b) = t;
        (*t).queued = true;
    }
}

/// # Safety
/// WHEEL_LOCK held.
unsafe fn level_head_mut(w: &mut Wheel, lvl: usize, b: usize) -> &mut *mut LrTimer {
    // SAFETY: callers pass in-range lvl/b; indexing itself is safe.
    match lvl {
        0 => &mut w.tv1[b],
        1 => &mut w.tv2[b],
        2 => &mut w.tv3[b],
        3 => &mut w.tv4[b],
        _ => &mut w.tv5[b],
    }
}

/// Unlink a queued timer (wheel lock held). Head-of-bucket unlinks
/// search the (small) level for the matching head pointer.
///
/// # Safety
/// WHEEL_LOCK held.
unsafe fn unlink_locked(w: &mut Wheel, t: *mut LrTimer) {
    // SAFETY: caller contract; queue pointers consistent.
    unsafe {
        if !(*t).queued {
            return;
        }
        if !(*t).prev.is_null() {
            (*(*t).prev).next = (*t).next;
        } else {
            for lvl in 0..5 {
                for b in 0..level_len(lvl) {
                    if *level_head_mut(w, lvl, b) == t {
                        *level_head_mut(w, lvl, b) = (*t).next;
                    }
                }
            }
        }
        if !(*t).next.is_null() {
            (*(*t).next).prev = (*t).prev;
        }
        (*t).next = core::ptr::null_mut();
        (*t).prev = core::ptr::null_mut();
        (*t).queued = false;
    }
}

fn level_len(lvl: usize) -> usize {
    if lvl == 0 {
        TVR_SIZE
    } else {
        TVN_SIZE
    }
}

/// Queue (or re-queue) `t` to expire at absolute jiffies `expires`.
///
/// # Safety
/// `t` outlives its queued period (del_timer before dropping).
pub unsafe fn add_timer(t: *mut LrTimer, expires: u64, cb: fn(*mut LrTimer)) {
    let _guard = WHEEL_LOCK.lock();
    // SAFETY: wheel lock held; we own `t`.
    unsafe {
        let w = core::ptr::addr_of_mut!(WHEEL);
        unlink_locked(&mut *w, t);
        (*t).expires = expires;
        (*t).callback = Some(cb);
        let idx = expires.saturating_sub((*w).jiffies);
        // Linux: past/negative idx parks in tv1 at the current index
        // (expires on the next tick).
        if expires <= (*w).jiffies {
            let b = ((*w).jiffies & (TVR_SIZE as u64 - 1)) as usize;
            level_push(&mut *w, 0, b, t);
        } else {
            let lvl = level_of(idx);
            let b = bucket_of(lvl, expires);
            level_push(&mut *w, lvl, b, t);
        }
    }
}

/// Dequeue `t`; true when it was still queued (had not run).
///
/// # Safety
/// Caller owns `t`.
pub unsafe fn del_timer(t: *mut LrTimer) -> bool {
    let _guard = WHEEL_LOCK.lock();
    // SAFETY: wheel lock held.
    unsafe {
        let was = (*t).queued;
        let w = core::ptr::addr_of_mut!(WHEEL);
        unlink_locked(&mut *w, t);
        was
    }
}

/// Advance the wheel to `now` jiffies: expire due tv1 buckets,
/// cascading higher levels on wrap. Callbacks run outside the lock
/// (they may re-arm timers). Returns how many timers ran.
///
/// # Safety
/// Timer-interrupt context; callback contracts apply.
pub unsafe fn wheel_tick(now: u64) -> usize {
    let mut ran = 0usize;
    loop {
        let mut due: [*mut LrTimer; 32] = [core::ptr::null_mut(); 32];
        let n = {
            let _guard = WHEEL_LOCK.lock();
            // SAFETY: wheel lock held.
            unsafe {
                let w = core::ptr::addr_of_mut!(WHEEL);
                if (*w).jiffies >= now {
                    0
                } else {
                    (*w).jiffies += 1;
                    let j = (*w).jiffies;
                    let idx = (j & (TVR_SIZE as u64 - 1)) as usize;
                    if idx == 0 {
                        // Linux run_timer chain: cascade the next level
                        // while the wrapped bucket was empty.
                        if !cascade(&mut *w, 1) && !cascade(&mut *w, 2) && !cascade(&mut *w, 3) {
                            cascade(&mut *w, 4);
                        }
                    }
                    // Drain the bucket: timers with expires == j run;
                    // late-parked (expires < j) run too.
                    let mut n = 0usize;
                    let mut head = (*w).tv1[idx];
                    while !head.is_null() && n < 32 {
                        let next = (*head).next;
                        unlink_locked(&mut *w, head);
                        due[n] = head;
                        n += 1;
                        head = next;
                    }
                    n
                }
            }
        };
        if n == 0 {
            break;
        }
        ran += n;
        for i in 0..n {
            // SAFETY: timer was queued by its armer; callback contract.
            unsafe {
                if let Some(cb) = (*due[i]).callback {
                    cb(due[i]);
                }
            }
        }
    }
    ran
}

/// Move level `lvl`'s current bucket down a level (wheel lock held).
/// Returns false when the bucket was empty (stops the cascade chain).
///
/// # Safety
/// WHEEL_LOCK held.
unsafe fn cascade(w: &mut Wheel, lvl: usize) -> bool {
    // SAFETY: wheel lock held.
    unsafe {
        let shift = TVR_BITS + TVN_BITS * (lvl - 1);
        let b = ((w.jiffies >> shift) & (TVN_SIZE as u64 - 1)) as usize;
        let mut head = *level_head_mut(w, lvl, b);
        let mut moved = false;
        while !head.is_null() {
            let next = (*head).next;
            unlink_locked(w, head);
            let idx = (*head).expires.saturating_sub(w.jiffies);
            if (*head).expires <= w.jiffies {
                level_push(w, 0, (w.jiffies & 255) as usize, head);
            } else {
                let new_lvl = lvl - 1;
                level_push(w, new_lvl, bucket_of(new_lvl, (*head).expires), head);
            }
            moved = true;
            let _ = idx;
            head = next;
        }
        moved
    }
}

/// Pending queued timer count (suite/tests).
pub fn pending() -> usize {
    let _guard = WHEEL_LOCK.lock();
    // SAFETY: wheel lock held; pure count walk.
    unsafe {
        let w = core::ptr::addr_of!(WHEEL);
        let mut n = 0usize;
        for lvl in 0..5 {
            for b in 0..level_len(lvl) {
                let mut t = *level_head_mut(&mut *core::ptr::addr_of_mut!(WHEEL), lvl, b);
                while !t.is_null() {
                    n += 1;
                    t = (*t).next;
                }
            }
        }
        let _ = w;
        n
    }
}

/// Current wheel position in jiffies (under the wheel lock; sys_alarm
/// deadline arithmetic reads the wheel's own time base, not TICKS,
/// which may run one period ahead of the cascade).
pub fn jiffies() -> u64 {
    let _guard = WHEEL_LOCK.lock();
    // SAFETY: wheel lock held; plain counter read.
    unsafe { (*core::ptr::addr_of!(WHEEL)).jiffies }
}
