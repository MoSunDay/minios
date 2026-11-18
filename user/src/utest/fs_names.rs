//! xv6 usertests.c name-handling cases: subdir (the full deep-path
//! suite), fourteen (14-char DIRSIZ boundary), rmdot (unlink "." and
//! ".." must fail), dirfile (dir vs file confusion must fail).

use crate::sys;

/// xv6 subdir: nested dirs, ".." walks (including root clamping),
/// unlink of non-empty dirs failing, and every path-shape confusion.
pub fn case_subdir() -> bool {
    let _ = sys::unlink(b"ff\0");
    if sys::mkdir(b"dd\0").is_err() {
        return false;
    }
    let fd = match sys::open(b"dd/ff\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let wrote = sys::write(fd, b"ff") == Ok(2);
    let _ = sys::close(fd);
    if !wrote || sys::unlink(b"dd\0").is_ok() {
        return false; // unlink of a non-empty dir must fail
    }
    if sys::mkdir(b"/dd/dd\0").is_err() {
        return false;
    }
    let fd = match sys::open(b"dd/dd/ff\0", abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let wrote = sys::write(fd, b"FF") == Ok(2);
    let _ = sys::close(fd);
    if !wrote {
        return false;
    }
    // dd/dd/.. == dd: "ff" (2 bytes, starts 'f').
    let mut buf = [0u8; 64];
    let fd = match sys::open(b"dd/dd/../ff\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let n = sys::read(fd, &mut buf).unwrap_or(0);
    let _ = sys::close(fd);
    if n != 2 || buf[0] != b'f' {
        return false;
    }
    if sys::link(b"dd/dd/ff\0", b"dd/dd/ffff\0").is_err() {
        return false;
    }
    if sys::unlink(b"dd/dd/ff\0").is_err() {
        return false;
    }
    if sys::open(b"dd/dd/ff\0", abi::O_RDONLY).is_ok() {
        return false; // unlinked
    }
    // ".." clamps at the root: every one of these lands back in dd.
    for path in [
        b"dd\0".as_slice(),
        b"dd/../../dd\0".as_slice(),
        b"dd/../../../dd\0".as_slice(),
    ] {
        if sys::chdir(path).is_err() {
            return false;
        }
    }
    if sys::chdir(b"./..\0").is_err() {
        return false; // now at /
    }
    let fd = match sys::open(b"dd/dd/ffff\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let n = sys::read(fd, &mut buf).unwrap_or(0);
    let _ = sys::close(fd);
    if n != 2 || buf[0] != b'F' {
        return false;
    }
    // Every confused path shape must fail.
    let must_fail = |path: &[u8], create: bool| -> bool {
        let flags = if create {
            abi::O_CREAT | abi::O_RDWR
        } else {
            abi::O_RDONLY
        };
        match sys::open(path, flags) {
            Ok(fd) => {
                let _ = sys::close(fd as i32);
                false
            }
            Err(_) => true,
        }
    };
    if !must_fail(b"dd/dd/ff\0", false) // unlinked earlier
        || !must_fail(b"dd/ff/ff\0", true) // file under a file
        || !must_fail(b"dd/xx/ff\0", true) // file under a missing dir
        || !must_fail(b"dd\0", true)
    // O_CREAT on an existing dir: type mismatch -> EEXIST.
    {
        return false;
    }
    // open("dd", O_RDWR) and open("dd", O_WRONLY): writing a dir.
    if sys::open(b"dd\0", abi::O_RDWR).is_ok() || sys::open(b"dd\0", abi::O_WRONLY).is_ok() {
        return false;
    }
    if sys::link(b"dd/ff/ff\0", b"dd/dd/xx\0").is_ok()
        || sys::link(b"dd/xx/ff\0", b"dd/dd/xx\0").is_ok()
    {
        return false;
    }
    // link onto an existing name must fail (EEXIST).
    if sys::link(b"dd/ff\0", b"dd/dd/ffff\0").is_ok() {
        return false;
    }
    if sys::mkdir(b"dd/ff/ff\0").is_ok()
        || sys::mkdir(b"dd/xx/ff\0").is_ok()
        || sys::mkdir(b"dd/dd/ffff\0").is_ok()
    {
        return false;
    }
    if sys::unlink(b"dd/xx/ff\0").is_ok() || sys::unlink(b"dd/ff/ff\0").is_ok() {
        return false;
    }
    if sys::chdir(b"dd/ff\0").is_ok() || sys::chdir(b"dd/xx\0").is_ok() {
        return false;
    }
    // Teardown in the exact xv6 order.
    sys::unlink(b"dd/dd/ffff\0").is_ok()
        && sys::unlink(b"dd/ff\0").is_ok()
        && sys::unlink(b"dd\0").is_err() // still contains dd/dd
        && sys::unlink(b"dd/dd\0").is_ok()
        && sys::unlink(b"dd\0").is_ok()
}

/// xv6 fourteen: DIRSIZ is 14 — 14-char names work, 15-char paths fail.
pub fn case_fourteen() -> bool {
    if sys::mkdir(b"12345678901234\0").is_err() {
        return false;
    }
    if sys::mkdir(b"12345678901234/123456789012345\0").is_err() {
        return false; // the final path component is 14 chars
    }
    let fd = sys::open(
        b"123456789012345/123456789012345/123456789012345\0",
        abi::O_CREAT,
    );
    if let Ok(fd) = fd {
        let _ = sys::close(fd as i32);
    } else {
        return false;
    }
    let fd = sys::open(
        b"12345678901234/12345678901234/12345678901234\0",
        abi::O_RDONLY,
    );
    if let Ok(fd) = fd {
        let _ = sys::close(fd as i32);
    } else {
        return false;
    }
    if sys::mkdir(b"12345678901234/12345678901234\0").is_ok() {
        return false; // exists
    }
    if sys::mkdir(b"123456789012345/12345678901234\0").is_ok() {
        return false; // leading 15-char component cannot resolve
    }
    true
}

/// xv6 rmdot: unlink of "." and ".." must fail everywhere.
pub fn case_rmdot() -> bool {
    if sys::mkdir(b"dots\0").is_err() || sys::chdir(b"dots\0").is_err() {
        return false;
    }
    let dot = sys::unlink(b".\0").is_err();
    let dotdot = sys::unlink(b"..\0").is_err();
    if sys::chdir(b"/\0").is_err() {
        return false;
    }
    let dots_dot = sys::unlink(b"dots/.\0").is_err();
    let dots_dotdot = sys::unlink(b"dots/..\0").is_err();
    let cleaned = sys::unlink(b"dots\0").is_ok();
    dot && dotdot && dots_dot && dots_dotdot && cleaned
}

/// xv6 dirfile: a plain file is not a directory — every confused op on
/// it must fail, and writing a directory fd is refused.
pub fn case_dirfile() -> bool {
    let fd = match sys::open(b"dirfile\0", abi::O_CREAT) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let _ = sys::close(fd);
    if sys::chdir(b"dirfile\0").is_ok() {
        return false;
    }
    if sys::open(b"dirfile/xx\0", abi::O_RDONLY).is_ok() {
        return false;
    }
    if sys::open(b"dirfile/xx\0", abi::O_CREAT).is_ok() {
        return false;
    }
    if sys::mkdir(b"dirfile/xx\0").is_ok() {
        return false;
    }
    if sys::unlink(b"dirfile/xx\0").is_ok() {
        return false;
    }
    if sys::link(b"ut-nonexistent\0", b"dirfile/xx\0").is_ok() {
        return false;
    }
    if sys::unlink(b"dirfile\0").is_err() {
        return false;
    }
    // A directory fd opens read-only, but writing it is refused.
    if sys::open(b".\0", abi::O_RDWR).is_ok() {
        return false;
    }
    let fd = match sys::open(b".\0", abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let write_refused = sys::write(fd, b"x") != Ok(1);
    let _ = sys::close(fd);
    write_refused
}
