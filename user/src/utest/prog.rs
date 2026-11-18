//! Exercisers for the S4e-1 program set: each runs a real binary via
//! exec with stdout piped to us, then checks the exit code, the pipe
//! bytes, and the on-disk effects.

use super::exec::run_capture;
use crate::sys;

/// Decimal formatter for tiny argv fields (pid strings).
fn u32_dec(v: u32, out: &mut [u8; 12]) -> usize {
    if v == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut rev = [0u8; 12];
    let mut i = 0;
    let mut v = v;
    while v > 0 {
        rev[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        out[j] = rev[i - 1 - j];
    }
    i
}

/// Write `content` to a fresh `/name` file.
fn seed_file(name: &[u8], content: &[u8]) -> bool {
    let _ = sys::unlink(name);
    let fd = match sys::open(name, abi::O_CREAT | abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let ok = sys::write(fd, content) == Ok(content.len());
    let _ = sys::close(fd);
    ok
}

pub fn case_prog_echo() -> bool {
    let mut cap = [0u8; 64];
    match run_capture(b"/echo\0", &[b"echo", b"hello", b"xv6"], &mut cap) {
        Some((0, n)) => &cap[..n] == b"hello xv6\n",
        _ => false,
    }
}

pub fn case_prog_cat() -> bool {
    let content = b"usertests cat probe\nline two\n";
    if !seed_file(b"/utcat\0", content) {
        return false;
    }
    let mut cap = [0u8; 128];
    let ok = matches!(
        run_capture(b"/cat\0", &[b"cat", b"/utcat"], &mut cap),
        Some((0, n)) if &cap[..n] == content
    );
    sys::unlink(b"/utcat\0").is_ok() && ok
}

pub fn case_prog_grep() -> bool {
    if !seed_file(b"/utgrep\0", b"alpha\nbeta gamma\nalpha2\n") {
        return false;
    }
    let mut cap = [0u8; 128];
    let ok = matches!(
        run_capture(b"/grep\0", &[b"grep", b"alpha", b"/utgrep"], &mut cap),
        Some((0, n)) if &cap[..n] == b"alpha\nalpha2\n"
    );
    sys::unlink(b"/utgrep\0").is_ok() && ok
}

pub fn case_prog_wc() -> bool {
    // 2 lines, 5 words, 10 bytes (xv6 wc's counting on this content).
    if !seed_file(b"/utwc\0", b"a b c\nd e\n") {
        return false;
    }
    let mut cap = [0u8; 64];
    let ok = matches!(
        run_capture(b"/wc\0", &[b"wc", b"/utwc"], &mut cap),
        Some((0, n)) if &cap[..n] == b"2 5 10 /utwc\n"
    );
    sys::unlink(b"/utwc\0").is_ok() && ok
}

pub fn case_prog_ls() -> bool {
    // Root listing must carry the program set and this suite's own
    // driver image (both unique substrings).
    let mut cap = [0u8; 1024];
    match run_capture(b"/ls\0", &[b"ls"], &mut cap) {
        Some((0, n)) => {
            let out = &cap[..n];
            find(out, b"echo").is_some() && find(out, b"usertests").is_some()
        }
        _ => false,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn case_prog_mkdir() -> bool {
    let _ = sys::unlink(b"/utdir\0");
    let mut cap = [0u8; 64];
    let made = matches!(
        run_capture(b"/mkdir\0", &[b"mkdir", b"/utdir"], &mut cap),
        Some((0, _))
    );
    let mut st = unsafe { core::mem::zeroed::<abi::Stat>() };
    let is_dir = sys::stat(b"/utdir\0", &mut st) == Ok(0) && st.type_ == abi::Stat::T_DIR;
    // mkdir on the existing name: the program must fail.
    let refused = matches!(
        run_capture(b"/mkdir\0", &[b"mkdir", b"/utdir"], &mut cap),
        Some((code, _)) if code != 0
    );
    sys::unlink(b"/utdir\0").is_ok() && made && is_dir && refused
}

pub fn case_prog_rm() -> bool {
    if sys::mkdir(b"/utdir2\0").is_err() {
        return false;
    }
    let mut cap = [0u8; 64];
    let removed = matches!(
        run_capture(b"/rm\0", &[b"rm", b"/utdir2"], &mut cap),
        Some((0, _))
    ) && matches!(sys::open(b"/utdir2\0", abi::O_RDONLY), Err(e) if e == abi::ENOENT);
    let missing_fails = matches!(
        run_capture(b"/rm\0", &[b"rm", b"/utnoent"], &mut cap),
        Some((code, _)) if code != 0
    );
    removed && missing_fails
}

pub fn case_prog_ln() -> bool {
    if !seed_file(b"/utln-a\0", b"hello") {
        return false;
    }
    let mut cap = [0u8; 64];
    let linked = matches!(
        run_capture(b"/ln\0", &[b"ln", b"/utln-a", b"/utln-b"], &mut cap),
        Some((0, _))
    );
    let mut sa = unsafe { core::mem::zeroed::<abi::Stat>() };
    let mut sb = unsafe { core::mem::zeroed::<abi::Stat>() };
    let same_inode = sys::stat(b"/utln-a\0", &mut sa).is_ok()
        && sys::stat(b"/utln-b\0", &mut sb).is_ok()
        && sa.ino == sb.ino
        && sa.nlink == 2;
    let _ = sys::unlink(b"/utln-a\0");
    let _ = sys::unlink(b"/utln-b\0");
    linked && same_inode
}

pub fn case_prog_kill() -> bool {
    // A sleeper child; the real kill(1) binary must reap it via
    // kill(2): the sleeper wakes with EINTR and exits by itself.
    let child = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if child == 0 {
        match sys::sleep(30) {
            Err(_) => sys::exit(1), // killed: nanosleep aborted (EINTR)
            Ok(_) => sys::exit(0),
        }
    }
    let mut pidstr = [0u8; 12];
    let plen = u32_dec(child as u32, &mut pidstr);
    let pid_arg = &pidstr[..plen];
    let mut cap = [0u8; 64];
    let killed = matches!(
        run_capture(b"/kill\0", &[b"kill", pid_arg], &mut cap),
        Some((0, _))
    );
    let reaped = matches!(sys::wait4(child as i32, core::ptr::null_mut()), Ok(p) if p == child);
    killed && reaped
}

pub fn case_prog_zombie() -> bool {
    // The binary's parent exits at once; the pipe stays open ~5 s in
    // the orphaned grandchild — EOF proves the grandchild finally
    // exited (and pid 1 reaps it later as its orphan).
    let mut cap = [0u8; 128];
    matches!(
        run_capture(b"/zombie\0", &[b"zombie"], &mut cap),
        Some((0, _))
    )
}

pub fn case_prog_forktest() -> bool {
    let mut cap = [0u8; 128];
    match run_capture(b"/forktest\0", &[b"forktest"], &mut cap) {
        Some((0, n)) => find(&cap[..n], b"forktest").is_some(),
        _ => false,
    }
}

pub fn case_prog_stressfs() -> bool {
    let mut cap = [0u8; 128];
    let ran = matches!(
        run_capture(b"/stressfs\0", &[b"stressfs"], &mut cap),
        Some((0, n)) if find(&cap[..n], b"stressfs starting").is_some()
    );
    // The fork chain (root + 4 generations) left /stressfs0../stressfs4,
    // each 20 x 512 bytes.
    let mut files_ok = true;
    for i in 0..=4u8 {
        let mut name = *b"/stressfs0\0";
        name[9] = b'0' + i;
        let mut st = unsafe { core::mem::zeroed::<abi::Stat>() };
        if sys::stat(&name, &mut st) != Ok(0)
            || st.type_ != abi::Stat::T_FILE
            || st.size != 20 * 512
        {
            files_ok = false;
        }
    }
    ran && files_ok
}
