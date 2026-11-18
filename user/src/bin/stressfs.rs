//! stressfs(1), xv6 stressfs.c: a fork chain (5 processes) hammers file
//! create/write/read to provoke IDE and log contention.

#![no_std]
#![no_main]

use minios_user::{io, sys};

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let _ = io::print_str("stressfs starting\n");
    let mut path = *b"/stressfs0\0";
    for i in 0..4u8 {
        match sys::fork() {
            Ok(0) => {}
            _ => break,
        }
        // Child i renames its target file to stressfs{i+1} (xv6 mutates
        // the inherited buffer in place).
        path[9] = b'1' + i;
    }
    let _ = io::print_str("write ");
    let data = [b'a'; 512];
    match sys::open(&path, abi::O_RDWR | abi::O_CREAT) {
        Ok(fd) => {
            for _ in 0..20 {
                if sys::write(fd as i32, &data) != Ok(512) {
                    break;
                }
            }
            let _ = sys::close(fd as i32);
            let _ = io::print_str("done\n");
            let _ = io::print_str("read ");
            if let Ok(fd2) = sys::open(&path, abi::O_RDONLY) {
                for _ in 0..20 {
                    let mut buf = [0u8; 512];
                    if !matches!(sys::read(fd2 as i32, &mut buf), Ok(512)) {
                        break;
                    }
                }
                let _ = sys::close(fd2 as i32);
            }
        }
        Err(_) => {}
    }
    let _ = io::print_str("done\n");
    let _ = sys::wait4(-1, core::ptr::null_mut());
    sys::exit(0)
}
