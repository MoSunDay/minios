//! Clocksource abstraction (21 §4): monotonic time from a rated source.
//! The TSC is calibrated against the PIT at boot; until then the jiffies
//! counter serves as the low-rated fallback (the rating switch is the
//! whole point of the abstraction — readers never care which one is
//! active). UP only; no NTP/adjust/suspend handling (M8+ parity list).

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::x86_64::io;

/// Rating (Linux clocksource rating conventions): higher wins.
pub const JIFFIES_RATING: u32 = 1;
pub const TSC_RATING: u32 = 300;

/// ns per 10 ms PIT interval, fixed by definition.
pub const PIT_INTERVAL_NS: u64 = 10_000_000;

/// TSC cycle count of one 10 ms PIT interval (0 until calibrated).
static CYCLES_PER_INTERVAL: AtomicU64 = AtomicU64::new(0);
/// TSC at calibration time — monotonic reads are deltas from here.
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);
/// True once the TSC calibration finished (TSC becomes the best source).
static TSC_READY: AtomicBool = AtomicBool::new(false);

/// Raw timestamp counter with a serializing fence before the read.
pub fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: rdtsc is unprivileged and always available under qemu64
    // (and on every x86_64 baseline we target); lfence orders it against
    // prior loads so the timestamp cannot be hoisted.
    unsafe {
        core::arch::asm!("lfence; rdtsc", out("eax") lo, out("edx") hi, out("ecx") _, options(nostack));
    }
    ((hi as u64) << 32) | lo as u64
}

/// Calibrate the TSC against `n_ticks` PIT ticks (tick counter already
/// running). Called once at boot; the wait briefly enables interrupts
/// because tick delivery is the thing being measured.
pub fn calibrate_tsc(n_ticks: u64) {
    let t0 = read_tsc();
    let j0 = super::TICKS.load(Ordering::SeqCst);
    // Boot context runs with interrupts off (only kernel::suite ever
    // raises IF before this point), so the PIT tick that advances TICKS
    // can never fire while the busy wait spins — enable interrupts
    // around the wait and restore the boot state after (the xv6
    // early-calibration idiom).
    io::sti();
    while super::TICKS.load(Ordering::SeqCst) < j0 + n_ticks {
        core::hint::spin_loop();
    }
    io::cli();
    let t1 = read_tsc();
    let cycles = t1.saturating_sub(t0);
    if cycles == 0 {
        return; // keep the jiffies fallback
    }
    CYCLES_PER_INTERVAL.store(cycles / n_ticks.max(1), Ordering::SeqCst);
    BOOT_TSC.store(t1, Ordering::SeqCst);
    TSC_READY.store(true, Ordering::SeqCst);
}

/// Pure ns conversion for a given cycles-per-interval ratio (host
/// tests exercise the arithmetic without touching the calibration).
pub fn cycles_to_ns_with(delta: u64, cycles_per_interval: u64) -> u64 {
    if cycles_per_interval == 0 {
        return 0;
    }
    ((delta as u128 * PIT_INTERVAL_NS as u128) / cycles_per_interval as u128) as u64
}

/// Cycle delta → nanoseconds through the calibrated ratio (u128 math;
/// reads are rare — charge points, syscalls — so the wide divide is
/// fine).
pub fn cycles_to_ns(delta: u64) -> u64 {
    cycles_to_ns_with(delta, CYCLES_PER_INTERVAL.load(Ordering::SeqCst))
}

/// The active source's rating (jiffies until calibrated, TSC after).
pub fn active_rating() -> u32 {
    if TSC_READY.load(Ordering::SeqCst) {
        TSC_RATING
    } else {
        JIFFIES_RATING
    }
}

/// Monotonic nanoseconds since boot from the best active source.
pub fn monotonic_ns() -> u64 {
    if TSC_READY.load(Ordering::SeqCst) {
        let base = BOOT_TSC.load(Ordering::SeqCst);
        let now = read_tsc();
        cycles_to_ns(now.saturating_sub(base))
    } else {
        // Jiffies fallback: 100 Hz PIT count scaled to ns.
        super::TICKS.load(Ordering::SeqCst) * PIT_INTERVAL_NS
    }
}

/// Wall-clock epoch base (ns): zero until an RTC lands (M8+ parity
/// list — M1R/M3 QEMU configs carry no RTC driver), so gettimeofday
/// reports the monotonic clock as wall time.
pub fn boot_epoch_ns() -> u64 {
    0
}

/// Wall-clock nanoseconds (epoch + monotonic).
pub fn wall_ns() -> u64 {
    boot_epoch_ns() + monotonic_ns()
}
