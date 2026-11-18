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
