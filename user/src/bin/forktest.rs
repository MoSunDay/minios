//! forktest(1), xv6 forktest.c: fork until the process table is full,
//! then reap every child and require the final wait to fail with ECHILD.

#![no_std]
#![no_main]

use minios_user::{io, sys};

/// This kernel's process table size (xv6 NPROC).
const N: usize = 64;

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let _ = io::print_str("forktest\n");
    let mut n: usize = 0;
    while n < N {
        match sys::fork() {
            Ok(0) => sys::exit(0),
            Ok(_) => n += 1,
            Err(_) => break, // table full, like xv6's -1
        }
    }
    if n == N {
        let _ = io::eprint_str("forktest: fork claimed to work 64 times!\n");
        sys::exit(1);
    }
    let mut pid = sys::wait4(-1, core::ptr::null_mut());
    let mut reaped = 0usize;
    while let Ok(p) = pid {
        if p <= 0 {
            break;
        }
        reaped += 1;
        pid = sys::wait4(-1, core::ptr::null_mut());
    }
    if reaped != n {
        let _ = io::eprint_str("forktest: wait got too many\n");
        sys::exit(1);
    }
    // The table is empty again: the last wait must fail with ECHILD.
    match sys::wait4(-1, core::ptr::null_mut()) {
        Err(e) if e == abi::ECHILD => sys::exit(0),
        _ => {
            let _ = io::eprint_str("forktest: wait got too many\n");
            sys::exit(1)
        }
    }
}
