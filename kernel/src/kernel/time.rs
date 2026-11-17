//! PIT/PIC timer: jiffies and preemption tick (UP, plans/linux26/21 §1).

use core::sync::atomic::{AtomicU64, Ordering};

use super::traps::{Trapframe, IRQ_TIMER};
use crate::arch::x86_64::io;

pub static TICKS: AtomicU64 = AtomicU64::new(0);
/// Total timer ticks observed; also the sleep/wakeup channel.
#[allow(dead_code)] // S2 sleep/wakeup channel
pub fn ticks() -> u64 {
    TICKS.load(Ordering::SeqCst)
}

/// Program PIT channel 0 at 100 Hz (1193182 / 100).
pub fn init_pit() {
    const PIT_CH0: u16 = 0x40;
    const PIT_CMD: u16 = 0x43;
    const DIVISOR: u32 = 1193182 / 100;
    io::out8(PIT_CMD, 0x36); // channel 0, lobyte/hibyte, mode 3
    io::out8(PIT_CH0, (DIVISOR & 0xFF) as u8);
    io::out8(PIT_CH0, (DIVISOR >> 8) as u8);
}

pub fn tick(tf: &mut Trapframe) {
    crate::drivers::pic::eoi((IRQ_TIMER) as u8);
    let now = TICKS.fetch_add(1, Ordering::SeqCst) + 1;
    super::syscall::on_tick(now);
    // Wake sleepers and preempt user contexts (xv6 discipline).
    crate::proc::sleep::wakeup_ticks(now);
    if tf.from_user() {
        crate::proc::sched::yield_cpu();
    }
}
