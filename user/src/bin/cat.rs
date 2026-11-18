//! cat(1), xv6 cat.c: concatenate files (or stdin) to stdout.

#![no_std]
#![no_main]

use minios_user::{io, sys};

fn cat(fd: i32) -> bool {
    let mut buf = [0u8; 512];
    loop {
        match sys::read(fd, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if !io::write_all(io::STDOUT, &buf[..n]) {
                    return false;
                }
            }
        }
    }
    true
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { minios_user::args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() <= 1 {
        cat(0);
        sys::exit(0);
    }
    for i in 1..a.argc() {
        let path = match a.arg(i) {
            Some(p) => p,
            None => continue,
        };
        match sys::open(path, abi::O_RDONLY) {
            Ok(fd) => {
                cat(fd as i32);
                let _ = sys::close(fd as i32);
            }
            Err(_) => {
                let _ = io::eprint_str("cat: cannot open ");
                let _ = io::write_all(io::STDERR, path);
                let _ = io::eprint_str("\n");
                sys::exit(1);
            }
        }
    }
    sys::exit(0)
}
