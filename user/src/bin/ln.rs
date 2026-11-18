//! ln(1), xv6 ln.c: hard-link old to new.

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() != 3 {
        let _ = io::eprint_str("usage: ln old new\n");
        sys::exit(1);
    }
    let (old, new) = (a.arg(1), a.arg(2));
    if let (Some(o), Some(n)) = (old, new) {
        if sys::link(o, n).is_ok() {
            sys::exit(0);
        }
        let _ = io::eprint_str("ln: ");
        let _ = io::write_all(io::STDERR, o);
        let _ = io::eprint_str(" ");
        let _ = io::write_all(io::STDERR, n);
        let _ = io::eprint_str(" failed\n");
    }
    sys::exit(1)
}
