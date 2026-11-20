//! Time subsystem (21 §4): clocksource (TSC calibrated over PIT, with a
//! jiffies fallback), clock-event device (PIT periodic, the UP
//! reference; one-shot waits for the M8 LAPIC), hrtimers over the
//! monotonic clock, and the jiffies timer wheel as the low-resolution
//! compat interface. nanosleep/alarm/gettimeofday sit on this common
//! layer; the periodic tick still drives scheduler quanta and expiry.

pub mod clockevents;
pub mod clocksource;
pub mod hrtimer;
pub mod wheel;

use core::sync::atomic::{AtomicU64, Ordering};

use super::traps::{Trapframe, IRQ_BASE, IRQ_TIMER};
use crate::arch::x86_64::io;

pub static TICKS: AtomicU64 = AtomicU64::new(0);
/// Serializes TICKS reads against the tick interrupt for sleepers
/// (xv6 tickslock; the sleep channel is the TICKS address).
pub static TICK_LOCK: Spinlock<()> = Spinlock::new("time", ());

use crate::klib::sync::Spinlock;

/// Sleep channel every nanosleep-style sleeper parks on.
pub fn ticks_chan() -> usize {
    core::ptr::addr_of!(TICKS) as usize
}

/// Total timer ticks observed (jiffies).
pub fn ticks() -> u64 {
    TICKS.load(Ordering::SeqCst)
}

/// Legacy PIT programming kept verbatim (clockevents::init_periodic
/// wraps it for the device abstraction).
pub fn legacy_init_pit() {
    const PIT_CH0: u16 = 0x40;
    const PIT_CMD: u16 = 0x43;
    const DIVISOR: u32 = 1193182 / 100;
    io::out8(PIT_CMD, 0x36); // channel 0, lobyte/hibyte, mode 3
    io::out8(PIT_CH0, (DIVISOR & 0xFF) as u8);
    io::out8(PIT_CH0, (DIVISOR >> 8) as u8);
}

/// Boot-time time bring-up: start the PIT periodic event device, open
/// its IRQ line (pic::init masked everything and the calibration wait
/// below needs tick delivery), then calibrate the TSC against it
/// (~50 ms busy wait).
pub fn init_time() {
    clockevents::init_periodic();
    crate::drivers::pic::unmask((IRQ_TIMER - IRQ_BASE) as u8);
    clocksource::calibrate_tsc(5);
}

/// Timer interrupt: jiffies++, expire hrtimers and wheel timers, wake
/// tick-channel sleepers, and preempt user contexts.
pub fn tick(tf: &mut Trapframe) {
    crate::drivers::pic::eoi((IRQ_TIMER) as u8);
    let now = TICKS.fetch_add(1, Ordering::SeqCst) + 1;
    super::syscall::on_tick(now);
    // Monotonic now for expiry dispatch (jiffies may lag the clocksource
    // by up to one period; hrtimers compare absolute ns).
    let now_ns = clocksource::monotonic_ns();
    // SAFETY: timer-interrupt context; callbacks wake sleepers only.
    unsafe {
        hrtimer::run_expired(now_ns);
        wheel::wheel_tick(now);
    }
    // Wake nanosleep sleepers (they re-check their deadline under
    // TICK_LOCK; xv6 wakeup(&ticks)) and preempt user contexts.
    // SAFETY: plain lock acquire/release in interrupt context.
    unsafe { TICK_LOCK.raw_lock() };
    crate::proc::sleep::wakeup(ticks_chan());
    unsafe { TICK_LOCK.raw_release() };
    crate::proc::sleep::wakeup_ticks(now);
    if tf.from_user() {
        crate::proc::sched::entity_tick();
    }
}
