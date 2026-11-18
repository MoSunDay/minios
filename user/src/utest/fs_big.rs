//! xv6 usertests.c size-stress cases: bigwrite (writes larger than the
//! log), bigfile (indirect-block file with a verified pattern), bigdir
//! (a directory grown past its direct blocks via 500 links).

use crate::sys;

/// xv6 bigwrite: two `sz`-byte writes for sz 499..6143 step 471 —
/// sizes above LOGSIZE force the write path to chunk transactions.
pub fn case_bigwrite() -> bool {
    let mut buf = [0xaau8; 8192];
    let mut sz = 499usize;
    while sz < 12 * 512 {
        let _ = sys::unlink(b"bigwrite\0");
        let fd = match sys::open(b"bigwrite\0", abi::O_CREAT | abi::O_RDWR) {
            Ok(fd) => fd as i32,
            Err(_) => return false,
        };
        let mut ok = true;
        for _ in 0..2 {
            if sys::write(fd, &buf[..sz]) != Ok(sz) {
                ok = false;
                break;
            }
        }
        let _ = sys::close(fd);
        if !ok {
            return false;
        }
        let fd = match sys::open(b"bigwrite\0", abi::O_RDONLY) {
            Ok(fd) => fd as i32,
            Err(_) => return false,
        };
        let mut total = 0usize;
        loop {
            match sys::read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(_) => return false,
            }
        }
        let _ = sys::close(fd);
        if total != 2 * sz {
            return false;
        }
        if sys::unlink(b"bigwrite\0").is_err() {
            return false;
        }
        sz += 471;
    }
    true
}

/// xv6 bigfile: 20 x 600 bytes of a block-index pattern, read back in
/// 300-byte chunks (crossing block boundaries) and verified.
pub fn case_bigfile() -> bool {
    let _ = sys::unlink(b"bigfile\0");
    let fd = match sys::open(b"bigfile\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    for i in 0..20u8 {
        let buf = [i; 600];
        if sys::write(fd, &buf) != Ok(600) {
            let _ = sys::close(fd);
            return false;
        }
    }
    let _ = sys::close(fd);
    let fd = match sys::open(b"bigfile\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut total = 0usize;
    let mut i = 0usize;
    let mut ok = true;
    let mut buf = [0u8; 300];
    loop {
        match sys::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(300) => {
                if buf[0] != (i / 2) as u8 || buf[299] != (i / 2) as u8 {
                    ok = false;
                }
                total += 300;
            }
            _ => {
                ok = false;
                break;
            }
        }
        i += 1;
    }
    let _ = sys::close(fd);
    ok && total == 20 * 600 && sys::unlink(b"bigfile\0").is_ok()
}

/// xv6 bigdir: link one file under 500 names, growing the root dirent
/// list past its direct blocks; unlinking all of them must restore it.
pub fn case_bigdir() -> bool {
    let _ = sys::unlink(b"bd\0");
    let fd = match sys::open(b"bd\0", abi::O_CREAT) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let _ = sys::close(fd);
    let mut name = [0u8; 5];
    name[0] = b'x';
    for i in 0..500u16 {
        name[1] = b'0' + (i / 64) as u8;
        name[2] = b'0' + (i % 64) as u8;
        if sys::link(b"bd\0", &name).is_err() {
            return false;
        }
    }
    if sys::unlink(b"bd\0").is_err() {
        return false;
    }
    for i in 0..500u16 {
        name[1] = b'0' + (i / 64) as u8;
        name[2] = b'0' + (i % 64) as u8;
        if sys::unlink(&name).is_err() {
            return false;
        }
    }
    // Space restored: a fresh file and inode alloc must succeed again.
    let fd = match sys::open(b"bd\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let ok = sys::write(fd, b"ok") == Ok(2);
    let _ = sys::close(fd);
    ok && sys::unlink(b"bd\0").is_ok()
}
