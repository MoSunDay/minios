//! rm(1), xv6 rm.c: unlink(2) each argument.

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() < 2 {
        let _ = io::eprint_str("usage: rm file...\n");
        sys::exit(1);
    }
    for i in 1..a.argc() {
        if let Some(p) = a.arg(i) {
            if sys::unlink(p).is_err() {
                let _ = io::eprint_str("rm: ");
                let _ = io::write_all(io::STDERR, p);
                let _ = io::eprint_str(" failed\n");
                sys::exit(1);
            }
        }
    }
    sys::exit(0)
}
