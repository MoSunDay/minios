//! echo(1), xv6 echo.c: print each argument separated by single spaces.

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    for i in 1..a.argc() {
        if let Some(arg) = a.arg(i) {
            let _ = io::write_all(io::STDOUT, arg);
            if i + 1 < a.argc() {
                let _ = io::print_str(" ");
            }
        }
    }
    let _ = io::print_str("\n");
    sys::exit(0)
}
