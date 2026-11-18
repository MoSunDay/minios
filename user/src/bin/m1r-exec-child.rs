//! execve target for the M1R user suite: proving a fresh static ET_EXEC
//! image replaced the caller. Prints one marker line and exits 42.

#![no_std]
#![no_main]

use minios_user::{io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let ok = io::print_str("m1r-exec-child: replaced image\n");
    sys::exit(if ok { 42 } else { 1 })
}
