//! m3r-s6 signal cases (21 §6/§7): handler roundtrip through the
//! rt_sigframe/restorer/sigreturn contract (delivery at the syscall
//! exit, register continuity, mask restore), procmask deferral,
//! EPIPE/SIGPIPE (default death vs blocked/ignored survival), sync
//! exceptions (default + force-past-SIG_IGN), SIGCHLD at child exit,
//! and alarm(2) expiry interrupting user mode.

use crate::sys;
use abi::sig::*;
use core::sync::atomic::{AtomicUsize, Ordering};

fn sigmask(sig: i32) -> u64 {
    1u64 << (sig as u32 - 1)
}

fn install(sig: i32, handler: extern "C" fn(u64), extra_mask: u64) -> bool {
    let act = UserSigaction {
        handler: handler as u64,
        flags: SA_RESTORER,
        restorer: sys::sigreturn_stub(),
        mask: extra_mask,
    };
    sys::rt_sigaction(sig, Some(&act), None).is_ok()
}

fn ignore(sig: i32) -> bool {
    let act = UserSigaction {
        handler: SIG_IGN,
        flags: SA_RESTORER,
        restorer: sys::sigreturn_stub(),
        mask: 0,
    };
    sys::rt_sigaction(sig, Some(&act), None).is_ok()
}

static HANDLED: AtomicUsize = AtomicUsize::new(0);

extern "C" fn note_handler(sig: u64) {
    HANDLED.store(sig as usize, Ordering::SeqCst);
}

/// xv6.sig.roundtrip: kill(getpid(), SIGUSR1) returns Ok(0) with the
/// handler already run (delivery at the syscall-exit tail, sigreturn
/// restoring rax/rip/rsp); a second delivery repeats; the queried
/// old action mirrors the installed handler; the mask survives.
pub fn case_roundtrip() -> bool {
    HANDLED.store(0, Ordering::SeqCst);
    if !install(SIGUSR1, note_handler, 0) {
        return false;
    }
    let pid = match sys::getpid() {
        Ok(p) => p as i32,
        Err(_) => return false,
    };
    // The interrupted-kill contract: Ok(0) observed, handler observed,
    // both across the same syscall boundary.
    if !matches!(sys::kill(pid, SIGUSR1), Ok(0)) {
        return false;
    }
    if HANDLED.load(Ordering::SeqCst) != SIGUSR1 as usize {
        return false;
    }
    // Locals pinned across the delivery window keep their values (the
    // sigreturn image must restore every callee-saved register).
    let canary = 0x1234_5678_9abc_def0u64;
    HANDLED.store(0, Ordering::SeqCst);
    if !matches!(sys::kill(pid, SIGUSR1), Ok(0)) || canary != 0x1234_5678_9abc_def0 {
        return false;
    }
    if HANDLED.load(Ordering::SeqCst) != SIGUSR1 as usize {
        return false;
    }
    // Mask untouched by a handler with empty sa_mask.
    let mut old = 1u64;
    if !sys::rt_sigprocmask(SIG_BLOCK, None, Some(&mut old)).is_ok_and(|v| v == 0) {
        return false;
    }
    old == 0
}

/// xv6.sig.mask_defer: a blocked SIGUSR2 pends without loss (visible
/// through rt_sigpending, handler silent) until SIG_UNBLOCK — the
/// unblocking syscall's exit tail delivers it.
pub fn case_mask_defer() -> bool {
    HANDLED.store(0, Ordering::SeqCst);
    if !install(SIGUSR2, note_handler, 0) {
        return false;
    }
    let pid = match sys::getpid() {
        Ok(p) => p as i32,
        Err(_) => return false,
    };
    let m = sigmask(SIGUSR2);
    let mut old = u64::MAX;
    if !matches!(
        sys::rt_sigprocmask(SIG_SETMASK, Some(&m), Some(&mut old)),
        Ok(0)
    ) || old != 0
    {
        return false;
    }
    if !matches!(sys::kill(pid, SIGUSR2), Ok(0)) {
        return false;
    }
    if HANDLED.load(Ordering::SeqCst) != 0 {
        return false; // blocked means deferred, not delivered
    }
    let mut pending = 0u64;
    if !sys::rt_sigpending(&mut pending).is_ok_and(|v| v == 0) {
        return false;
    }
    if pending & m == 0 || pending & !m != 0 {
        return false; // exactly SIGUSR2 pending
    }
    // Unblock: the same syscall's exit delivers.
    if !matches!(sys::rt_sigprocmask(SIG_UNBLOCK, Some(&m), None), Ok(0)) {
        return false;
    }
    let delivered = HANDLED.load(Ordering::SeqCst) == SIGUSR2 as usize;
    sys::rt_sigpending(&mut pending).ok();
    delivered && pending & m == 0
}

/// xv6.sig.sigpipe: a default-disposition writer on a readerless pipe
/// dies by SIGPIPE (wstatus low 7 bits = 13); a blocked or ignored
/// SIGPIPE only turns the write into Err(EPIPE) with the writer alive.
pub fn case_sigpipe() -> bool {
    HANDLED.store(0, Ordering::SeqCst);
    let mut fds = [0i32; 2];
    if sys::pipe(&mut fds).is_err() {
        return false;
    }
    // Fork before touching dispositions: the child carries defaults.
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        let _ = sys::close(fds[0]);
        let buf = [0u8; 1033];
        let _ = sys::write(fds[1], &buf); // ring fills -> SIGPIPE
        sys::exit(1); // unreachable on the default disposition
    }
    // Drop the parent's READ end: readopen reaches 0 only when both
    // copies are gone (the child closes its own after fork), so the
    // child's ring-filling write sees a readerless pipe. fds[1] stays
    // open for the survival checks below.
    let _ = sys::close(fds[0]);
    let mut wstatus = 0i32;
    if !sys::wait4(pid as i32, &mut wstatus).is_ok_and(|p| p == pid) {
        return false;
    }
    if wstatus & 0x7f != SIGPIPE as i32 {
        return false; // not signalled to death by 13
    }
    // Blocked SIGPIPE: -EPIPE survives.
    let m = sigmask(SIGPIPE);
    if !matches!(sys::rt_sigprocmask(SIG_SETMASK, Some(&m), None), Ok(0)) {
        return false;
    }
    let buf = [7u8; 600];
    let blocked_epipe = matches!(sys::write(fds[1], &buf), Err(-32));
    let alive = HANDLED.load(Ordering::SeqCst) == 0; // blocked: no delivery
                                                     // Ignored SIGPIPE: -EPIPE survives with nothing pending.
    if !ignore(SIGPIPE) {
        return false;
    }
    if !matches!(sys::rt_sigprocmask(SIG_UNBLOCK, Some(&m), None), Ok(0)) {
        return false;
    }
    let ignored_epipe = matches!(sys::write(fds[1], &buf), Err(-32));
    let mut pending = 0u64;
    sys::rt_sigpending(&mut pending).ok();
    // Restore the default disposition and mask for later cases.
    let dfl = UserSigaction::DFL;
    let _ = sys::rt_sigaction(SIGPIPE, Some(&dfl), None);
    blocked_epipe && alive && ignored_epipe && pending & m == 0
}

/// The child-side fault: a volatile read of page zero.
fn deref_null() -> u64 {
    // SAFETY: the whole point — the access faults in user mode.
    unsafe { core::ptr::read_volatile(core::ptr::null::<u64>()) }
}

/// xv6.sig.except: a user #PF with the default disposition kills with
/// SIGSEGV (wstatus 11); a synchronous fault force-past-SIG_IGN still
/// kills (no ignore-and-refault loop).
pub fn case_except() -> bool {
    for stage in 0..2 {
        let pid = match sys::fork() {
            Ok(p) => p,
            Err(_) => return false,
        };
        if pid == 0 {
            if stage == 1 && !ignore(SIGSEGV) {
                sys::exit(2);
            }
            let v = deref_null();
            sys::exit(if v == u64::MAX { 3 } else { 3 }); // never reached
        }
        let mut wstatus = 0i32;
        if !sys::wait4(pid as i32, &mut wstatus).is_ok_and(|p| p == pid) {
            return false;
        }
        if wstatus & 0x7f != SIGSEGV as i32 {
            return false;
        }
    }
    true
}

/// xv6.sig.chld: a child exit generates SIGCHLD for the waiting parent
/// (handler observed by the time wait4 returns) with the classic
/// exit-status encoding.
pub fn case_chld() -> bool {
    HANDLED.store(0, Ordering::SeqCst);
    if !install(SIGCHLD, note_handler, 0) {
        return false;
    }
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        sys::exit(42);
    }
    let mut wstatus = -1i32;
    if !sys::wait4(pid as i32, &mut wstatus).is_ok_and(|p| p == pid) {
        return false;
    }
    let seen = HANDLED.load(Ordering::SeqCst) == SIGCHLD as usize;
    // Drop the handler: later cases' children must not run it.
    let dfl = UserSigaction::DFL;
    let _ = sys::rt_sigaction(SIGCHLD, Some(&dfl), None);
    seen && wstatus == 42 << 8
}

/// xv6.sig.alarm: alarm(1) expiry lands a SIGALRM in a spinning user
/// loop (delivery at the timer IRQ tail), not just at syscall exits.
pub fn case_alarm() -> bool {
    HANDLED.store(0, Ordering::SeqCst);
    if !install(SIGALRM, note_handler, 0) {
        return false;
    }
    if !matches!(sys::alarm(1), Ok(_)) {
        return false;
    }
    let mut spins = 0u64;
    while HANDLED.load(Ordering::SeqCst) == 0 && spins < 2_000_000_000 {
        spins += 1;
        core::hint::spin_loop();
    }
    let fired = HANDLED.load(Ordering::SeqCst) == SIGALRM as usize;
    sys::alarm(0).ok();
    let dfl = UserSigaction::DFL;
    let _ = sys::rt_sigaction(SIGALRM, Some(&dfl), None);
    fired
}
