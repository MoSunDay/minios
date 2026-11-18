//! Write-based output helpers (plans/linux26/04 §3): serial fd 1/2 only;
//! no std formatting machinery. Number rendering is plain decimal/hex.

use crate::sys;

pub const STDOUT: i32 = 1;
pub const STDERR: i32 = 2;

/// Best-effort write of the whole buffer (used for test markers, where a
/// short write must fail the case rather than silently truncate).
pub fn write_all(fd: i32, buf: &[u8]) -> bool {
    match sys::write(fd, buf) {
        Ok(n) => n == buf.len(),
        Err(_) => false,
    }
}

pub fn print_str(text: &str) -> bool {
    write_all(STDOUT, text.as_bytes())
}

pub fn eprint_str(text: &str) -> bool {
    write_all(STDERR, text.as_bytes())
}

pub fn print_uint(value: u64) -> bool {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    let mut v = value;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    write_all(STDOUT, &digits[i..])
}

pub fn eprint_dec(fd: i32, value: u64) -> bool {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    let mut v = value;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    write_all(fd, &digits[i..])
}

pub fn print_hex(value: u64) -> bool {
    let mut digits = [0u8; 16];
    let mut i = digits.len();
    let mut v = value;
    loop {
        i -= 1;
        let d = (v & 0xf) as u8;
        digits[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        v >>= 4;
        if v == 0 {
            break;
        }
    }
    write_all(STDOUT, b"0x") && write_all(STDOUT, &digits[i..])
}

/// gets() the xv6 way (ulib.c): read one byte at a time from `fd` into
/// `buf` until newline or EOF; NUL-terminates (newline kept, like xv6).
/// Returns the line length without the NUL, or None on immediate EOF.
pub fn gets(fd: i32, buf: &mut [u8]) -> Option<usize> {
    let mut n = 0usize;
    while n < buf.len() - 1 {
        let mut one = [0u8; 1];
        match sys::read(fd, &mut one) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => return None,
        }
        buf[n] = one[0];
        n += 1;
        if one[0] == b'\n' {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    buf[n] = 0;
    Some(n)
}

/// Signed decimal print to `fd` (printf %d).
pub fn print_i64_fd(fd: i32, value: i64) -> bool {
    let (neg, mag) = if value < 0 {
        (true, (value as i128).unsigned_abs() as u128)
    } else {
        (false, value as u128)
    };
    let mut digits = [0u8; 21];
    let mut i = digits.len();
    let mut v = mag;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        digits[i] = b'-';
    }
    write_all(fd, &digits[i..])
}
