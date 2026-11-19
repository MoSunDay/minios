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
    MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_DUP, SYS_EXECVE,
    SYS_EXIT, SYS_EXIT_GROUP, SYS_FORK, SYS_FSTAT, SYS_GETPID, SYS_KILL, SYS_LINK, SYS_MKDIR,
    SYS_MKNOD, SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP, SYS_NANOSLEEP, SYS_OPEN, SYS_PIPE, SYS_READ,
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
        SYS_WAIT4 => sys_wait4(a.wait_pid(0), a.arg_opt_ptr(1)),
        SYS_KILL => {
            if crate::proc::proc::kill(a.arg_i32(0)) == 0 {
                0
            } else {
                ESRCH
            }
        }
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
/// optional user wstatus pointer (fixup-guarded copyout).
fn sys_wait4(pid: i32, wstatus: Option<usize>) -> i64 {
    let (reaped, xstate) = crate::proc::proc::wait_status(pid);
    if reaped < 0 {
        return ECHILD;
    }
    if let Some(dst) = wstatus {
        // Linux wait status encoding: bits 8..15 carry the exit code
        // (WEXITSTATUS); the M1R kernel has no signals yet.
        let wstatus_val: i32 = (xstate & 0xff) << 8;
        let r = uaccess::copyout(dst as *mut u8, &wstatus_val as *const i32 as *const u8, 4);
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

/// nanosleep(2) (Linux x86-64 ABI 35): park on the ticks channel until
/// the deadline passes (xv6 sys_sleep loop against tickslock). No signal
/// delivery exists in M1R, so the return is always 0 on success.
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
    // 100 Hz PIT: whole ticks, at least one for any nonzero request.
    let dur = (sec as u64) * 100 + (nsec as u64) / 10_000_000;
    let deadline = crate::kernel::time::ticks() + dur.max(1);
    // SAFETY: balanced TICK_LOCK acquire/release around each sleep; the
    // channel is woken from the timer interrupt under the same lock.
    unsafe {
        loop {
            crate::kernel::time::TICK_LOCK.raw_lock();
            if crate::kernel::time::ticks() >= deadline {
                crate::kernel::time::TICK_LOCK.raw_release();
                break;
            }
            crate::proc::sleep::sleep(
                crate::kernel::time::ticks_chan(),
                &crate::kernel::time::TICK_LOCK,
            );
            // SAFETY: sleep returns with TICK_LOCK held.
            crate::kernel::time::TICK_LOCK.raw_release();
            // xv6 sys_sleep: a killed sleeper aborts (EINTR); the next
            // timer trap from user mode exits it (traps_dispatch).
            let p = crate::proc::proc::myproc();
            if !p.is_null() && (*p).killed != 0 {
                return EINTR;
            }
        }
    }
    0
}
