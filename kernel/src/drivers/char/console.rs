//! console: console input/output, ported from xv6 console.c + kbd.c
//! (+ uart.c's uartgetc/uartintr).
//!
//! Input is from the keyboard or COM1, buffered in the 128-byte CONS
//! ring; a read sleeps until a full line is available. Output goes
//! through crate::console::write_str — VGA (p2v(0xB8000), see vga.rs for
//! cgaputc's role) + serial in lockstep, with the xv6 BACKSPACE
//! erase sequence handled on both.
//!
//! File-layer wiring: conswrite/conswrite take raw buffers; the inode
//! plumbing of xv6's consolewrite/consoleread (iunlock/ilock) belongs to
//! the future fs layer, so the signatures are (buf, n) -> n.
//!
//! Proc-layer coupling: crate::kernel::sched's RawLock sleep/wakeup
//! (sleep(chan, &INPUT_LOCK) releases it exactly like xv6).

use crate::arch::x86::kernel::pic;
use crate::arch::x86::kernel::{IRQ_COM1, IRQ_KBD};
use crate::io::inb;
use crate::kernel::sched::{acquire, myproc, procdump, release, sleep, wakeup, RawLock};
// ---- kbd.h ---------------------------------------------------------------
const NO: u8 = 0;
#[allow(dead_code)] // kbd.h flag set; not all states are decoded yet
const SHIFT: u32 = 1 << 0;
const CTL: u32 = 1 << 1;
#[allow(dead_code)] // kbd.h flag set; not all states are decoded yet
const ALT: u32 = 1 << 2;
const CAPSLOCK: u32 = 1 << 3;
#[allow(dead_code)] // kbd.h flag set; not all states are decoded yet
const NUMLOCK: u32 = 1 << 4;
#[allow(dead_code)] // kbd.h flag set; not all states are decoded yet
const SCROLLLOCK: u32 = 1 << 5;
const E0ESC: u32 = 1 << 6;
const KBSTATP: u16 = 0x64; // kbd controller status port
const KBS_DIB: u8 = 0x01; // kbd data in buffer
const KBDATAP: u16 = 0x60; // kbd data port
/// C('A') == Control-A (kbd.h).
const fn ctrl(x: u8) -> u8 {
    x.wrapping_sub(b'@')
}

const BACKSPACE: i32 = 0x100;
const INPUT_BUF: usize = 128;

const CTRL_P: i32 = 0x10; // process listing
const CTRL_U: i32 = 0x15; // kill line
const CTRL_H: i32 = 0x08; // backspace
const CTRL_D: i32 = 0x04; // EOF

// ---- kbd.c tables (designated initializers -> sparse const builder) ------

/// Overlay `(code, key)` pairs on a zeroed (= NO) 256-entry map; this is
/// Rust's spelling of C's designated array initializers.
const fn map(entries: &[(usize, u8)]) -> [u8; 256] {
    let mut m = [NO; 256];
    let mut i = 0;
    while i < entries.len() {
        m[entries[i].0] = entries[i].1;
        i += 1;
    }
    m
}

#[rustfmt::skip]
const SHIFTCODE: [u8; 256] = map(&[
    (0x1D, 2), // CTL (1<<1)
    (0x2A, 1), // SHIFT (1<<0)
    (0x36, 1),
    (0x38, 4), // ALT (1<<2)
    (0x9D, 2),
    (0xB8, 4),
]);

#[rustfmt::skip]
const TOGGLECODE: [u8; 256] = map(&[
    (0x3A, 8),  // CAPSLOCK (1<<3)
    (0x45, 16), // NUMLOCK (1<<4)
    (0x46, 32), // SCROLLLOCK (1<<5)
]);

#[rustfmt::skip]
const NORMALMAP: [u8; 256] = map(&[
    (0x01, 0x1B), (0x02, b'1'), (0x03, b'2'), (0x04, b'3'),
    (0x05, b'4'), (0x06, b'5'), (0x07, b'6'), (0x08, b'7'),
    (0x09, b'8'), (0x0A, b'9'), (0x0B, b'0'), (0x0C, b'-'),
    (0x0D, b'='), (0x0E, 0x08), (0x0F, b'\t'), (0x10, b'q'),
    (0x11, b'w'), (0x12, b'e'), (0x13, b'r'), (0x14, b't'),
    (0x15, b'y'), (0x16, b'u'), (0x17, b'i'), (0x18, b'o'),
    (0x19, b'p'), (0x1A, b'['), (0x1B, b']'), (0x1C, b'\n'),
    (0x1E, b'a'), (0x1F, b's'), (0x20, b'd'), (0x21, b'f'),
    (0x22, b'g'), (0x23, b'h'), (0x24, b'j'), (0x25, b'k'),
    (0x26, b'l'), (0x27, b';'), (0x28, b'\''), (0x29, b'`'),
    (0x2B, b'\\'), (0x2C, b'z'), (0x2D, b'x'), (0x2E, b'c'),
    (0x2F, b'v'), (0x30, b'b'), (0x31, b'n'), (0x32, b'm'),
    (0x33, b','), (0x34, b'.'), (0x35, b'/'), (0x37, b'*'),
    (0x39, b' '), (0x47, b'7'), (0x48, b'8'), (0x49, b'9'),
    (0x4A, b'-'), (0x4B, b'4'), (0x4C, b'5'), (0x4D, b'6'),
    (0x4E, b'+'), (0x4F, b'1'), (0x50, b'2'), (0x51, b'3'),
    (0x52, b'0'), (0x53, b'.'),
    (0x9C, b'\n'), // KP_Enter
    (0xB5, b'/'),  // KP_Div
    (0xC8, 0xE2),  // KEY_UP
    (0xD0, 0xE3),  // KEY_DN
    (0xC9, 0xE6),  // KEY_PGUP
    (0xD1, 0xE7),  // KEY_PGDN
    (0xCB, 0xE4),  // KEY_LF
    (0xCD, 0xE5),  // KEY_RT
    (0x97, 0xE0),  // KEY_HOME
    (0xCF, 0xE1),  // KEY_END
    (0xD2, 0xE8),  // KEY_INS
    (0xD3, 0xE9),  // KEY_DEL
]);

#[rustfmt::skip]
const SHIFTMAP: [u8; 256] = map(&[
    (0x01, 0x1B), (0x02, b'!'), (0x03, b'@'), (0x04, b'#'),
    (0x05, b'$'), (0x06, b'%'), (0x07, b'^'), (0x08, b'&'),
    (0x09, b'*'), (0x0A, b'('), (0x0B, b')'), (0x0C, b'_'),
    (0x0D, b'+'), (0x0E, 0x08), (0x0F, b'\t'), (0x10, b'Q'),
    (0x11, b'W'), (0x12, b'E'), (0x13, b'R'), (0x14, b'T'),
    (0x15, b'Y'), (0x16, b'U'), (0x17, b'I'), (0x18, b'O'),
    (0x19, b'P'), (0x1A, b'{'), (0x1B, b'}'), (0x1C, b'\n'),
    (0x1E, b'A'), (0x1F, b'S'), (0x20, b'D'), (0x21, b'F'),
    (0x22, b'G'), (0x23, b'H'), (0x24, b'J'), (0x25, b'K'),
    (0x26, b'L'), (0x27, b':'), (0x28, b'"'), (0x29, b'~'),
    (0x2B, b'|'), (0x2C, b'Z'), (0x2D, b'X'), (0x2E, b'C'),
    (0x2F, b'V'), (0x30, b'B'), (0x31, b'N'), (0x32, b'M'),
    (0x33, b'<'), (0x34, b'>'), (0x35, b'?'), (0x37, b'*'),
    (0x39, b' '), (0x47, b'7'), (0x48, b'8'), (0x49, b'9'),
    (0x4A, b'-'), (0x4B, b'4'), (0x4C, b'5'), (0x4D, b'6'),
    (0x4E, b'+'), (0x4F, b'1'), (0x50, b'2'), (0x51, b'3'),
    (0x52, b'0'), (0x53, b'.'),
    (0x9C, b'\n'), // KP_Enter
    (0xB5, b'/'),  // KP_Div
    (0xC8, 0xE2), (0xD0, 0xE3), (0xC9, 0xE6), (0xD1, 0xE7),
    (0xCB, 0xE4), (0xCD, 0xE5), (0x97, 0xE0), (0xCF, 0xE1),
    (0xD2, 0xE8), (0xD3, 0xE9),
]);

#[rustfmt::skip]
const CTLMAP: [u8; 256] = map(&[
    (0x10, ctrl(b'Q')), (0x11, ctrl(b'W')), (0x12, ctrl(b'E')), (0x13, ctrl(b'R')),
    (0x14, ctrl(b'T')), (0x15, ctrl(b'Y')), (0x16, ctrl(b'U')), (0x17, ctrl(b'I')),
    (0x18, ctrl(b'O')), (0x19, ctrl(b'P')), (0x1C, b'\r'), (0x1E, ctrl(b'A')),
    (0x1F, ctrl(b'S')), (0x20, ctrl(b'D')), (0x21, ctrl(b'F')), (0x22, ctrl(b'G')),
    (0x23, ctrl(b'H')), (0x24, ctrl(b'J')), (0x25, ctrl(b'K')), (0x26, ctrl(b'L')),
    (0x2B, ctrl(b'\\')), (0x2C, ctrl(b'Z')), (0x2D, ctrl(b'X')), (0x2E, ctrl(b'C')),
    (0x2F, ctrl(b'V')), (0x30, ctrl(b'B')), (0x31, ctrl(b'N')), (0x32, ctrl(b'M')),
    (0x35, ctrl(b'/')),
    (0x9C, b'\r'),      // KP_Enter
    (0xB5, ctrl(b'/')), // KP_Div
    (0xC8, 0xE2), (0xD0, 0xE3), (0xC9, 0xE6), (0xD1, 0xE7),
    (0xCB, 0xE4), (0xCD, 0xE5), (0x97, 0xE0), (0xCF, 0xE1),
    (0xD2, 0xE8), (0xD3, 0xE9),
]);

const CHARCODE: [&[u8; 256]; 4] = [&NORMALMAP, &SHIFTMAP, &CTLMAP, &CTLMAP];

// ---- console.c input buffer (console.c:120) -------------------------------

/// The CONS ring: r = read index, w = write index, e = edit index.
struct Input {
    buf: [u8; INPUT_BUF],
    r: u32,
    w: u32,
    e: u32,
}

/// xv6 cons.lock / input.lock; RawLock because sleep() releases by ref.
static CONS_LOCK: RawLock = RawLock::new("console");
static INPUT_LOCK: RawLock = RawLock::new("input");
static mut INPUT: Input = Input {
    buf: [0; INPUT_BUF],
    r: 0,
    w: 0,
    e: 0,
};

/// xv6 sleeps/wakes on &input.r; we use the INPUT object's address.
fn input_chan() -> usize {
    core::ptr::addr_of!(INPUT) as usize
}

/// xv6 `proc->killed` test used by consoleread.
fn cur_killed() -> bool {
    // SAFETY: reads this CPU's proc slot + a plain i32 flag.
    let p = unsafe { myproc() };
    !p.is_null() && unsafe { (*p).killed } != 0
}

// ---- output ---------------------------------------------------------------

/// Emit one raw byte through both console sinks (VGA + COM1).
fn emit(byte: u8) {
    // Only ASCII is representable in &str; higher bytes degrade to '?'.
    if byte < 0x80 {
        let buf = [byte];
        // SAFETY: bytes < 0x80 are valid UTF-8.
        if let Ok(s) = core::str::from_utf8(&buf) {
            crate::console::write_str(s);
        }
    } else {
        crate::console::write_str("?");
    }
}

/// consputc (console.c:99): BACKSPACE erases ("\b \b" on both sinks — the
/// role of uartputc('\b')+' '+'\b' plus cgaputc's pos--); anything else
/// is echoed as-is.
pub fn consputc(c: i32) {
    if c == BACKSPACE {
        crate::console::write_str("\x08 \x08");
    } else {
        emit(c as u8);
    }
}

// ---- interrupt-side input (console.c:130) ----------------------------------

/// consoleintr (console.c:130): drain `getc` into the edit buffer,
/// handling ^P (procdump), ^U (kill line), backspace, and line commit.
pub fn consoleintr(getc: fn() -> i32) {
    // SAFETY: xv6 lock discipline; the ring is only touched under
    // INPUT_LOCK (or single-threaded boot before interrupts are on).
    unsafe {
        acquire(&INPUT_LOCK);
        loop {
            let c = getc();
            if c < 0 {
                break;
            }
            let inp = core::ptr::addr_of_mut!(INPUT);
            if c == CTRL_P {
                // Process listing.
                procdump();
            } else if c == CTRL_U {
                // Kill line.
                while (*inp).e != (*inp).w
                    && (*inp).buf[((*inp).e as usize - 1) % INPUT_BUF] != b'\n'
                {
                    (*inp).e -= 1;
                    consputc(BACKSPACE);
                }
            } else if c == CTRL_H || c == 0x7f {
                // Backspace.
                if (*inp).e != (*inp).w {
                    (*inp).e -= 1;
                    consputc(BACKSPACE);
                }
            } else if c != 0 && (*inp).e - (*inp).r < INPUT_BUF as u32 {
                let mut c = c as u8;
                if c == b'\r' {
                    c = b'\n';
                }
                (*inp).buf[(*inp).e as usize % INPUT_BUF] = c;
                (*inp).e += 1;
                consputc(c as i32);
                if c == b'\n' || c == CTRL_D as u8 || (*inp).e == (*inp).r + INPUT_BUF as u32 {
                    (*inp).w = (*inp).e;
                    wakeup(input_chan());
                }
            }
        }
        release(&INPUT_LOCK);
    }
}

// ---- kbd.c / uart.c producers ----------------------------------------------

/// kbdgetc (kbd.c:8): read one character from the keyboard, or -1.
pub fn kbdgetc() -> i32 {
    static mut SHIFT: u32 = 0; // kbd.c's static shift state
                               // SAFETY: SHIFT is only touched from consoleintr call paths, which
                               // are serialized by INPUT_LOCK; ports are read-only probes.
    unsafe {
        let st = inb(KBSTATP);
        if st & KBS_DIB == 0 {
            return -1;
        }
        let mut data = inb(KBDATAP) as u32;

        if data == 0xE0 {
            SHIFT |= E0ESC;
            return 0;
        } else if data & 0x80 != 0 {
            // Key released.
            data = if SHIFT & E0ESC != 0 {
                data
            } else {
                data & 0x7F
            };
            SHIFT &= !(SHIFTCODE[data as usize] as u32 | E0ESC);
            return 0;
        } else if SHIFT & E0ESC != 0 {
            // Last character was an E0 escape; or with 0x80.
            data |= 0x80;
            SHIFT &= !E0ESC;
        }

        SHIFT |= SHIFTCODE[data as usize] as u32;
        SHIFT ^= TOGGLECODE[data as usize] as u32;
        let mut c = CHARCODE[(SHIFT & (CTL | SHIFT)) as usize][data as usize] as i32;
        if SHIFT & CAPSLOCK != 0 {
            if c >= 'a' as i32 && c <= 'z' as i32 {
                c += 'A' as i32 - 'a' as i32;
            } else if c >= 'A' as i32 && c <= 'Z' as i32 {
                c += 'a' as i32 - 'A' as i32;
            }
        }
        c
    }
}

/// kbdintr (kbd.c:46).
pub fn kbdintr() {
    consoleintr(kbdgetc);
}

const COM1: u16 = 0x3F8;

/// uartgetc (uart.c:64): read one character from COM1, or -1.
pub fn uartgetc() -> i32 {
    if inb(COM1 + 5) & 0x01 == 0 {
        return -1;
    }
    inb(COM1 + 0) as i32
}

/// uartintr (uart.c:74).
pub fn uartintr() {
    let _ = inb(COM1 + 5);
    let _ = inb(COM1 + 2);
    consoleintr(uartgetc);
}

// ---- file-layer entry points -----------------------------------------------

/// consoleread (console.c:222): read one line from the CONS buffer into
/// `dst`; sleeps (releasing the input lock) until a line is complete or
/// ^D arrives.
pub fn consread(dst: *mut u8, n: i32) -> i32 {
    let target = n;
    let mut n = n;
    let mut dst = dst;
    // SAFETY: xv6 lock discipline + caller-granted n writable bytes.
    unsafe {
        acquire(&INPUT_LOCK);
        while n > 0 {
            while INPUT.r == INPUT.w {
                if cur_killed() {
                    release(&INPUT_LOCK);
                    return -1;
                }
                sleep(input_chan(), &INPUT_LOCK);
            }
            let c = INPUT.buf[INPUT.r as usize % INPUT_BUF];
            INPUT.r += 1;
            if c == CTRL_D as u8 {
                // EOF: save ^D for next time so the caller can get a
                // 0-byte result.
                if n < target {
                    INPUT.r -= 1;
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
        release(&INPUT_LOCK);
    }
    target - n
}

/// consolewrite (console.c:262): echo `n` bytes from `buf` under cons.lock.
pub fn conswrite(buf: *const u8, n: i32) -> i32 {
    // SAFETY: caller granted n readable bytes at buf; xv6 lock discipline.
    unsafe {
        acquire(&CONS_LOCK);
        let mut i = 0;
        while i < n {
            consputc(*buf.add(i as usize) as i32);
            i += 1;
        }
        release(&CONS_LOCK);
    }
    n
}

/// consoleinit (console.c:273): the locks are compile-time statics here;
/// unmask the keyboard and COM1 interrupts on the PIC (xv6 also routes
/// them through the IOAPIC, which minios does not have).
pub fn consoleinit() {
    pic::picenable(IRQ_KBD);
    pic::picenable(IRQ_COM1);
}
