//! Host-side stand-in for the kernel's `kernel::time` module: the real
//! clocksource/hrtimer/wheel sources are included by path (single
//! source of truth, no test copies). `TICKS` is a host stand-in for the
//! kernel's jiffies counter — no timer interrupt advances it here, so
//! tests drive wheel_tick/run_expired with explicit `now` values.

use core::sync::atomic::AtomicU64;

pub static TICKS: AtomicU64 = AtomicU64::new(0);

#[path = "../../kernel/src/kernel/time/clocksource.rs"]
pub mod clocksource;
#[path = "../../kernel/src/kernel/time/hrtimer.rs"]
pub mod hrtimer;
#[path = "../../kernel/src/kernel/time/wheel.rs"]
pub mod wheel;
