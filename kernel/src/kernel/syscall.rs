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

pub use abi::{ECHILD, EINVAL, ENOENT, ENOSYS, ESRCH};

/// Linux x86-64 syscall numbers (abi crate re-export for dispatch match).
use abi::{
    SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_DUP, SYS_EXECVE, SYS_EXIT, SYS_EXIT_GROUP, SYS_FORK,
    SYS_FSTAT, SYS_GETPID, SYS_KILL, SYS_LINK, SYS_MKDIR, SYS_MKNOD, SYS_OPEN, SYS_PIPE, SYS_READ,
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
        SYS_BRK => sys_brk(a.arg64(0)),
        SYS_FORK => crate::proc::proc::fork() as i64,
        SYS_GETPID => sys_getpid(),
        SYS_EXECVE => sys_execve(tf, a.arg_usize(0), a.arg_usize(1)),
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

/// Timer hook (S3 nanosleep bookkeeping); no-op until the suite lands.
pub fn on_tick(_now: u64) {}
