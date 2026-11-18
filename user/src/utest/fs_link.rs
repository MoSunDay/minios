//! xv6 usertests.c hard-link cases: linkunlink (concurrent create/link/
//! unlink of one name), linktest (link semantics + nlink counts),
//! unlinkread (an open fd survives the unlink and sees the old inode).

use crate::sys;

/// LCG from xv6 linkunlink (x = x * 1103515245 + 12345).
fn lcg(x: &mut u32) -> u32 {
    *x = x.wrapping_mul(1103515245).wrapping_add(12345);
    *x
}

/// xv6 linkunlink: parent and child run 100 randomized ops on the same
/// name (create / link /cat to it / unlink it) — a deadlock and
/// consistency probe for the inode and log layers.
pub fn case_linkunlink() -> bool {
    let _ = sys::unlink(b"x\0");
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut x: u32 = if pid != 0 { 1 } else { 97 };
    for _ in 0..100 {
        match lcg(&mut x) % 3 {
            0 => {
                if let Ok(fd) = sys::open(b"x\0", abi::O_RDWR | abi::O_CREAT) {
                    let _ = sys::close(fd as i32);
                }
            }
            1 => {
                let _ = sys::link(b"/cat\0", b"x\0");
            }
            _ => {
                let _ = sys::unlink(b"x\0");
            }
        }
    }
    if pid == 0 {
        sys::exit(0);
    }
    // The name may or may not survive the last random op; either way the
    // per-round root copy absorbs the leftover (xv6 leaves it too).
    let _ = sys::unlink(b"x\0");
    matches!(sys::wait4(pid as i32, core::ptr::null_mut()), Ok(p) if p == pid)
}

/// xv6 linktest plus a nlink check: two names share one inode with
/// nlink == 2, links onto existing names / dirs / missing files fail.
pub fn case_linktest() -> bool {
    let _ = sys::unlink(b"lf1\0");
    let _ = sys::unlink(b"lf2\0");
    let fd = match sys::open(b"lf1\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    if sys::write(fd, b"hello") != Ok(5) {
        let _ = sys::close(fd);
        return false;
    }
    let mut st = unsafe { core::mem::zeroed::<abi::Stat>() };
    let nlink1 = matches!(sys::fstat(fd, &mut st), Ok(_)) && st.nlink == 1;
    let _ = sys::close(fd);
    if !nlink1 {
        return false;
    }
    if sys::link(b"lf1\0", b"lf2\0").is_err() {
        return false;
    }
    // Both names now share the inode: same ino, nlink == 2 each.
    let mut st1 = unsafe { core::mem::zeroed::<abi::Stat>() };
    let mut st2 = unsafe { core::mem::zeroed::<abi::Stat>() };
    if sys::stat(b"lf1\0", &mut st1).is_err() || sys::stat(b"lf2\0", &mut st2).is_err() {
        return false;
    }
    if st1.ino != st2.ino || st1.nlink != 2 || st2.nlink != 2 {
        return false;
    }
    if sys::unlink(b"lf1\0").is_err() {
        return false;
    }
    if sys::open(b"lf1\0", abi::O_RDONLY).is_ok() {
        return false; // gone
    }
    let fd = match sys::open(b"lf2\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut buf = [0u8; 64];
    let n = sys::read(fd, &mut buf).unwrap_or(0);
    let _ = sys::close(fd);
    if n != 5 || buf[0] != b'h' {
        return false;
    }
    // link onto an existing name / a missing file / a directory: all fail.
    let self_link = sys::link(b"lf2\0", b"lf2\0").is_err();
    let _ = sys::unlink(b"lf2\0");
    let missing = sys::link(b"lf2\0", b"lf1\0").is_err();
    let dir_link = sys::link(b".\0", b"lf1\0").is_err();
    self_link && missing && dir_link
}

/// xv6 unlinkread: after unlink, the open fd still reads the original
/// inode; a new create of the same name builds a fresh inode.
pub fn case_unlinkread() -> bool {
    let fd = match sys::open(b"unlinkread\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    if sys::write(fd, b"hello") != Ok(5) {
        let _ = sys::close(fd);
        return false;
    }
    let _ = sys::close(fd);
    let fd = match sys::open(b"unlinkread\0", abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    if sys::unlink(b"unlinkread\0").is_err() {
        let _ = sys::close(fd);
        return false;
    }
    // Same name, new inode: writing it must not disturb the open fd.
    let fd1 = sys::open(b"unlinkread\0", abi::O_CREAT | abi::O_RDWR);
    if let Ok(fd1) = fd1 {
        let _ = sys::write(fd1 as i32, b"yyy");
        let _ = sys::close(fd1 as i32);
    } else {
        let _ = sys::close(fd);
        return false;
    }
    let mut buf = [0u8; 64];
    let n = sys::read(fd, &mut buf).unwrap_or(0);
    let read_ok = n == 5 && buf[0] == b'h';
    // The unlinked inode is still writable through its open fd (xv6).
    let write_ok = sys::write(fd, &buf[..10]) == Ok(10);
    let _ = sys::close(fd);
    let cleaned = sys::unlink(b"unlinkread\0").is_ok();
    read_ok && write_ok && cleaned
}
