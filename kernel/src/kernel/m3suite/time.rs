//! M3 time-layer cases (21 §4): rated clocksource/clockevents wiring
//! (TSC won at calibration, PIT in periodic mode), hrtimer sorted
//! insert / batched expiry / cancel-wins-the-race semantics, and a
//! real wheel cascade (tv2 -> tv1) driven by the live tick interrupt.

use crate::arch::x86_64::io;
use crate::kernel::time::{clockevents, clocksource, hrtimer, wheel};
use core::sync::atomic::{AtomicUsize, Ordering};

// ---- time layer (21 §4): clocksource/clockevents wiring, hrtimer
// order/cancel, and a real wheel cascade driven by the live tick. ----

/// m3.time.clock_layer_wiring: rated clocksource switch (TSC 300 won at
/// calibration), monotonic progress, PIT event device in periodic mode.
pub(super) fn case_time_clock_layer_wiring() -> bool {
    let dev = clockevents::active();
    let ok_dev = dev.name == "pit"
        && dev.hz == 100
        && dev.features & clockevents::CLOCK_EVT_FEAT_PERIODIC != 0
        && dev.rating == 100;
    let a = clocksource::monotonic_ns();
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    let b = clocksource::monotonic_ns();
    clocksource::active_rating() == clocksource::TSC_RATING
        && clocksource::TSC_RATING > clocksource::JIFFIES_RATING
        && a > 0
        && b > a
        && clocksource::wall_ns() >= b
        && ok_dev
}

/// Timers for the hrtimer order/cancel case (stable addresses: callbacks
/// receive raw pointers; accessed only through addr_of, never &mut).
static mut HR_T1: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut HR_T2: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut HR_T3: hrtimer::HrTimer = hrtimer::HrTimer::new();
/// Bitmask of fired timers (bit0 T1, bit1 T2, bit2 T3).
static HR_ORDER: AtomicUsize = AtomicUsize::new(0);

/// Record which timer fired into HR_ORDER.
fn hr_probe(t: *mut hrtimer::HrTimer) {
    let bit = if t as usize == core::ptr::addr_of!(HR_T1) as usize {
        1
    } else if t as usize == core::ptr::addr_of!(HR_T2) as usize {
        2
    } else {
        4
    };
    HR_ORDER.fetch_or(bit, Ordering::SeqCst);
}

/// m3.time.hrtimer_order_cancel: sorted insert regardless of insertion
/// order, batched expiry in expiry order, and cancel-wins-the-race
/// semantics (a cancelled timer's callback never runs).
pub(super) fn case_time_hrtimer_order_cancel() -> bool {
    const BASE: u64 = 1_000_000;
    HR_ORDER.store(0, Ordering::SeqCst);
    // SAFETY: boot context; the timers are module statics that outlive
    // every armed window (fired or cancelled within this case).
    unsafe {
        let t1 = core::ptr::addr_of_mut!(HR_T1);
        let t2 = core::ptr::addr_of_mut!(HR_T2);
        let t3 = core::ptr::addr_of_mut!(HR_T3);
        // Deliberately out of expiry order: T1 latest, T2 earliest.
        hrtimer::hrtimer_start(t1, BASE + 30_000_000, hrtimer::HrMode::Abs, hr_probe);
        hrtimer::hrtimer_start(t2, BASE + 10_000_000, hrtimer::HrMode::Abs, hr_probe);
        hrtimer::hrtimer_start(t3, BASE + 20_000_000, hrtimer::HrMode::Abs, hr_probe);
        // Sorted insert: the head is the earliest expiry, not the last
        // started timer.
        if hrtimer::next_expiry_ns() != BASE + 10_000_000 {
            return false;
        }
        // Cancel before any expiry: the callback must never run.
        if !hrtimer::hrtimer_cancel(t3) {
            return false;
        }
        // First batch at T2's expiry: only T2 ran, T1 still pending.
        let n1 = hrtimer::run_expired(BASE + 15_000_000);
        if n1 != 1 || HR_ORDER.load(Ordering::SeqCst) != 2 {
            return false;
        }
        if hrtimer::pending() != 1 || hrtimer::next_expiry_ns() != BASE + 30_000_000 {
            return false;
        }
        // Second batch past T1's expiry: T3's bit stays clear (the
        // cancel won the race), T1 fired.
        let n2 = hrtimer::run_expired(BASE + 40_000_000);
        if n2 != 1 || HR_ORDER.load(Ordering::SeqCst) != 1 | 2 {
            return false;
        }
        // Cancel after fire loses the race; the fired flag proves the
        // callback ran while the cancelled timer never did.
        if !(!hrtimer::hrtimer_cancel(t1)
            && hrtimer::pending() == 0
            && (*t1).fired()
            && !(*t3).fired())
        {
            return false;
        }
        // Rel anchoring: a zero offset arms at the current monotonic
        // read, so the timer is already due by the next expire pass
        // (monotonic strictly increases between the two reads). T3's
        // order bit is still clear (its cancel won the race above), so
        // the re-fire completes the mask.
        hrtimer::hrtimer_start(t3, 0, hrtimer::HrMode::Rel, hr_probe);
        hrtimer::next_expiry_ns() != 0
            && hrtimer::run_expired(clocksource::monotonic_ns()) == 1
            && HR_ORDER.load(Ordering::SeqCst) == 1 | 2 | 4
            && (*t3).fired()
            && hrtimer::pending() == 0
    }
}

/// Jiffy at which the wheel probe ran (usize::MAX = never fired).
static WHEEL_FIRED_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Timer for the wheel cascade case (stable address for the callback;
/// accessed only through addr_of, never &mut).
static mut WHEEL_T: wheel::LrTimer = wheel::LrTimer::new();

/// Record the jiffy the wheel probe ran at.
fn wheel_probe(_t: *mut wheel::LrTimer) {
    WHEEL_FIRED_AT.store(crate::kernel::time::ticks() as usize, Ordering::SeqCst);
}

/// m3.time.wheel_cascade: a tv2-placed timer (t0+300) that only reaches
/// tv1 after the tv1 index wraps — the real cascade path, driven by the
/// live tick interrupt. Fires exactly on its expiry jiffy.
pub(super) fn case_time_wheel_cascade() -> bool {
    let baseline = wheel::pending();
    let t0 = crate::kernel::time::ticks();
    let expires = t0 + 300; // level 1 by the wheel placement math
    WHEEL_FIRED_AT.store(usize::MAX, Ordering::SeqCst);
    let wt = core::ptr::addr_of_mut!(WHEEL_T);
    // SAFETY: boot context; WHEEL_T is a module static that outlives
    // the armed window (fired or dequeued before this case returns).
    unsafe {
        wheel::add_timer(wt, expires, wheel_probe);
    }
    // SAFETY: plain queued-flag read of the static timer.
    let armed = unsafe { (*wt).queued() };
    if !(wheel::pending() == baseline + 1 && armed) {
        // SAFETY: boot context; dequeue the un-fired timer.
        unsafe {
            wheel::del_timer(wt);
        }
        return false;
    }
    // Boot context runs with interrupts off; the tick interrupt drives
    // wheel_tick, so enable interrupts around the wait (suite.rs idiom)
    // and restore the boot state after.
    io::sti();
    while crate::kernel::time::ticks() < expires + 2
        && WHEEL_FIRED_AT.load(Ordering::SeqCst) == usize::MAX
    {
        io::hlt();
    }
    io::cli();
    // SAFETY: plain queued-flag read of the static timer.
    let ok = WHEEL_FIRED_AT.load(Ordering::SeqCst) == expires as usize
        && !unsafe { (*wt).queued() }
        && wheel::pending() == baseline;
    if !ok && armed {
        // SAFETY: boot context; dequeue the un-fired timer.
        unsafe {
            wheel::del_timer(wt);
        }
    }
    ok
}
