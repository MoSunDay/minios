//! wc(1), xv6 wc.c: count lines, words, bytes for stdin or files.

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

struct Counts {
    l: u64,
    w: u64,
    c: u64,
}

fn wc(fd: i32, name: &[u8]) {
    let mut t = Counts { l: 0, w: 0, c: 0 };
    let mut buf = [0u8; 512];
    let mut inword = false;
    loop {
        let n = match sys::read(fd, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            t.c += 1;
            if b == b'\n' {
                t.l += 1;
            }
            if b == b' ' || b == b'\t' || b == b'\n' {
                inword = false;
            } else if !inword {
                inword = true;
                t.w += 1;
            }
        }
    }
    let _ = io::print_uint(t.l);
    let _ = io::print_str(" ");
    let _ = io::print_uint(t.w);
    let _ = io::print_str(" ");
    let _ = io::print_uint(t.c);
    let _ = io::print_str(" ");
    let _ = io::write_all(io::STDOUT, name);
    let _ = io::print_str("\n");
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() <= 1 {
        wc(0, b"\0");
        sys::exit(0);
    }
    for i in 1..a.argc() {
        match a.arg(i) {
            Some(p) => match sys::open(p, abi::O_RDONLY) {
                Ok(fd) => {
                    wc(fd as i32, p);
                    let _ = sys::close(fd as i32);
                }
                Err(_) => {
                    let _ = io::eprint_str("wc: cannot open ");
                    let _ = io::write_all(io::STDERR, p);
                    let _ = io::eprint_str("\n");
                }
            },
            None => continue,
        }
    }
    sys::exit(0)
}
