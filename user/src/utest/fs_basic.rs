//! xv6 usertests.c basic fs cases: opentest, writetest, writetest1
//! (MAXFILE), createtest, createdelete, twofiles, sharedfd.

use crate::sys;

/// xv6 opentest: open a known file, fail to open a missing one.
pub fn case_opentest() -> bool {
    match sys::open(b"/echo\0", abi::O_RDONLY) {
        Ok(fd) => {
            let _ = sys::close(fd as i32);
        }
        Err(_) => return false,
    }
    matches!(sys::open(b"/doesnotexist\0", abi::O_RDONLY), Err(e) if e == abi::ENOENT)
}

/// xv6 writetest: 100 x ("a"*10 + "b"*10), reopen, one 2000-byte read.
pub fn case_writetest() -> bool {
    let fd = match sys::open(b"/small\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    for _ in 0..100 {
        if sys::write(fd, b"aaaaaaaaaa") != Ok(10) || sys::write(fd, b"bbbbbbbbbb") != Ok(10) {
            let _ = sys::close(fd);
            return false;
        }
    }
    let _ = sys::close(fd);
    let fd = match sys::open(b"/small\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut buf = [0u8; 2000];
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n != Ok(2000) {
        return false;
    }
    sys::unlink(b"/small\0").is_ok()
}

/// The on-disk file-size ceiling: NDIRECT(12) + NINDIRECT(128) blocks.
const MAXFILE: usize = 140;

/// xv6 writetest1: MAXFILE 512-byte blocks with a leading block index,
/// read back and verified; the 141st block write must fail.
pub fn case_writetest1() -> bool {
    let _ = sys::unlink(b"/big\0");
    let fd = match sys::open(b"/big\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut buf = [0u8; 512];
    for i in 0..MAXFILE as u32 {
        buf[..4].copy_from_slice(&i.to_le_bytes());
        if sys::write(fd, &buf) != Ok(512) {
            let _ = sys::close(fd);
            return false;
        }
    }
    if sys::write(fd, &buf) == Ok(512) {
        // Past MAXFILE the writei cap must refuse (short/failed write).
        let _ = sys::close(fd);
        return false;
    }
    let _ = sys::close(fd);
    let fd = match sys::open(b"/big\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut ok = true;
    for i in 0..MAXFILE as u32 {
        match sys::read(fd, &mut buf) {
            Ok(512) => {
                if u32::from_le_bytes(buf[..4].try_into().unwrap()) != i {
                    ok = false;
                }
            }
            _ => ok = false,
        }
    }
    if ok && sys::read(fd, &mut buf) != Ok(0) {
        ok = false; // EOF exactly at MAXFILE blocks
    }
    let _ = sys::close(fd);
    ok && sys::unlink(b"/big\0").is_ok()
}

/// xv6 createtest (52 creates + unlink) plus the create() semantics
/// guards: re-create of an existing file succeeds (xv6 create returns
/// the inode), while mkdir on an existing name returns EEXIST.
pub fn case_createtest() -> bool {
    let mut name = [0u8; 3];
    name[0] = b'a';
    for i in 0..52u8 {
        name[1] = b'0' + i;
        match sys::open(&name, abi::O_CREAT | abi::O_RDWR) {
            Ok(fd) => {
                let _ = sys::close(fd as i32);
            }
            Err(_) => return false,
        }
    }
    for i in 0..52u8 {
        name[1] = b'0' + i;
        if sys::unlink(&name).is_err() {
            return false;
        }
    }
    // Re-create of an existing file: xv6 create() hands back the inode
    // (no O_EXCL), so this must succeed.
    match sys::open(b"/ut-c0\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => {
            let _ = sys::close(fd as i32);
        }
        Err(_) => return false,
    }
    let reopen = sys::open(b"/ut-c0\0", abi::O_CREAT | abi::O_RDWR);
    if reopen.is_err() {}
    if let Ok(fd) = reopen {
        let _ = sys::close(fd as i32);
    }
    let c0_ok = reopen.is_ok();
    let _ = sys::unlink(b"/ut-c0\0");
    if sys::mkdir(b"/ut-d0\0").is_err() {
        return false;
    }
    // xv6-public create(): an existing entry is only reused for
    // file-on-file — mkdir on the existing dir (and on an existing file)
    // must fail with EEXIST.
    let second = sys::mkdir(b"/ut-d0\0");
    let dir_ok = sys::unlink(b"/ut-d0\0").is_ok();
    let mkdir_eexist = match sys::open(b"/ut-m0\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => {
            let r = matches!(sys::mkdir(b"/ut-m0\0"), Err(e) if e == abi::EEXIST);
            let _ = sys::close(fd as i32);
            let _ = sys::unlink(b"/ut-m0\0");
            r
        }
        Err(_) => return false,
    };
    let d0_eexist = matches!(second, Err(e) if e == abi::EEXIST);
    if !d0_eexist {}
    if !c0_ok {}
    if !mkdir_eexist {}
    c0_ok && dir_ok && d0_eexist && mkdir_eexist
}

/// xv6 createdelete: parent (p*) and child (c*) concurrently create files
/// in the same directory, deleting the earlier half as they go, then both
/// sets are checked: i==0 and i>=N/2 survive, 1..N/2 were deleted.
pub fn case_createdelete() -> bool {
    const N: u8 = 20;
    let pid = match sys::fork() {
        Ok(pid) => pid as i32,
        Err(_) => return false,
    };
    let mut name = [0u8; 3];
    name[0] = if pid != 0 { b'p' } else { b'c' };
    for i in 0..N {
        name[1] = b'0' + i;
        match sys::open(&name, abi::O_CREAT | abi::O_RDWR) {
            Ok(fd) => {
                let _ = sys::close(fd as i32);
            }
            Err(_) => return exit_createdelete(pid, false),
        }
        if i > 0 && i % 2 == 0 {
            name[1] = b'0' + i / 2;
            if sys::unlink(&name).is_err() {
                return exit_createdelete(pid, false);
            }
            name[1] = b'0' + i;
        }
    }
    if pid == 0 {
        sys::exit(0);
    }
    let mut status: i32 = 0;
    if !matches!(sys::wait4(pid, &mut status), Ok(p) if p == pid as i64) {
        return false;
    }
    // Both file sets: i == 0 or i >= N/2 must exist; 1..N/2 were unlinked.
    for set in [b'p', b'c'] {
        for i in 0..N {
            name[0] = set;
            name[1] = b'0' + i;
            match sys::open(&name, abi::O_RDONLY) {
                Ok(fd) => {
                    let _ = sys::close(fd as i32);
                    if i >= 1 && i < N / 2 {
                        return false;
                    }
                }
                Err(_) => {
                    if i == 0 || i >= N / 2 {
                        return false;
                    }
                }
            }
        }
    }
    for set in [b'p', b'c'] {
        for i in 0..N {
            name[0] = set;
            name[1] = b'0' + i;
            let _ = sys::unlink(&name);
        }
    }
    true
}

/// createdelete: the child reports its failure through its exit status.
fn exit_createdelete(pid: i32, ok: bool) -> bool {
    if pid == 0 {
        sys::exit(if ok { 0 } else { 1 });
    }
    let mut status: i32 = 0;
    let _ = sys::wait4(pid, &mut status);
    ok
}

/// xv6 twofiles: parent and child write distinct patterns to f1/f2, then
/// both files are read back and verified (500-byte writes, 12 rounds).
pub fn case_twofiles() -> bool {
    let _ = sys::unlink(b"/f1\0");
    let _ = sys::unlink(b"/f2\0");
    let pid = match sys::fork() {
        Ok(pid) => pid as i32,
        Err(_) => return false,
    };
    let fname: &[u8] = if pid != 0 { b"/f1\0" } else { b"/f2\0" };
    let fd = match sys::open(fname, abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return exit_twofiles(pid, false),
    };
    let fill = if pid != 0 { b'p' } else { b'c' };
    let mut ok = true;
    let buf = [fill; 512];
    for _ in 0..12 {
        // 500-byte writes leave the tail of the 512 buffer unwritten.
        if sys::write(fd, &buf[..500]) != Ok(500) {
            ok = false;
            break;
        }
    }
    let _ = sys::close(fd);
    if pid == 0 {
        sys::exit(if ok { 0 } else { 1 });
    }
    let mut status: i32 = 0;
    if !matches!(sys::wait4(pid, &mut status), Ok(p) if p == pid as i64) || !ok {
        return false;
    }
    for i in 0..2u8 {
        let fname: &[u8] = if i != 0 { b"/f1\0" } else { b"/f2\0" };
        let want = if i != 0 { b'p' } else { b'c' };
        let fd = match sys::open(fname, abi::O_RDONLY) {
            Ok(fd) => fd as i32,
            Err(_) => return false,
        };
        let mut buf = [0u8; 512];
        let mut total = 0usize;
        let mut ok = true;
        loop {
            match sys::read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if buf[..n].iter().any(|&b| b != want) {
                        ok = false;
                        break;
                    }
                    total += n;
                }
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        let _ = sys::close(fd);
        if !ok || total != 12 * 500 {
            return false;
        }
    }
    sys::unlink(b"/f1\0").is_ok() && sys::unlink(b"/f2\0").is_ok()
}

/// twofiles: the child reports its failure through its exit status.
fn exit_twofiles(pid: i32, ok: bool) -> bool {
    if pid == 0 {
        sys::exit(if ok { 0 } else { 1 });
    }
    let mut status: i32 = 0;
    let _ = sys::wait4(pid, &mut status);
    ok
}

/// xv6 sharedfd: parent and child share one fd, each appending 1000
/// 10-byte records ('p'/'c'); the file must hold exactly 10000 of each.
pub fn case_sharedfd() -> bool {
    let _ = sys::unlink(b"/sharedfd\0");
    let fd = match sys::open(b"/sharedfd\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let pid = match sys::fork() {
        Ok(pid) => pid as i32,
        Err(_) => return false,
    };
    let fill = if pid == 0 { b'c' } else { b'p' };
    let buf = [fill; 10];
    let mut ok = true;
    for _ in 0..1000 {
        if sys::write(fd, &buf) != Ok(10) {
            ok = false;
            break;
        }
    }
    if pid == 0 {
        sys::exit(if ok { 0 } else { 1 });
    }
    let mut status: i32 = 0;
    let reaped = matches!(sys::wait4(pid, &mut status), Ok(p) if p == pid as i64);
    let _ = sys::close(fd);
    if !ok || !reaped || (status >> 8) & 0xff != 0 {
        return false;
    }
    let fd = match sys::open(b"/sharedfd\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut buf = [0u8; 10];
    let (mut nc, mut np) = (0usize, 0usize);
    loop {
        match sys::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for i in 0..n {
                    if buf[i] == b'c' {
                        nc += 1;
                    }
                    if buf[i] == b'p' {
                        np += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = sys::close(fd);
    let cleaned = sys::unlink(b"/sharedfd\0").is_ok();
    nc == 10000 && np == 10000 && cleaned
}
