//! kill(1), xv6 kill.c: kill(2) each pid argument.

#![no_std]
#![no_main]

use abi::sig::SIGKILL;
use minios_user::{args, io, sys};

fn atoi(s: &[u8]) -> i32 {
    let mut i = 0usize;
    let mut sign = 1i32;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        if s[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let mut n = 0i32;
    while i < s.len() && s[i].is_ascii_digit() {
        n = n.wrapping_mul(10).wrapping_add((s[i] - b'0') as i32);
        i += 1;
    }
    sign.wrapping_mul(n)
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() < 2 {
        let _ = io::eprint_str("usage: kill pid...\n");
        sys::exit(1);
    }
    for i in 1..a.argc() {
        if let Some(p) = a.arg(i) {
            let pid = atoi(p);
            if sys::kill(pid, SIGKILL).is_err() {
                let _ = io::eprint_str("kill: exit status error\n");
                sys::exit(1);
            }
        }
    }
    sys::exit(0)
}
