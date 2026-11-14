//! printk: kernel printf, ported from xv6 printf.c (user) + console.c's
//! printint. Understands %d %u %x %s %c %p, output goes to
//! crate::console::write_str (VGA + COM1 sinks).
//!
//! Rust has no varargs, so arguments are passed as a slice of `Arg`; the
//! `printk!` macro below gives the ergonomic xv6 `cprintf` call shape.

use crate::console::write_str;

/// One printf argument (stand-in for a varargs slot).
#[derive(Clone, Copy)]
pub enum Arg {
    /// %d — signed decimal
    D(i32),
    /// %u — unsigned decimal
    #[allow(dead_code)] // printf.c state machine supports it; no caller yet
    U(u32),
    /// %x — unsigned hexadecimal
    X(u32),
    /// %p — pointer-sized value, printed as plain hex like xv6
    P(usize),
    /// %s — NUL-free string slice (NULL prints "(null)" like xv6)
    S(Option<&'static str>),
    /// %c — single character
    #[allow(dead_code)] // printf.c state machine supports it; no caller yet
    C(u8),
}

/// Emit one raw byte through the console sinks.
fn putc(c: u8) {
    // Only ASCII is representable in &str; higher bytes degrade to '?'.
    if c < 0x80 {
        let buf = [c];
        // SAFETY: bytes < 0x80 are valid UTF-8.
        if let Ok(s) = core::str::from_utf8(&buf) {
            write_str(s);
        }
    } else {
        write_str("?");
    }
}

/// printint: print `xx` in `base`, with '-' when `sgn` and negative
/// (printf.c:11; digit table lower-case like console.c:35).
pub fn printint(xx: i32, base: u32, sgn: bool) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 32]; // 32 digits covers base 2 of u32
    let mut i = 0usize;
    let neg = sgn && xx < 0;
    let mut x = if neg { (-xx) as u32 } else { xx as u32 };

    loop {
        buf[i] = DIGITS[(x % base) as usize];
        i += 1;
        x /= base;
        if x == 0 {
            break;
        }
    }
    if neg {
        buf[i] = b'-';
        i += 1;
    }
    while i > 0 {
        i -= 1;
        putc(buf[i]);
    }
}

/// printf: interpret `fmt` against `args` exactly like printf.c's state
/// machine (state 0 / state '%'); unknown conversions print themselves to
/// draw attention.
pub fn printf(fmt: &str, args: &[Arg]) {
    let mut argi = 0usize;
    let bytes = fmt.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'%' {
            putc(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }
        let c = bytes[i];
        i += 1;
        let mut next = || -> Arg {
            let a = if argi < args.len() {
                args[argi]
            } else {
                Arg::D(0)
            };
            argi += 1;
            a
        };
        match c {
            b'd' => {
                if let Arg::D(v) = next() {
                    printint(v, 10, true);
                }
            }
            b'u' => {
                let v = match next() {
                    Arg::U(v) => v,
                    Arg::D(v) => v as u32,
                    other => other.as_u32_or(0),
                };
                printint(v as i32, 10, false);
            }
            b'x' => {
                let v = match next() {
                    Arg::X(v) => v,
                    Arg::D(v) => v as u32,
                    other => other.as_u32_or(0),
                };
                printint(v as i32, 16, false);
            }
            b'p' => {
                let v = match next() {
                    Arg::P(v) => v,
                    other => other.as_u32_or(0) as usize,
                };
                printint(v as i32, 16, false); // xv6 prints %p as plain hex
            }
            b's' => {
                if let Arg::S(s) = next() {
                    match s {
                        Some(s) => write_str(s),
                        None => write_str("(null)"),
                    }
                }
            }
            b'c' => {
                if let Arg::C(ch) = next() {
                    putc(ch);
                }
            }
            b'%' => putc(b'%'),
            _ => {
                // Unknown % sequence: print it to draw attention.
                putc(b'%');
                putc(c);
            }
        }
    }
}

impl Arg {
    /// Loose numeric fallback for conversions fed a mismatched slot.
    fn as_u32_or(self, default: u32) -> u32 {
        match self {
            Arg::D(v) => v as u32,
            Arg::U(v) => v,
            Arg::X(v) => v,
            Arg::P(v) => v as u32,
            _ => default,
        }
    }
}

/// cprintf-style formatting: `printk!("pid %d\n", Arg::D(3))`.
#[macro_export]
macro_rules! printk {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::lib::printk::printf($fmt, &[$($arg),*])
    };
}
