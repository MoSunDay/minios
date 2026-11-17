//! Syscall dispatch hub. Numbers follow Linux 2.6.39.4 x86-64 (02 §ABI);
//! arguments arrive in the kernel ABI order rdi rsi rdx r10 r8 r9 (rcx is
//! clobbered by sysretq). Returns in -4095..-1 are errnos (04 §3).
//! Handlers without a runtime caller yet (S3 user chain) stay minimal but
//! honest: execve reports -ENOENT (implemented, no image source until S3),
//! everything unclaimed reports -ENOSYS.

use super::sysarg::SyscallArgs;
use super::traps::Trapframe;

pub const ENOENT: i64 = -2;
pub const ESRCH: i64 = -3;
pub const ECHILD: i64 = -10;
pub const EBADF: i64 = -9;
pub const ENOSYS: i64 = -38;

/// Linux x86-64 syscall numbers wired in M1R S2.
pub const SYS_WRITE: u64 = 1;
pub const SYS_BRK: u64 = 12;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_FORK: u64 = 57;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_WAIT4: u64 = 61;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_KILL: u64 = 62;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_GETPID: u64 = 39;
#[allow(dead_code)] // S3 runtime chain
pub const SYS_EXIT_GROUP: u64 = 231;

/// C entry from the syscall stub (tf.rax = number, arg registers follow
/// the x86-64 kernel ABI: rdi rsi rdx r10 r8 r9).
pub fn dispatch(tf: &mut Trapframe) -> i64 {
    let a = SyscallArgs::new(tf.rax, tf.rdi, tf.rsi, tf.rdx, tf.r10, tf.r8, tf.r9);
    match a.nr {
        SYS_WRITE => sys_write(a.arg_usize(0), a.arg_usize(1), a.arg_usize(2)),
        SYS_BRK => sys_brk(a.arg64(0)),
        SYS_FORK => crate::proc::proc::fork() as i64,
        SYS_GETPID => sys_getpid(),
        SYS_EXECVE => ENOENT, // implemented; no image source until the S3 user chain
        SYS_EXIT | SYS_EXIT_GROUP => {
            crate::proc::proc::exit(a.arg_i32(0) & 0xff);
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

/// write(2) v1: fd 1/2 only, byte stream to the serial console; the buffer
/// is a user pointer copied through the fixup-guarded copyin.
fn sys_write(fd: usize, buf: usize, count: usize) -> i64 {
    if fd != 1 && fd != 2 {
        return EBADF;
    }
    let src = buf as *const u8;
    let mut chunk = [0u8; 128];
    let mut total = 0usize;
    while total < count {
        let n = core::cmp::min(chunk.len(), count - total);
        // SAFETY: dst is a kernel stack buffer; a fault recovers as EFAULT.
        let r = crate::mm::uaccess::copyin(chunk.as_mut_ptr(), src.wrapping_add(total), n);
        if r != 0 {
            return if total == 0 { r } else { total as i64 };
        }
        for &b in &chunk[..n] {
            crate::arch::x86_64::console::write_byte(b);
        }
        total += n;
    }
    total as i64
}

/// brk(2): addr 0 queries the current break; otherwise grow/shrink to the
/// page-rounded addr, returning the new break or the old one on failure.
fn sys_brk(addr: u64) -> i64 {
    // Sanity ceiling mirroring proc::growproc's HEAP_LIMIT (0x1000_0000_0000).
    const HEAP_LIMIT: u64 = 0x0000_1000_0000_0000;
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: myproc() is the running (or seeded) process.
    let old = unsafe { (*p).mm.as_ref().map_or(0, |m| m.sz) } as i64;
    if addr == 0 {
        return old;
    }
    if addr >= HEAP_LIMIT {
        return old;
    }
    let want = addr as i64;
    if want == old {
        return old;
    }
    let r = crate::proc::proc::growproc(want - old);
    if r as i64 == want {
        want
    } else {
        old
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
        let r = crate::mm::uaccess::copyout(dst as *mut u8, &xstate as *const i32 as *const u8, 4);
        if r != 0 {
            return r;
        }
    }
    reaped as i64
}

/// Timer hook (S3 nanosleep bookkeeping); no-op until the suite lands.
pub fn on_tick(_now: u64) {}
