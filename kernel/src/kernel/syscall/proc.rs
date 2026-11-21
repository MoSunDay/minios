use super::{ECHILD, EINVAL, ENOENT, ESRCH};
use crate::kernel::syspath;
use crate::kernel::traps::Trapframe;
use crate::mm::uaccess;

pub(super) fn sys_getpid() -> i64 {
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
pub(super) fn sys_wait4(pid: i32, wstatus: Option<usize>, rusage: Option<usize>) -> i64 {
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
pub(super) fn sys_execve(tf: &mut Trapframe, path: usize, argv: usize) -> i64 {
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
    // m4r-s2b: the VFS-glue walk; Err folds to the legacy miss.
    let ip = match crate::fs::vfs_glue::kern_namei_c(name.as_ptr()) {
        Ok(ip) => ip,
        Err(_) => return ENOENT,
    };
    unsafe { crate::fs::xv6fs::inode::ilock(ip) };
    let fsize = unsafe { (*ip).size } as usize;
    let read = |dst: &mut [u8], off: u32| unsafe {
        crate::fs::xv6fs::inode::readi(ip, dst.as_mut_ptr(), off, dst.len() as u32)
    };
    // SAFETY: p is the current process; exec_elf swaps CR3 itself and
    // returns with the image either replaced or the caller's kept.
    let r = unsafe { crate::proc::exec::exec_elf(p, tf, &read, fsize, &args[..nargs]) };
    unsafe { crate::fs::xv6fs::inode::iunlockput(ip) };
    match r {
        Err(e) => e,
        // POSIX exec success point (m4r-s2b): descriptors marked
        // FD_CLOEXEC close now; the new image sees the survivors.
        Ok(()) => unsafe {
            for fd in 0..(*p).ofile.len() {
                if (*p).ofile_flags[fd] as u64 & abi::FD_CLOEXEC != 0 {
                    let f = (*p).ofile[fd];
                    (*p).ofile[fd] = core::ptr::null_mut();
                    (*p).ofile_flags[fd] = 0;
                    crate::fs::file_table::fileclose(f);
                }
            }
            0
        },
    }
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
pub(super) fn sys_sched_setscheduler(pid: i32, policy: i64, param: usize) -> i64 {
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
pub(super) fn sys_sched_getscheduler(pid: i32) -> i64 {
    let p = sched_target(pid);
    if p.is_null() {
        return ESRCH;
    }
    // SAFETY: stable enum field read.
    unsafe { (*p).policy as i64 }
}

/// sched_setparam(2): set the RT priority, keeping the class.
pub(super) fn sys_sched_setparam(pid: i32, param: usize) -> i64 {
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
pub(super) fn sys_sched_getparam(pid: i32, param: usize) -> i64 {
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
pub(super) fn sys_sched_prio_bound(policy: i64, max: bool) -> i64 {
    match crate::proc::SchedPolicy::from_i64(policy) {
        Some(crate::proc::SchedPolicy::Normal) => 0,
        Some(_) if max => crate::proc::rt::RT_PRIO_MAX as i64,
        Some(_) => crate::proc::rt::RT_PRIO_MIN as i64,
        None => EINVAL,
    }
}
