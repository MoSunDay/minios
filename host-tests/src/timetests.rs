//! Time-layer policy tests (21 §4): wheel placement/cascade, hrtimer
//! order/cancel, clocksource ratio math. Real kernel time modules,
//! included by path — they hold global statics, so every test that
//! touches them serializes through `time_lock` and arms relative to
//! the live jiffies/queue state (never assuming a fresh wheel).

use std::sync::{Mutex, MutexGuard};

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::time::{clocksource, hrtimer, wheel};

/// Serializes tests that touch the wheel/hrtimer global statics
/// (cargo otherwise runs #[test]s in parallel threads).
fn time_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap()
}

// ---- probe plumbing (atomics: no `static mut` references) ----

/// Ring of hrtimer addresses in callback-run order.
static FIRED_ORDER: [AtomicUsize; 8] = [const { AtomicUsize::new(0) }; 8];
/// Count of callbacks run since the last reset.
static FIRED_N: AtomicUsize = AtomicUsize::new(0);
/// jiffies at which each wheel probe ran (u64::MAX = never fired).
static WHEEL_FIRED_AT: AtomicU64 = AtomicU64::new(u64::MAX);
/// Second probe's jiffies (kept separate so deletions are checkable).
static WHEEL2_FIRED_AT: AtomicU64 = AtomicU64::new(u64::MAX);

/// Module-level timers: callbacks receive raw pointers, so armed
/// storage needs a stable address for the whole armed period.
static mut T1: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut T2: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut T3: hrtimer::HrTimer = hrtimer::HrTimer::new();
static mut WHEEL_T: wheel::LrTimer = wheel::LrTimer::new();

/// Reset the probe statics so runs are order-independent.
fn reset_probes() {
    FIRED_N.store(0, Ordering::SeqCst);
    for slot in FIRED_ORDER.iter() {
        slot.store(0, Ordering::SeqCst);
    }
    WHEEL_FIRED_AT.store(u64::MAX, Ordering::SeqCst);
    WHEEL2_FIRED_AT.store(u64::MAX, Ordering::SeqCst);
}

/// Index+1 of `t` within `table` (0 when absent): identifies fired
/// timers by address without reading their state.
fn tag_of_ptr<T>(t: *mut T, table: &[*mut T]) -> usize {
    for (i, e) in table.iter().enumerate() {
        if *e == t {
            return i + 1;
        }
    }
    0
}

/// Hrtimer probe: appends the timer address to the fire-order ring.
fn hr_probe(t: *mut hrtimer::HrTimer) {
    let n = FIRED_N.fetch_add(1, Ordering::SeqCst);
    if n < FIRED_ORDER.len() {
        FIRED_ORDER[n].store(t as usize, Ordering::SeqCst);
    }
}

/// Wheel probes record the jiffies they ran at. Safe: wheel_tick runs
/// callbacks after dropping the wheel lock (verified in wheel.rs), so
/// jiffies() can take it from here.
fn wheel_probe(_t: *mut wheel::LrTimer) {
    WHEEL_FIRED_AT.store(wheel::jiffies(), Ordering::SeqCst);
}

fn wheel_probe2(_t: *mut wheel::LrTimer) {
    WHEEL2_FIRED_AT.store(wheel::jiffies(), Ordering::SeqCst);
}

/// Fire order as tags over `table` (1 = first entry, ...).
fn fired_tags(table: &[*mut hrtimer::HrTimer]) -> Vec<usize> {
    (0..FIRED_N.load(Ordering::SeqCst))
        .map(|i| {
            let t = FIRED_ORDER[i].load(Ordering::SeqCst) as *mut hrtimer::HrTimer;
            tag_of_ptr(t, table)
        })
        .collect()
}

/// Arm static timer `t` at absolute ns `at` with the order probe.
///
/// # Safety
/// `t` must be one of the static timers above (stable address) and the
/// caller must hold `time_lock`.
unsafe fn arm_at(t: *mut hrtimer::HrTimer, at: u64) {
    // SAFETY: caller contract; hrtimer_start keeps `t` queued until it
    // fires, is cancelled, or is re-armed.
    unsafe { hrtimer::hrtimer_start(t, at, hrtimer::HrMode::Abs, hr_probe) }
}

// ---- pure placement math ----

#[test]
fn wheel_level_thresholds() {
    let cases: [(u64, usize); 10] = [
        (0, 0),
        (255, 0),           // idx 2^8-1 still tv1
        (256, 1),           // idx 2^8 wraps into tv2
        ((1 << 14) - 1, 1), // idx 2^14-1 still tv2
        (1 << 14, 2),       // idx 2^14 wraps into tv3
        ((1 << 20) - 1, 2), // idx 2^20-1 still tv3
        (1 << 20, 3),       // idx 2^20 wraps into tv4
        ((1 << 26) - 1, 3), // idx 2^26-1 still tv4
        (1 << 26, 4),       // horizon idx clamps to tv5
        (u64::MAX / 4, 4),  // far-horizon idx clamps to tv5
    ];
    for (idx, lvl) in cases {
        assert!(
            wheel::level_of(idx) == lvl,
            "idx {idx} must sit at level {lvl}"
        );
    }
    assert!(wheel::WHEEL_HORIZON == 1u64 << 26, "horizon = 2^26 jiffies");
    assert!(
        wheel::TVR_BITS == 8 && wheel::TVN_BITS == 6,
        "tv1 256 buckets, uppers 64"
    );
}

#[test]
fn wheel_bucket_bits() {
    assert!(wheel::bucket_of(0, 0xABCDEF) == 0xEF, "expires & 255");
    let shifts: [(usize, u32); 4] = [(1, 8), (2, 14), (3, 20), (4, 26)];
    for e in [0xDEADBEEFu64, 0x123456789u64] {
        for (lvl, shift) in shifts {
            assert!(
                wheel::bucket_of(lvl, e) == ((e >> shift) & 63) as usize,
                "lvl {lvl} buckets by (expires >> {shift}) & 63, e={e:#x}"
            );
        }
    }
}

// ---- wheel queue discipline (locked; global statics) ----

/// A 300-jiffy timer arms at tv2 and must cascade down once; it runs on
/// its exact expiry jiffy, no earlier and no later.
#[test]
fn wheel_cascade_fires_exact_jiffy() {
    let _g = time_lock();
    reset_probes();
    let base = wheel::jiffies();
    let baseline = wheel::pending();
    let expires = base + 300;
    assert!(wheel::level_of(expires - base) == 1, "distance 300 = tv2");
    // SAFETY: WHEEL_T is a static (stable address) and the test is
    // serialized; the timer is fully drained before this test returns.
    unsafe { wheel::add_timer(addr_of_mut!(WHEEL_T), expires, wheel_probe) };
    assert!(
        unsafe { (*addr_of!(WHEEL_T)).queued() },
        "armed timer reports queued"
    );
    assert!(
        unsafe { (*addr_of!(WHEEL_T)).expires() } == expires,
        "queued timer carries its absolute expiry"
    );
    assert!(
        wheel::pending() == baseline + 1,
        "pending grew by exactly one timer"
    );
    for j in (base + 1)..=expires {
        // SAFETY: explicit-tick test context; the only queued timer is
        // the static WHEEL_T and its callback only stores an atomic.
        let ran = unsafe { wheel::wheel_tick(j) };
        if j < expires {
            assert!(
                ran == 0,
                "jiffy {j} is before expires {expires} but ran {ran}"
            );
        } else {
            assert!(ran == 1, "exact-expiry jiffy {j} ran {ran}, want 1");
        }
    }
    assert!(
        WHEEL_FIRED_AT.load(Ordering::SeqCst) == expires,
        "callback observed jiffies == expires exactly (after the cascade)"
    );
    assert!(
        !unsafe { (*addr_of!(WHEEL_T)).queued() },
        "fired timer is unlinked"
    );
    assert!(wheel::pending() == baseline, "pending back to baseline");
}

/// del_timer on a queued timer wins the race: the callback never runs.
#[test]
fn wheel_del_timer_prevents_fire() {
    let _g = time_lock();
    reset_probes();
    let base = wheel::jiffies();
    let baseline = wheel::pending();
    let mut t = wheel::LrTimer::new();
    // SAFETY: stack timer deleted before return, so no dangling link.
    unsafe { wheel::add_timer(&mut t, base + 3, wheel_probe2) };
    assert!(t.queued(), "armed stack timer is queued");
    assert!(wheel::pending() == baseline + 1, "pending grew by one");
    // SAFETY: `t` is owned by this frame and still queued.
    let was_queued = unsafe { wheel::del_timer(&mut t) };
    assert!(was_queued, "del_timer true = was armed and had not run");
    assert!(!t.queued(), "deleted timer is unlinked");
    assert!(
        wheel::pending() == baseline,
        "pending restored after delete"
    );
    // SAFETY: explicit-tick test context; nothing is queued anymore.
    let ran = unsafe { wheel::wheel_tick(base + 5) };
    assert!(ran == 0, "wheel passed the old expiry and ran {ran}");
    assert!(
        WHEEL2_FIRED_AT.load(Ordering::SeqCst) == u64::MAX,
        "deleted timer's callback never ran"
    );
    assert!(wheel::pending() == baseline, "pending still at baseline");
}

/// Past-expiry timers park in tv1's current bucket and run (late, via
/// the expires < j drain path) when the wheel reaches that bucket again
/// — the first revisit of bucket `base & 255` is jiffy base+256.
#[test]
fn wheel_past_expiry_runs_on_revisit() {
    let _g = time_lock();
    reset_probes();
    let base = wheel::jiffies();
    let baseline = wheel::pending();
    let mut t = wheel::LrTimer::new();
    // SAFETY: stack timer armed already-expired; it fires before return.
    unsafe { wheel::add_timer(&mut t, base, wheel_probe) };
    assert!(t.queued(), "past-expiry timer is parked, not dropped");
    assert!(wheel::pending() == baseline + 1, "pending grew by one");
    let revisit = base + 256; // first j > base with j & 255 == base & 255
    for j in (base + 1)..=revisit {
        // SAFETY: explicit-tick test context; the only queued timer is
        // this frame's `t` and its callback only stores an atomic.
        let ran = unsafe { wheel::wheel_tick(j) };
        if j < revisit {
            assert!(ran == 0, "jiffy {j}: parked bucket not yet reached");
        } else {
            assert!(ran == 1, "bucket revisit {j} runs the late-parked timer");
        }
    }
    assert!(
        WHEEL_FIRED_AT.load(Ordering::SeqCst) == revisit,
        "late-parked timer ran at the bucket revisit"
    );
    assert!(!t.queued(), "fired timer is unlinked");
    assert!(wheel::pending() == baseline, "pending back to baseline");
}

// ---- hrtimer queue discipline (locked; global statics) ----

/// The queue stays sorted by absolute expiry: due timers run earliest
/// first regardless of insertion order.
#[test]
fn hrtimer_expiry_order() {
    let _g = time_lock();
    reset_probes();
    let base = 1_000_000u64;
    let table = [addr_of_mut!(T1), addr_of_mut!(T2), addr_of_mut!(T3)];
    // SAFETY: static timers (stable addresses), serialized by time_lock.
    unsafe {
        arm_at(addr_of_mut!(T1), base + 30_000_000);
        arm_at(addr_of_mut!(T2), base + 10_000_000);
        arm_at(addr_of_mut!(T3), base + 20_000_000);
    }
    assert!(
        hrtimer::next_expiry_ns() == base + 10_000_000,
        "queue head is the earliest expiry (T2), not the first inserted (T1)"
    );
    assert!(hrtimer::pending() == 3, "all three armed");
    // SAFETY: boot-context test; callbacks only touch probe atomics.
    let ran = unsafe { hrtimer::run_expired(base + 15_000_000) };
    assert!(ran == 1, "only T2 was due at base+15ms, ran {ran}");
    assert!(fired_tags(&table) == vec![2], "T2 fired first");
    // SAFETY: static timers, read-only while we hold time_lock.
    let (f1, f2, f3) = unsafe {
        (
            (*addr_of!(T1)).fired(),
            (*addr_of!(T2)).fired(),
            (*addr_of!(T3)).fired(),
        )
    };
    assert!(f2 && !f1 && !f3, "exactly T2 fired at the first deadline");
    assert!(
        hrtimer::next_expiry_ns() == base + 20_000_000,
        "after T2 ran, T3 is the new head"
    );
    // SAFETY: same context; drains the rest of the queue.
    let ran = unsafe { hrtimer::run_expired(base + 40_000_000) };
    assert!(ran == 2, "T3 and T1 were both due at base+40ms, ran {ran}");
    assert!(
        fired_tags(&table) == vec![2, 3, 1],
        "expiry order T2, T3, T1 regardless of insertion order T1, T2, T3"
    );
    assert!(hrtimer::pending() == 0, "queue drained");
}

/// Cancel of an armed, unfired timer wins the race: the callback never
/// runs, even when expiry is later dispatched past its deadline.
#[test]
fn hrtimer_cancel_wins_race() {
    let _g = time_lock();
    reset_probes();
    const AT: u64 = 500_000_000;
    // SAFETY: static timer; the queue is left empty before return.
    unsafe { arm_at(addr_of_mut!(T1), AT) };
    assert!(
        hrtimer::next_expiry_ns() == AT,
        "armed at the fresh absolute ns"
    );
    assert!(hrtimer::pending() == 1, "one timer armed");
    // SAFETY: T1 stays valid; only queue membership ends.
    let won = unsafe { hrtimer::hrtimer_cancel(addr_of_mut!(T1)) };
    assert!(won, "cancel of armed+unfired timer reports true");
    // SAFETY: boot-context test; nothing is armed.
    let ran = unsafe { hrtimer::run_expired(2 * AT) };
    assert!(
        ran == 0,
        "expiry dispatched past AT but the cancel already won"
    );
    // SAFETY: static timer, read-only under time_lock.
    let fired = unsafe { (*addr_of!(T1)).fired() };
    assert!(!fired, "cancelled timer never reports fired");
    assert!(FIRED_N.load(Ordering::SeqCst) == 0, "probe never ran");
    assert!(hrtimer::pending() == 0, "queue empty after cancel");
    // Second race over the same timer: re-arm then cancel again.
    // SAFETY: same as above; queue drained by the cancel itself.
    unsafe { arm_at(addr_of_mut!(T1), AT + 1) };
    let won2 = unsafe { hrtimer::hrtimer_cancel(addr_of_mut!(T1)) };
    assert!(won2, "re-armed timer cancels again");
    let ran2 = unsafe { hrtimer::run_expired(2 * AT + 2) };
    assert!(ran2 == 0, "nothing runs after the second cancel");
    assert!(hrtimer::pending() == 0, "queue empty");
}

/// Re-arming repositions: the old queue position disappears and the
/// timer fires exactly once, at the new expiry.
#[test]
fn hrtimer_rearm_repositions() {
    let _g = time_lock();
    reset_probes();
    // SAFETY: static timer, fired before the test returns.
    unsafe { arm_at(addr_of_mut!(T2), 900_000_000) };
    // SAFETY: re-arm of the same timer (unlinks the old entry first).
    unsafe { arm_at(addr_of_mut!(T2), 10_000) };
    assert!(
        hrtimer::next_expiry_ns() == 10_000,
        "queue head follows the re-arm position"
    );
    assert!(hrtimer::pending() == 1, "re-arm must not duplicate");
    // SAFETY: boot-context test; callbacks only touch probe atomics.
    let ran = unsafe { hrtimer::run_expired(10_001) };
    assert!(ran == 1, "fires exactly once at the new position");
    assert!(FIRED_N.load(Ordering::SeqCst) == 1, "one callback total");
    // SAFETY: same context; the old position must be gone.
    let ran2 = unsafe { hrtimer::run_expired(900_000_001) };
    assert!(ran2 == 0, "old 900ms position vanished after the re-arm");
    assert!(hrtimer::pending() == 0, "queue drained");
}

// ---- clocksource ratio math (pure) ----

#[test]
fn clocksource_cycles_ratio() {
    // ns = cycles * PIT_INTERVAL_NS / cycles_per_interval (u128 math).
    // 10M cycles per 10 ms interval = 1 GHz: nanoseconds == cycles.
    assert!(
        clocksource::cycles_to_ns_with(10_000_000, 10_000_000) == 10_000_000,
        "1 GHz: 10M cycles over a 10M-cycle interval is exactly one interval"
    );
    // Still 1 GHz, twice the interval: 20M cycles = 20 ms.
    assert!(
        clocksource::cycles_to_ns_with(20_000_000, 10_000_000) == 20_000_000,
        "1 GHz: 20M cycles = 20 ms"
    );
    // 20M cycles per 10 ms interval = 2 GHz: 100k cycles = 100_000 *
    // 10_000_000 / 20_000_000 = 50_000 ns (50 µs) — derived, not guessed.
    assert!(
        clocksource::cycles_to_ns_with(100_000, 20_000_000) == 50_000,
        "2 GHz: 100k cycles = 50_000 ns"
    );
    // Uncalibrated (per == 0): report 0, never divide by zero.
    assert!(
        clocksource::cycles_to_ns_with(123, 0) == 0,
        "uncalibrated source reports 0 ns"
    );
}
