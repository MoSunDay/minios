//! usertests(1), the xv6 usertests port (S4e-2): pid 1 in the `m1r-user`
//! boot. Console seeding follows xv6 init.c (open /console, mknod on the
//! first boot, dup onto stdin/stdout/stderr), then the utest battery
//! prints its CASE lines and this driver exits with (total<<8)|failed
//! for the kernel verdict merge.

#![no_std]
#![no_main]

use minios_user::{sys, utest};

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // xv6 init.c: the first process owns the console.
    let console = b"/console\0";
    let fd0 = match sys::open(console, abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => {
            if sys::mknod(console, 1, 1).is_err() {
                sys::exit(-1);
            }
            match sys::open(console, abi::O_RDWR) {
                Ok(fd) => fd as i32,
                Err(_) => sys::exit(-1),
            }
        }
    };
    if fd0 != 0 {
        let _ = sys::close(fd0);
        sys::exit(-1);
    }
    if sys::dup(0).is_err() || sys::dup(0).is_err() {
        sys::exit(-1);
    }

    let counts = utest::run_all();
    sys::exit(((counts.total << 8) | counts.failed) as i32)
}
