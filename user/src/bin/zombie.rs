//! zombie(1), xv6 zombie.c: the parent exits at once; the child naps so
//! it is briefly an orphan (reparented to init) before it exits.

#![no_std]
#![no_main]

use minios_user::sys;

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    if let Ok(pid) = sys::fork() {
        if pid != 0 {
            // Parent: leave the child behind as an orphan-to-be.
            sys::exit(0);
        }
    }
    let _ = sys::sleep(5);
    sys::exit(0)
}
