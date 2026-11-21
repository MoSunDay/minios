//! Syscall dispatch hub. Numbers and errno values come from the shared
//! abi crate (Linux 2.6.39.4 x86-64, plans/linux26/02 §5); arguments
//! arrive in the kernel ABI order rdi rsi rdx r10 r8 r9 (rcx is clobbered
//! by sysretq). Returns in -4095..-1 are errnos (04 §3). Numbers outside
//! the whitelist answer -ENOSYS; S4d-2 execve streams the image from the
//! on-disk inode (the S3 embedded-table source is retired; the embedded
//! copy remains only as pid-1 seed and byte-compare reference).

use super::sysarg::SyscallArgs;
use super::traps::Trapframe;
use super::{sysfile, syspath};
use crate::mm::uaccess;

pub use abi::{ECHILD, EINTR, EINVAL, ENOENT, ENOMEM, ENOSYS, ESRCH};

/// Linux x86-64 syscall numbers (abi crate re-export for dispatch match).
use abi::{
    MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, SYS_ALARM, SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_DUP,
    SYS_EXECVE, SYS_EXIT, SYS_EXIT_GROUP, SYS_FORK, SYS_FSTAT, SYS_GETPID, SYS_GETTIMEOFDAY,
    SYS_KILL, SYS_LINK, SYS_MKDIR, SYS_MKNOD, SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP, SYS_NANOSLEEP,
    SYS_OPEN, SYS_PIPE, SYS_READ, SYS_RT_SIGACTION, SYS_RT_SIGPENDING, SYS_RT_SIGPROCMASK,
    SYS_RT_SIGRETURN, SYS_SCHED_GETPARAM, SYS_SCHED_GETSCHEDULER, SYS_SCHED_GET_PRIORITY_MAX,
    SYS_SCHED_GET_PRIORITY_MIN, SYS_SCHED_SETPARAM, SYS_SCHED_SETSCHEDULER, SYS_SCHED_YIELD,
    SYS_UNLINK, SYS_WAIT4, SYS_WRITE,
};

/// C entry from the syscall stub (tf.rax = number, arg registers follow
/// the x86-64 kernel ABI: rdi rsi rdx r10 r8 r9).
pub fn dispatch(tf: &mut Trapframe) -> i64 {
    let a = SyscallArgs::new(tf.rax, tf.rdi, tf.rsi, tf.rdx, tf.r10, tf.r8, tf.r9);
    match a.nr {
        SYS_READ => sysfile::sys_read(a.arg_usize(0), a.arg_usize(1), a.arg64(2)),
        SYS_WRITE => sysfile::sys_write(a.arg_usize(0), a.arg_usize(1), a.arg64(2)),
        SYS_OPEN => syspath::sys_open(a.arg_usize(0), a.arg64(1)),
        SYS_CLOSE => sysfile::sys_close(a.arg_usize(0)),
        SYS_DUP => sysfile::sys_dup(a.arg_usize(0)),
        SYS_PIPE => sysfile::sys_pipe(a.arg_usize(0)),
        SYS_FSTAT => sysfile::sys_fstat(a.arg_usize(0), a.arg_usize(1)),
        SYS_MKNOD => syspath::sys_mknod(a.arg_usize(0), a.arg64(1), a.arg64(2)),
        SYS_MKDIR => syspath::sys_mkdir(a.arg_usize(0)),
        SYS_CHDIR => syspath::sys_chdir(a.arg_usize(0)),
        SYS_LINK => syspath::sys_link(a.arg_usize(0), a.arg_usize(1)),
        SYS_UNLINK => syspath::sys_unlink(a.arg_usize(0)),
        SYS_NANOSLEEP => sys_nanosleep(a.arg_usize(0)),
        SYS_GETTIMEOFDAY => sys_gettimeofday(a.arg_opt_ptr(0)),
        SYS_ALARM => sys_alarm(a.arg_i64(0)),
        SYS_SCHED_YIELD => {
            crate::proc::sched::sched_yield();
            0
        }
        SYS_SCHED_SETSCHEDULER => {
            sys_sched_setscheduler(a.arg_i32(0), a.arg_i64(1), a.arg_usize(2))
        }
        SYS_SCHED_GETSCHEDULER => sys_sched_getscheduler(a.arg_i32(0)),
        SYS_SCHED_SETPARAM => sys_sched_setparam(a.arg_i32(0), a.arg_usize(1)),
        SYS_SCHED_GETPARAM => sys_sched_getparam(a.arg_i32(0), a.arg_usize(1)),
        SYS_SCHED_GET_PRIORITY_MAX => sys_sched_prio_bound(a.arg_i64(0), true),
        SYS_SCHED_GET_PRIORITY_MIN => sys_sched_prio_bound(a.arg_i64(0), false),
        SYS_BRK => sys_brk(a.arg64(0)),
        SYS_MMAP => sys_mmap(
            a.arg_usize(0),
            a.arg_usize(1),
            a.arg64(2) as i64,
            a.arg64(3) as i64,
            a.arg64(4) as i64,
            a.arg64(5) as i64,
        ),
        SYS_MUNMAP => sys_munmap(a.arg_usize(0), a.arg_usize(1)),
        SYS_MPROTECT => sys_mprotect(a.arg_usize(0), a.arg_usize(1), a.arg64(2) as i64),
        SYS_FORK => crate::proc::proc::fork() as i64,
        SYS_GETPID => sys_getpid(),
        SYS_EXECVE => sys_execve(tf, a.arg_usize(0), a.arg_usize(1)),
        SYS_EXIT | SYS_EXIT_GROUP => {
            // Full i32: the suite driver encodes (total<<8)|failed (S4e-2).
            crate::proc::proc::exit(a.arg_i32(0));
        }
        SYS_WAIT4 => sys_wait4(a.wait_pid(0), a.arg_opt_ptr(1), a.arg_opt_ptr(3)),
        SYS_KILL => super::signal::sys_kill(a.arg_i32(0), a.arg_i64(1)),
        SYS_RT_SIGACTION => {
            super::signal::sys_rt_sigaction(a.arg_i64(0), a.arg_opt_ptr(1), a.arg_opt_ptr(2))
        }
        SYS_RT_SIGPROCMASK => super::signal::sys_rt_sigprocmask(
            a.arg_i64(0),
            a.arg_opt_ptr(1),
            a.arg_opt_ptr(2),
            a.arg64(3),
        ),
        SYS_RT_SIGRETURN => super::signal::sys_rt_sigreturn(tf),
        SYS_RT_SIGPENDING => super::signal::sys_rt_sigpending(a.arg_usize(0), a.arg64(1)),
        _ => ENOSYS,
    }
}

/// brk(2): addr 0 queries the current break; otherwise move it to the
/// byte-exact addr, returning the new break or the old one on failure.
fn sys_brk(addr: u64) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: myproc() is the running (or seeded) process.
    let mm = match unsafe { (*p).mm.as_mut() } {
        Some(m) => m,
        None => return 0,
    };
    if addr == 0 {
        return mm.brk as i64;
    }
    mm.set_brk(addr as usize) as i64
}

/// mmap(2) M2 anonymous subset: MAP_ANONYMOUS|MAP_PRIVATE only (fd
/// ignored, offset must be 0); addr 0 places top-down, MAP_FIXED pins.
/// Returns the mapping start or -errno.
fn sys_mmap(addr: usize, len: usize, prot: i64, flags: i64, _fd: i64, off: i64) -> i64 {
    const FLAG_BITS: i64 = (MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED) as i64;
    const PROT_BITS: i64 = (abi::PROT_READ | abi::PROT_WRITE | abi::PROT_EXEC) as i64;
    if flags & (MAP_PRIVATE as i64) == 0 || flags & (MAP_ANONYMOUS as i64) == 0 {
        return EINVAL;
    }
    if (flags & !FLAG_BITS) != 0 {
        return EINVAL;
    }
    if off != 0 {
        return EINVAL;
    }
    if (prot & !PROT_BITS) != 0 {
        return EINVAL;
    }
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return ENOMEM;
    }
    let kprot = (prot & PROT_BITS) as u32;
    // vma.rs PROT_* bit values match the abi's (1, 2, 4).
    let fixed = flags & (MAP_FIXED as i64) != 0;
    // SAFETY: myproc() is the running process.
    let r = match unsafe { (*p).mm.as_mut() } {
        Some(mm) => mm.mmap_anon(addr, len, kprot, fixed),
        None => 0,
    };
    if r == 0 {
        ENOMEM
    } else {
        r as i64
    }
}

/// munmap(2): anonymous mappings only (M2 minimal set); 0 or -errno.
fn sys_munmap(addr: usize, len: usize) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return EINVAL;
    }
    // SAFETY: myproc() is the running process.
    match unsafe { (*p).mm.as_mut() } {
        Some(mm) => {
            if mm.munmap(addr, len) {
                0
            } else {
                EINVAL
            }
        }
        None => EINVAL,
    }
}

/// mprotect(2): 0 or -errno (EINVAL on bad prot, ENOMEM on holes).
fn sys_mprotect(addr: usize, len: usize, prot: i64) -> i64 {
    const PROT_BITS: i64 = (abi::PROT_READ | abi::PROT_WRITE | abi::PROT_EXEC) as i64;
    if (prot & !PROT_BITS) != 0 {
        return EINVAL;
    }
    let kprot = (prot & PROT_BITS) as u32;
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return EINVAL;
    }
    // SAFETY: myproc() is the running process.
    match unsafe { (*p).mm.as_mut() } {
        Some(mm) => {
            if mm.mprotect(addr, len, kprot) {
                0
            } else {
                ENOMEM
            }
        }
        None => ENOMEM,
    }
}

fn sys_getpid() -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: reading the pid of the running process.
    unsafe { (*p).pid as i64 }
}

/// wait4(2): reap one matching child; write the exit status through the
/// optional user wstatus pointer and the child's CPU accounting through
/// the optional user rusage pointer (both fixup-guarded copyouts).
fn sys_wait4(pid: i32, wstatus: Option<usize>, rusage: Option<usize>) -> i64 {
    let (reaped, xstate, exec_ns) = crate::proc::proc::wait_status(pid);
    if reaped == -2 {
        return abi::ERESTARTSYS;
    }
    if reaped < 0 {
        return ECHILD;
    }
    if let Some(dst) = wstatus {
        // Linux wait status encoding: bits 8..15 carry the exit code
        // (WEXITSTATUS); a signal death (signal::exit_status_for_signal,
        // xstate = -256-sig) reports the raw signal number (WIFSIGNALED).
        // The legacy xv6 kill mark (-1) keeps its 0xff exit-code image.
        let wstatus_val: i32 = if xstate <= -2 {
            (-xstate) & 0x7f
        } else {
            (xstate & 0xff) << 8
        };
        let r = uaccess::copyout(dst as *mut u8, &wstatus_val as *const i32 as *const u8, 4);
        if r != 0 {
            return r;
        }
    }
    if let Some(dst) = rusage {
        // Linux x86-64 struct rusage: ru_utime + ru_stime timeval pairs
        // (16 B each) then 16 i64 fields = 144 B total. ru_utime carries
        // the reaped child's scheduler-charged CPU ns; ru_stime and the
        // counters stay 0 until accounting splits user/system (M8+).
        let sec = (exec_ns / 1_000_000_000) as i64;
        let usec = ((exec_ns % 1_000_000_000) / 1_000) as i64;
        let mut ru = [0u8; 144];
        ru[0..8].copy_from_slice(&sec.to_le_bytes());
        ru[8..16].copy_from_slice(&usec.to_le_bytes());
        let r = uaccess::copyout(dst as *mut u8, ru.as_ptr(), 144);
        if r != 0 {
            return r;
        }
    }
    reaped as i64
}

/// execve(2), S3 image source: resolve the NUL-terminated path against
/// the embedded table, fetch the NULL-terminated argv pointer array, and
/// replace the current image. envp is ignored (empty environment). A
/// failed exec keeps the caller's image (Linux/xv6 contract).
fn sys_execve(tf: &mut Trapframe, path: usize, argv: usize) -> i64 {
    const MAX_ARGS: usize = 8;
    const ARG_MAX: usize = 64;

    let mut name = [0u8; syspath::MAXPATH];
    match syspath::copy_path(&mut name, path) {
        0 => {}
        e => return e,
    }

    if argv == 0 {
        return EINVAL;
    }
    let mut args_buf = [[0u8; ARG_MAX]; MAX_ARGS];
    let mut args: [&[u8]; MAX_ARGS] = [b""; MAX_ARGS];
    let mut nargs = 0usize;
    loop {
        let mut ptr_bytes = [0u8; 8];
        let r = uaccess::copyin(ptr_bytes.as_mut_ptr(), (argv + nargs * 8) as *const u8, 8);
        if r != 0 {
            return r;
        }
        let arg_ptr = u64::from_le_bytes(ptr_bytes) as usize;
        if arg_ptr == 0 {
            break;
        }
        if nargs == MAX_ARGS {
            return EINVAL; // E2BIG-class overflow folded into EINVAL in M1R
        }
        match uaccess::copyinstr(&mut args_buf[nargs], arg_ptr as *const u8) {
            0 => {}
            e => return e,
        }
        let len = args_buf[nargs].iter().position(|&b| b == 0).unwrap_or(0);
        // SAFETY: args_buf outlives exec_elf (segments + stack are built
        // from these bytes before it returns).
        args[nargs] = unsafe { core::slice::from_raw_parts(args_buf[nargs].as_ptr(), len) };
        nargs += 1;
    }

    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return ENOENT;
    }
    // S4d-2: resolve the path in the on-disk fs and stream the ELF from
    // the locked inode (xv6 sysfile.c exec: namei -> ilock -> readi ->
    // iunlockput; read-only, so no log transaction).
    // SAFETY: cpath is NUL-terminated; the inode protocol is balanced on
    // every path and the reader runs only while the lock is held.
    let ip = unsafe { crate::fs::xv6fs::namei::namei(name.as_ptr()) };
    if ip.is_null() {
        return ENOENT;
    }
    unsafe { crate::fs::xv6fs::inode::ilock(ip) };
    let fsize = unsafe { (*ip).size } as usize;
    let read = |dst: &mut [u8], off: u32| unsafe {
        crate::fs::xv6fs::inode::readi(ip, dst.as_mut_ptr(), off, dst.len() as u32)
    };
    // SAFETY: p is the current process; exec_elf swaps CR3 itself and
    // returns with the image either replaced or the caller's kept.
    let r = unsafe { crate::proc::exec::exec_elf(p, tf, &read, fsize, &args[..nargs]) };
    unsafe { crate::fs::xv6fs::inode::iunlockput(ip) };
    r.map_or_else(|e| e, |()| 0)
}

/// Timer hook (S3 bookkeeping); sleep wakeups live in time::tick.
pub fn on_tick(_now: u64) {}

// ---- Scheduler-policy syscalls (m3r-s2; minimal Linux semantics) ----

/// Resolve a pid to its ptable slot (0 = current); null when absent.
fn sched_target(pid: i32) -> *mut crate::proc::Proc {
    let p = crate::proc::proc::myproc();
    if pid == 0 {
        return p;
    }
    // SAFETY: the ptable lock guards the scan; slots never move.
    unsafe {
        let _guard = crate::proc::PTABLE_LOCK.lock();
        let table = crate::proc::ptable_mut();
        for slot in table.iter_mut() {
            if slot.pid == pid && slot.state != crate::proc::ProcState::UNUSED {
                return slot as *mut crate::proc::Proc;
            }
        }
    }
    core::ptr::null_mut()
}

/// Validate a (policy, priority) pair: Normal requires 0, RT requires
/// 1..=99 (Linux range).
fn sched_valid(policy: crate::proc::SchedPolicy, prio: i32) -> bool {
    match policy {
        crate::proc::SchedPolicy::Normal => prio == 0,
        _ => (crate::proc::rt::RT_PRIO_MIN..=crate::proc::rt::RT_PRIO_MAX).contains(&prio),
    }
}

/// sched_setscheduler(2): set class + priority for pid (0 = self).
fn sys_sched_setscheduler(pid: i32, policy: i64, param: usize) -> i64 {
    let Some(pol) = crate::proc::SchedPolicy::from_i64(policy) else {
        return EINVAL;
    };
    let mut buf = [0u8; 4];
    match uaccess::copyin(buf.as_mut_ptr(), param as *const u8, 4) {
        0 => {}
        e => return e,
    }
    let prio = i32::from_le_bytes(buf.try_into().unwrap());
    if !sched_valid(pol, prio) {
        return EINVAL;
    }
    let p = sched_target(pid);
    if p.is_null() {
        return ESRCH;
    }
    let _guard = crate::proc::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is a live slot.
    unsafe { crate::proc::sched::set_sched_policy(p, pol, prio) };
    0
}

/// sched_getscheduler(2): policy number for pid (0 = self).
fn sys_sched_getscheduler(pid: i32) -> i64 {
    let p = sched_target(pid);
    if p.is_null() {
        return ESRCH;
    }
    // SAFETY: stable enum field read.
    unsafe { (*p).policy as i64 }
}

/// sched_setparam(2): set the RT priority, keeping the class.
fn sys_sched_setparam(pid: i32, param: usize) -> i64 {
    let mut buf = [0u8; 4];
    match uaccess::copyin(buf.as_mut_ptr(), param as *const u8, 4) {
        0 => {}
        e => return e,
    }
    let prio = i32::from_le_bytes(buf.try_into().unwrap());
    let p = sched_target(pid);
    if p.is_null() {
        return ESRCH;
    }
    // SAFETY: stable enum field read.
    let pol = unsafe { (*p).policy };
    if !sched_valid(pol, prio) {
        return EINVAL;
    }
    let _guard = crate::proc::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is a live slot.
    unsafe { crate::proc::sched::set_sched_policy(p, pol, prio) };
    0
}

/// sched_getparam(2): write the priority out (copyout outside the lock).
fn sys_sched_getparam(pid: i32, param: usize) -> i64 {
    let p = sched_target(pid);
    if p.is_null() {
        return ESRCH;
    }
    // SAFETY: stable field read under the ptable lock.
    let prio = unsafe {
        let _guard = crate::proc::PTABLE_LOCK.lock();
        (*p).rt_prio
    };
    let r = uaccess::copyout(param as *mut u8, &prio as *const i32 as *const u8, 4);
    if r != 0 {
        return r;
    }
    0
}

/// sched_get_priority_max/min(2): RT bounds 99/1, Normal 0/0.
fn sys_sched_prio_bound(policy: i64, max: bool) -> i64 {
    match crate::proc::SchedPolicy::from_i64(policy) {
        Some(crate::proc::SchedPolicy::Normal) => 0,
        Some(_) if max => crate::proc::rt::RT_PRIO_MAX as i64,
        Some(_) => crate::proc::rt::RT_PRIO_MIN as i64,
        None => EINVAL,
    }
}

/// gettimeofday(2) (Linux x86-64 96): report the wall clock. There is
/// no RTC until M8+, so wall = boot_epoch(0) + monotonic (ADR-0019).
/// tv NULL is accepted (no copyout); tz is ignored.
fn sys_gettimeofday(tv: Option<usize>) -> i64 {
    let ns = crate::kernel::time::clocksource::wall_ns();
    let sec = (ns / 1_000_000_000) as i64;
    let usec = ((ns % 1_000_000_000) / 1_000) as i64;
    if let Some(dst) = tv {
        let mut ts = [0u8; 16];
        ts[0..8].copy_from_slice(&sec.to_le_bytes());
        ts[8..16].copy_from_slice(&usec.to_le_bytes());
        let r = uaccess::copyout(dst as *mut u8, ts.as_ptr(), 16);
        if r != 0 {
            return r;
        }
    }
    0
}

/// alarm(2) (37): arm the process alarm wheel timer; returns the whole
/// seconds (rounded up) remaining on the previous alarm, 0 if none.
/// seconds == 0 cancels. Expiry (wheel callback alarm_fire, s6) marks
/// alarm_fired and generates SIGALRM for delivery at user return.
fn sys_alarm(seconds: i64) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: myproc() is the running process; the alarm field is
    // exclusively ours and the wheel lock is a leaf (ptable not held).
    unsafe {
        let prev = if (*p).alarm.queued() {
            let rem = (*p)
                .alarm
                .expires()
                .saturating_sub(crate::kernel::time::wheel::jiffies());
            (rem + 99) / 100
        } else {
            0
        };
        if seconds < 0 {
            return EINVAL;
        }
        if seconds == 0 {
            if (*p).alarm.queued() {
                crate::kernel::time::wheel::del_timer(&mut (*p).alarm);
            }
        } else {
            // Linux retruncates oversize alarms; here the deadline clamps
            // to the wheel horizon (tv5 placement beyond it is imprecise).
            let offset = (seconds as u64)
                .saturating_mul(100)
                .min(crate::kernel::time::wheel::WHEEL_HORIZON);
            let expires = crate::kernel::time::wheel::jiffies().saturating_add(offset);
            crate::kernel::time::wheel::add_timer(
                &mut (*p).alarm,
                expires,
                crate::proc::proc::alarm_fire,
            );
        }
        prev as i64
    }
}

/// nanosleep(2) (35): park on an on-stack hrtimer until the monotonic
/// deadline. The callback must take TICK_LOCK before the wakeup so the
/// check-then-sleep window stays atomic against the expiry (a fire in
/// that window blocks on the sleeper's TICK_LOCK and its wakeup then
/// observes the sleeper — no missed wake). Cancelled on every return so
/// the on-stack timer never outlives the call; a killed sleeper aborts
/// with EINTR (xv6-visible).
fn sys_nanosleep(req: usize) -> i64 {
    let mut ts = [0u8; 16];
    match uaccess::copyin(ts.as_mut_ptr(), req as *const u8, 16) {
        0 => {}
        e => return e,
    }
    let sec = i64::from_le_bytes(ts[0..8].try_into().unwrap());
    let nsec = i64::from_le_bytes(ts[8..16].try_into().unwrap());
    if nsec < 0 || nsec >= 1_000_000_000 || sec < 0 {
        return EINVAL;
    }
    let dur_ns = (sec as u64) * 1_000_000_000 + nsec as u64;
    if dur_ns == 0 {
        return 0;
    }
    let mut timer = crate::kernel::time::hrtimer::HrTimer::new();
    let deadline = crate::kernel::time::clocksource::monotonic_ns().saturating_add(dur_ns);
    // One channel for the sleep and the callback to agree on byte-for-byte.
    let chan = (&mut timer as *mut _) as usize;
    // SAFETY: timer outlives every armed window (cancelled on all returns).
    unsafe {
        crate::kernel::time::hrtimer::hrtimer_start(
            &mut timer,
            deadline,
            crate::kernel::time::hrtimer::HrMode::Abs,
            nanosleep_wake,
        );
        let p = crate::proc::proc::myproc();
        loop {
            // SAFETY: balanced TICK_LOCK around each sleep check (same
            // protocol as the previous ticks-channel implementation).
            crate::kernel::time::TICK_LOCK.raw_lock();
            if crate::kernel::time::clocksource::monotonic_ns() >= deadline {
                crate::kernel::time::TICK_LOCK.raw_release();
                break;
            }
            // xv6 sys_sleep: a killed sleeper aborts (EINTR); the next
            // trap from user mode exits it (traps_dispatch). An
            // unblocked pending signal aborts the same way (s6): its
            // handler/default runs at the syscall return path.
            if !p.is_null() && ((*p).killed != 0 || super::signal::pending_unblocked(p)) {
                crate::kernel::time::TICK_LOCK.raw_release();
                crate::kernel::time::hrtimer::hrtimer_cancel(&mut timer);
                return EINTR;
            }
            crate::proc::sleep::sleep(chan, &crate::kernel::time::TICK_LOCK);
            // SAFETY: sleep returns with TICK_LOCK held.
            crate::kernel::time::TICK_LOCK.raw_release();
        }
        crate::kernel::time::hrtimer::hrtimer_cancel(&mut timer);
    }
    0
}

/// nanosleep wakeup (hrtimer callback): take TICK_LOCK around the
/// channel wakeup so the sleeper's check-then-sleep window stays atomic
/// against the expiry.
fn nanosleep_wake(t: *mut crate::kernel::time::hrtimer::HrTimer) {
    // Balanced TICK_LOCK acquire/release (timer-interrupt context; the
    // discipline matches time::tick's own wakeup).
    // SAFETY: same lock discipline as time::tick.
    unsafe {
        crate::kernel::time::TICK_LOCK.raw_lock();
        crate::proc::sleep::wakeup(t as usize);
        crate::kernel::time::TICK_LOCK.raw_release();
    }
}
