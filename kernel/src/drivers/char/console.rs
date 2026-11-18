//! Console device (xv6 console.c + uart.c receive side, M1R cut): the
//! devsw major-1 driver behind T_DEV inodes opened on /console.
//!
//! Output: cons_write routes each byte through arch console::write_byte
//! (VGA + COM1, with the xv6 CRLF expansion).
//!
//! Input: COM1 RX interrupts feed a 128-byte line-discipline ring; a read
//! sleeps until a line (or EOF at line start) is available. The PS/2
//! keyboard half of xv6 console.c/kbd.c is intentionally out of scope for
//! M1R (headless QEMU runs have no keyboard; -display none).
//!
//! File-layer wiring: readi/writei dispatch here via
//! fs::xv6fs::devsw for T_DEV inodes, so the signatures match DevReadFn /
//! DevWriteFn (kernel buffers; the sys layer bounced user memory already).

use crate::arch::x86_64::io::{in8, out8};
use crate::fs::xv6fs::devsw::register_dev;
use crate::fs::xv6fs::inode::Inode;
use crate::fs::xv6fs::layout::CONSOLE;
use crate::klib::sync::Spinlock;
use crate::proc::proc::myproc;
use crate::proc::sleep::{sleep, wakeup};

const COM1: u16 = 0x3f8;

const INPUT_BUF: usize = 128;
/// C('U') == 0x15: kill line. C('D') == 0x04: EOF. C('H') == 0x08: erase.
const CTRL_U: u8 = 0x15;
const CTRL_D: u8 = 0x04;
const CTRL_H: u8 = 0x08;

/// console.c `struct { ... } input` (the lock is a sibling static, like
/// the icache/ftable splits).
struct Input {
    buf: [u8; INPUT_BUF],
    /// Read index (console.c r).
    r: u32,
    /// Write index (console.c w).
    w: u32,
}

// SAFETY: guarded by INPUT_LOCK; unique statics.
static INPUT_LOCK: Spinlock<()> = Spinlock::new("console", ());
static mut INPUT: Input = Input {
    buf: [0; INPUT_BUF],
    r: 0,
    w: 0,
};

/// Sleeping channel: new input arrived (xv6 sleeps on &input.r).
// SAFETY: address-of a unique static; taken for the sleep channel only.
unsafe fn input_r_chan() -> usize {
    core::ptr::addr_of!(INPUT.r) as usize
}

/// console.c consoleinit(): register the device and enable RX interrupts.
pub fn consoleinit() {
    // SAFETY: boot-time registration before any T_DEV read/write.
    unsafe {
        register_dev(CONSOLE, Some(cons_read), Some(cons_write));
    }
    // Enable UART received-data interrupts (uart.c: outb(...,IER,0x01))
    // and open the PIC line.
    out8(COM1 + 1, 0x01);
    // COM1 is ISA IRQ 4 (routed at traps.rs IRQ_COM1).
    crate::drivers::pic::unmask(4);
}

/// UART receive: drain every ready byte into the ring (uart.c uartintr ->
/// consoleintr, inlined to the one interrupt source we keep).
pub fn consoleintr() {
    // SAFETY: INPUT_LOCK guards the ring indices.
    unsafe {
        while in8(COM1 + 5) & 0x01 != 0 {
            let c = in8(COM1);
            INPUT_LOCK.raw_lock();
            let inp = core::ptr::addr_of_mut!(INPUT);
            match c {
                CTRL_H | b'\x7f' => {
                    // Backspace: erase the last buffered byte.
                    if (*inp).w != (*inp).r {
                        (*inp).w -= 1;
                    }
                }
                CTRL_U => {
                    // Kill line.
                    (*inp).w = (*inp).r;
                }
                c => {
                    if ((*inp).w - (*inp).r) < INPUT_BUF as u32 {
                        (*inp).buf[((*inp).w % INPUT_BUF as u32) as usize] = c;
                        (*inp).w += 1;
                    }
                }
            }
            wakeup(input_r_chan());
            INPUT_LOCK.raw_release();
        }
    }
}

/// Read one line (or EOF) from the console into a KERNEL buffer
/// (console.c consoleread).
///
/// SAFETY: process context (sleeps); `dst` valid for `n` bytes.
pub unsafe fn cons_read(_ip: *mut Inode, dst: *mut u8, n: i32) -> i32 {
    let target = n;
    let mut n = n;
    let mut dst = dst;
    // SAFETY: INPUT_LOCK held across the loop; sleep() releases it while
    // waiting and hands it back on wake.
    unsafe {
        INPUT_LOCK.raw_lock();
        let me = myproc();
        while n > 0 {
            let inp = core::ptr::addr_of_mut!(INPUT);
            while (*inp).r == (*inp).w {
                if !me.is_null() && (*me).killed != 0 {
                    INPUT_LOCK.raw_release();
                    return -1;
                }
                sleep(input_r_chan(), &INPUT_LOCK);
            }
            let c = (*inp).buf[((*inp).r % INPUT_BUF as u32) as usize];
            (*inp).r += 1;
            if c == CTRL_D {
                // EOF: put the byte back unless it ends an empty read.
                if n < target {
                    (*inp).r -= 1;
                }
                break;
            }
            *dst = c;
            dst = dst.add(1);
            n -= 1;
            if c == b'\n' {
                break;
            }
        }
        INPUT_LOCK.raw_release();
    }
    target - n
}

/// Write `n` bytes from a KERNEL buffer to the console (console.c
/// consolewrite).
///
/// SAFETY: `src` valid for `n` bytes.
pub unsafe fn cons_write(_ip: *mut Inode, src: *const u8, n: i32) -> i32 {
    let mut i: i32 = 0;
    unsafe {
        while i < n {
            crate::arch::x86_64::console::write_byte(*src.add(i as usize));
            i += 1;
        }
    }
    n
}
