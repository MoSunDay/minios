//! The exec case + the shared fork/redirect/exec/capture helper the
//! program exercisers use (utest/prog.rs).

use crate::sys;

/// execve with an argv of NUL-free byte strings (argv[0] = program
/// name, xv6 style). The kernel cap is 8 args / 64 argv bytes.
pub fn exec_argv(path: &[u8], argv: &[&[u8]]) -> Result<i64, i64> {
    let mut strs = [[0u8; 24]; 8];
    let mut ptrs = [0usize; 8];
    for (slot, arg) in strs.iter_mut().zip(argv.iter()) {
        slot[..arg.len()].copy_from_slice(arg);
    }
    for (i, slot) in strs.iter().enumerate().take(argv.len()) {
        ptrs[i] = slot.as_ptr() as usize;
    }
    sys::execve(path, ptrs.as_ptr() as *const usize, core::ptr::null())
}

/// Fork, point the child's fd 1 at a pipe (close-then-dup, lowest fd —
/// the sh.rs redirection discipline), exec `path`, and capture its
/// stdout until EOF. Returns (exit_code, captured_len).
pub fn run_capture(path: &[u8], argv: &[&[u8]], cap: &mut [u8]) -> Option<(i32, usize)> {
    let mut fds = [0i32; 2];
    sys::pipe(&mut fds).ok()?;
    let pid = sys::fork().ok()?;
    if pid == 0 {
        if sys::close(fds[0]).is_err() {
            sys::exit(101);
        }
        if sys::close(1).is_err() {
            sys::exit(102);
        }
        match sys::dup(fds[1]) {
            Ok(1) => {}
            _ => sys::exit(103),
        }
        if sys::close(fds[1]).is_err() {
            sys::exit(104);
        }
        if exec_argv(path, argv).is_err() {
            sys::exit(105);
        }
        sys::exit(106); // a successful exec never returns
    }
    if sys::close(fds[1]).is_err() {
        return None;
    }
    let mut len = 0usize;
    while len < cap.len() {
        match sys::read(fds[0], &mut cap[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(_) => {
                let _ = sys::close(fds[0]);
                return None;
            }
        }
    }
    if sys::close(fds[0]).is_err() {
        return None;
    }
    let mut status: i32 = 0;
    match sys::wait4(pid as i32, &mut status) {
        Ok(p) if p == pid => Some(((status >> 8) & 0xff, len)),
        _ => None,
    }
}

/// xv6 exectest: exec /echo and check its output reached the pipe.
pub fn case_exectest() -> bool {
    let mut cap = [0u8; 64];
    match run_capture(
        b"/echo\0",
        &[b"echo", b"ALL", b"TESTS", b"PASSED"],
        &mut cap,
    ) {
        Some((0, n)) => &cap[..n] == b"ALL TESTS PASSED\n",
        _ => false,
    }
}
