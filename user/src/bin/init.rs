//! init(1), xv6 init.c: the first user program. Owns the console (mknod
//! on first boot), then runs sh forever, reaping every orphan child.

#![no_std]
#![no_main]

use minios_user::{io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Console seeding exactly like xv6 init.c: open, mknod if missing.
    let console = b"/console\0";
    let fd0 = match sys::open(console, abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => {
            let _ = sys::mknod(console, 1, 1);
            match sys::open(console, abi::O_RDWR) {
                Ok(fd) => fd as i32,
                Err(_) => sys::exit(1),
            }
        }
    };
    // pid 1 owns an empty descriptor table: the console must be fd 0.
    if fd0 != 0 {
        sys::exit(1);
    }
    if sys::dup(0).is_err() || sys::dup(0).is_err() {
        sys::exit(1);
    }

    // Run sh, reap everything, restart sh when it dies.
    let sh = b"/sh\0";
    loop {
        let _ = io::print_str("init: starting sh\n");
        match sys::fork() {
            Ok(0) => {
                let argv: [*const u8; 2] = [sh.as_ptr(), core::ptr::null()];
                match sys::execve(sh, argv.as_ptr() as *const usize, core::ptr::null()) {
                    _ => sys::exit(1), // successful exec never returns
                }
            }
            Ok(_) => {}
            Err(_) => sys::exit(1),
        }
        loop {
            match sys::wait4(-1, core::ptr::null_mut()) {
                Ok(_) => continue, // reaped a child (sh or an orphan)
                Err(_) => break,   // ECHILD: sh died, restart it
            }
        }
        let _ = io::print_str("init: sh exited, restarting\n");
    }
}
