//! Syscall dispatch hub. Numbers and errno values come from the shared
//! abi crate (Linux 2.6.39.4 x86-64, plans/linux26/02 §5); arguments
//! arrive in the kernel ABI order rdi rsi rdx r10 r8 r9 (rcx is clobbered
//! by sysretq). Returns in -4095..-1 are errnos (04 §3). Numbers outside
//! the whitelist answer -ENOSYS; S4d-2 execve streams the image from the
//! on-disk inode (the S3 embedded-table source is retired; the embedded
//! copy remains only as pid-1 seed and byte-compare reference).

mod mm;
mod proc;
mod time;

use super::sysarg::SyscallArgs;
use super::traps::Trapframe;
use super::{sysfile, syspath};

pub use abi::{ECHILD, EINTR, EINVAL, ENOENT, ENOMEM, ENOSYS, ESRCH};
pub use proc::on_tick;

/// Linux x86-64 syscall numbers (abi crate re-export for dispatch match).
use abi::{
    SYS_ALARM, SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_DUP, SYS_EXECVE, SYS_EXIT, SYS_EXIT_GROUP,
    SYS_FORK, SYS_FSTAT, SYS_GETPID, SYS_GETTIMEOFDAY, SYS_KILL, SYS_LINK, SYS_MKDIR, SYS_MKNOD,
    SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP, SYS_NANOSLEEP, SYS_OPEN, SYS_PIPE, SYS_READ,
    SYS_RT_SIGACTION, SYS_RT_SIGPENDING, SYS_RT_SIGPROCMASK, SYS_RT_SIGRETURN, SYS_SCHED_GETPARAM,
    SYS_SCHED_GETSCHEDULER, SYS_SCHED_GET_PRIORITY_MAX, SYS_SCHED_GET_PRIORITY_MIN,
    SYS_SCHED_SETPARAM, SYS_SCHED_SETSCHEDULER, SYS_SCHED_YIELD, SYS_UNLINK, SYS_WAIT4, SYS_WRITE,
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
        SYS_NANOSLEEP => time::sys_nanosleep(a.arg_usize(0)),
        SYS_GETTIMEOFDAY => time::sys_gettimeofday(a.arg_opt_ptr(0)),
        SYS_ALARM => time::sys_alarm(a.arg_i64(0)),
        SYS_SCHED_YIELD => {
            crate::proc::sched::sched_yield();
            0
        }
        SYS_SCHED_SETSCHEDULER => {
            proc::sys_sched_setscheduler(a.arg_i32(0), a.arg_i64(1), a.arg_usize(2))
        }
        SYS_SCHED_GETSCHEDULER => proc::sys_sched_getscheduler(a.arg_i32(0)),
        SYS_SCHED_SETPARAM => proc::sys_sched_setparam(a.arg_i32(0), a.arg_usize(1)),
        SYS_SCHED_GETPARAM => proc::sys_sched_getparam(a.arg_i32(0), a.arg_usize(1)),
        SYS_SCHED_GET_PRIORITY_MAX => proc::sys_sched_prio_bound(a.arg_i64(0), true),
        SYS_SCHED_GET_PRIORITY_MIN => proc::sys_sched_prio_bound(a.arg_i64(0), false),
        SYS_BRK => mm::sys_brk(a.arg64(0)),
        SYS_MMAP => mm::sys_mmap(
            a.arg_usize(0),
            a.arg_usize(1),
            a.arg64(2) as i64,
            a.arg64(3) as i64,
            a.arg64(4) as i64,
            a.arg64(5) as i64,
        ),
        SYS_MUNMAP => mm::sys_munmap(a.arg_usize(0), a.arg_usize(1)),
        SYS_MPROTECT => mm::sys_mprotect(a.arg_usize(0), a.arg_usize(1), a.arg64(2) as i64),
        SYS_FORK => crate::proc::proc::fork() as i64,
        SYS_GETPID => proc::sys_getpid(),
        SYS_EXECVE => proc::sys_execve(tf, a.arg_usize(0), a.arg_usize(1)),
        SYS_EXIT | SYS_EXIT_GROUP => {
            // Full i32: the suite driver encodes (total<<8)|failed (S4e-2).
            crate::proc::proc::exit(a.arg_i32(0));
        }
        SYS_WAIT4 => proc::sys_wait4(a.wait_pid(0), a.arg_opt_ptr(1), a.arg_opt_ptr(3)),
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
