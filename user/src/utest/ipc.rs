//! xv6 usertests.c IPC/process cases: pipe1 (a forked writer pushing
//! past the 512-byte ring so pipewrite blocks), preempt (spinners + a
//! pipe writer, all killed and reaped), and exitwait (100 fork/exit/
//! wait rounds with exact pid checks).

use crate::sys;
use abi::sig::SIGKILL;

/// xv6 pipe1: the child writes 5 x 1033 bytes of a byte sequence (the
/// write end blocks on the ring until the parent drains); the parent
/// reads with a doubling chunk size and re-verifies the sequence.
pub fn case_pipe1() -> bool {
    let mut fds = [0i32; 2];
    if sys::pipe(&mut fds).is_err() {
        return false;
    }
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut seq: u8 = 0;
    if pid == 0 {
        let _ = sys::close(fds[0]);
        let mut buf = [0u8; 1033];
        let mut ok = true;
        for _ in 0..5 {
            for b in buf.iter_mut() {
                *b = seq;
                seq = seq.wrapping_add(1);
            }
            if sys::write(fds[1], &buf) != Ok(1033) {
                ok = false;
                break;
            }
        }
        sys::exit(if ok { 0 } else { 1 });
    }
    let _ = sys::close(fds[1]);
    let mut buf = [0u8; 512];
    let mut total = 0usize;
    let mut expect: u8 = 0;
    let mut ok = true;
    let mut cc = 1usize;
    loop {
        let n = match sys::read(fds[0], &mut buf[..cc]) {
            Ok(0) => break, // EOF: child exited and the write end closed
            Ok(n) => n,
            Err(_) => {
                ok = false;
                break;
            }
        };
        for i in 0..n {
            if buf[i] != expect {
                ok = false;
            }
            expect = expect.wrapping_add(1);
        }
        total += n;
        cc = (cc * 2).min(buf.len());
    }
    let _ = sys::close(fds[0]);
    let mut status: i32 = 0;
    let reaped = matches!(sys::wait4(pid as i32, &mut status), Ok(p) if p == pid);
    ok && reaped && (status >> 8) & 0xff == 0 && total == 5 * 1033
}

/// xv6 exitwait: 100 fork/exit/wait rounds; wait4 must return the exact
/// child pid each time (exit/wait race probe).
pub fn case_exitwait() -> bool {
    for _ in 0..100 {
        let pid = match sys::fork() {
            Ok(p) => p,
            Err(_) => return false,
        };
        if pid == 0 {
            sys::exit(0);
        }
        if !matches!(sys::wait4(pid as i32, core::ptr::null_mut()), Ok(p) if p == pid) {
            return false;
        }
    }
    true
}

/// xv6 preempt: two pure user-mode spinners plus a third child that
/// writes one pipe byte and spins. Preemption must let the parent read
/// the byte; kill(2) must exit all three on their next user trap.
pub fn case_preempt() -> bool {
    let spin = || -> ! {
        loop {
            core::hint::spin_loop();
        }
    };
    let pid1 = match sys::fork() {
        Ok(0) => spin(),
        Ok(p) => p,
        Err(_) => return false,
    };
    let pid2 = match sys::fork() {
        Ok(0) => spin(),
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut fds = [0i32; 2];
    if sys::pipe(&mut fds).is_err() {
        return false;
    }
    let pid3 = match sys::fork() {
        Ok(0) => {
            let _ = sys::close(fds[0]);
            let one = *b"x";
            let wrote = sys::write(fds[1], &one) == Ok(1);
            let _ = sys::close(fds[1]);
            if !wrote {
                sys::exit(1);
            }
            spin();
        }
        Ok(p) => p,
        Err(_) => return false,
    };
    let _ = sys::close(fds[1]);
    let mut byte = [0u8; 1];
    let got = matches!(sys::read(fds[0], &mut byte), Ok(1)) && byte[0] == b'x';
    let _ = sys::close(fds[0]);
    let killed = sys::kill(pid1 as i32, SIGKILL).is_ok()
        && sys::kill(pid2 as i32, SIGKILL).is_ok()
        && sys::kill(pid3 as i32, SIGKILL).is_ok();
    let mut reaped = true;
    for pid in [pid1 as i32, pid2 as i32, pid3 as i32] {
        if !matches!(sys::wait4(pid, core::ptr::null_mut()), Ok(p) if p == pid as i64) {
            reaped = false;
        }
    }
    got && killed && reaped
}
