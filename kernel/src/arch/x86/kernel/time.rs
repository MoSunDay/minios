//! time: the tick source, ported from xv6 timer.c (PIT) with trap.c's
//! tick bookkeeping.
//!
//! PLAN-APPROVED DEVIATION from xv6-lapic: instead of programming the
//! local APIC timer (lapic.c), we drive the classic 8253/8254 PIT on
//! channel 0 — port 0x40 data, port 0x43 command — in mode 3 (square
//! wave) with divisor 1193182/100, i.e. 100 Hz. The T_IRQ0+0 interrupt
//! arrives through the PIC, so the EOI goes to pic.rs instead of
//! lapiceoi(). `ticks`/`tickslock` (trap.c:13) live here as
//! TICKS/TICKS_LOCK for the proc layer (sys_sleep wakes on &TICKS).

use core::ptr::addr_of_mut;

use crate::arch::x86::kernel::{pic, IRQ_TIMER};
use crate::io::outb;
use crate::kernel::sched::{acquire, release, wakeup, RawLock};

const IO_TIMER1: u16 = 0x040; // 8253 Timer #1 (channel 0 data port)
const TIMER_FREQ: u32 = 1193182; // all three counters count at this rate
const TIMER_DIV: u32 = TIMER_FREQ / 100; // 100 Hz -> 11932
const TIMER_MODE: u16 = IO_TIMER1 + 3; // 0x43: timer mode port
const TIMER_SEL0: u8 = 0x00; // select counter 0
const TIMER_SQWAVE: u8 = 0x06; // mode 3, square wave (xv6 timer.c uses mode 2)
const TIMER_16BIT: u8 = 0x30; // r/w counter 16 bits, LSB first

/// xv6 trap.c:13 `uint ticks`.
pub static mut TICKS: u32 = 0;
/// xv6 trap.c:12 `tickslock` ("time").
pub static TICKS_LOCK: RawLock = RawLock::new("time");

/// The wait channel sleepers park on: xv6 wakeup(&ticks).
pub fn ticks_chan() -> usize {
    addr_of_mut!(TICKS) as usize
}

/// Snapshot of the global tick counter (sys_sleep).
pub fn ticks() -> u32 {
    // SAFETY: read under tickslock like xv6.
    unsafe {
        acquire(&TICKS_LOCK);
        let t = TICKS;
        release(&TICKS_LOCK);
        t
    }
}

/// timerinit (timer.c:26 + lapic.c timerinit): program the PIT to
/// interrupt 100 times a second and unmask IRQ_TIMER on the PIC.
pub fn timerinit() {
    outb(TIMER_MODE, TIMER_SEL0 | TIMER_SQWAVE | TIMER_16BIT);
    outb(IO_TIMER1, (TIMER_DIV % 256) as u8);
    outb(IO_TIMER1, (TIMER_DIV / 256) as u8);
    pic::picenable(IRQ_TIMER);
}

/// timerintr (trap.c:46, lapiceoi replaced by the PIC EOI): bump the tick
/// counter, wake everything sleeping on &ticks, acknowledge the PIC.
pub fn timerintr() {
    // SAFETY: xv6 lock discipline: ticks guarded by tickslock, wakeup
    // called while holding it.
    unsafe {
        acquire(&TICKS_LOCK);
        TICKS += 1;
        wakeup(ticks_chan());
        release(&TICKS_LOCK);
    }
    pic::eoi(IRQ_TIMER);
}
