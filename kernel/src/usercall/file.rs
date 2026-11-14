//! file.rs: xv6 sysfile.c part 1 — fd helpers and the I/O syscalls
//! (dup/read/write/close/fstat/exec/pipe). Path-based syscalls live in
//! fsops.rs (split for the 400-line file limit).

use core::mem::{size_of, zeroed};
use core::ptr::null_mut;

use crate::arch::x86::mm::pgtable as vm;
use crate::fs::file_table::{fileclose, filedup, fileread, filestat, filewrite, File};
use crate::fs::Stat;
use crate::ipc::pipe::pipealloc;
use crate::kernel::exec::exec;
use crate::kernel::sched::{myproc, MAXARG};
use crate::usercall::{argint, argptr, argstr, fetchint, fetchstr};

/// Fetch the nth word-sized system call argument as a file descriptor and
/// return both the descriptor and the corresponding struct file.
unsafe fn argfd(n: i32, pfd: *mut i32, pf: *mut *mut File) -> i32 {
    let mut fd: i32 = 0;
    if unsafe { argint(n, &mut fd) } < 0 {
        return -1;
    }
    if fd < 0 || fd as usize >= crate::fs::NOFILE {
        return -1;
    }
    // SAFETY: myproc's fd table, fd bounds-checked above.
    let f = unsafe { (*myproc()).ofile[fd as usize] };
    if f.is_null() {
        return -1;
    }
    if !pfd.is_null() {
        // SAFETY: caller-supplied live slot (often null to skip).
        unsafe { *pfd = fd };
    }
    if !pf.is_null() {
        // SAFETY: see above.
        unsafe { *pf = f };
    }
    0
}

/// Allocate a file descriptor for the given file. Takes over file
/// reference from caller on success.
pub(crate) unsafe fn fdalloc(f: *mut File) -> i32 {
    let p = myproc();
    for fd in 0..crate::fs::NOFILE {
        // SAFETY: myproc's fd table.
        if unsafe { (*p).ofile[fd] }.is_null() {
            unsafe { (*p).ofile[fd] = f };
            return fd as i32;
        }
    }
    -1
}

pub unsafe fn sys_dup() -> i32 {
    let mut f: *mut File = null_mut();
    if unsafe { argfd(0, null_mut(), &mut f) } < 0 {
        return -1;
    }
    let fd = unsafe { fdalloc(f) };
    if fd < 0 {
        return -1;
    }
    // SAFETY: f is a live open file.
    unsafe { filedup(f) };
    fd
}

pub unsafe fn sys_read() -> i32 {
    let mut f: *mut File = null_mut();
    let mut n: i32 = 0;
    let mut p: *mut u8 = null_mut();
    if unsafe { argfd(0, null_mut(), &mut f) } < 0
        || unsafe { argint(2, &mut n) } < 0
        || unsafe { argptr(1, &mut p, n) } < 0
    {
        return -1;
    }
    unsafe { fileread(f, p, n) }
}

pub unsafe fn sys_write() -> i32 {
    let mut f: *mut File = null_mut();
    let mut n: i32 = 0;
    let mut p: *mut u8 = null_mut();
    if unsafe { argfd(0, null_mut(), &mut f) } < 0
        || unsafe { argint(2, &mut n) } < 0
        || unsafe { argptr(1, &mut p, n) } < 0
    {
        return -1;
    }
    unsafe { filewrite(f, p, n) }
}

pub unsafe fn sys_close() -> i32 {
    let mut fd: i32 = 0;
    let mut f: *mut File = null_mut();
    if unsafe { argfd(0, &mut fd, &mut f) } < 0 {
        return -1;
    }
    let p = myproc();
    // SAFETY: myproc's fd table; the slot was validated by argfd.
    unsafe {
        (*p).ofile[fd as usize] = null_mut();
        fileclose(f);
    }
    0
}

pub unsafe fn sys_fstat() -> i32 {
    let mut f: *mut File = null_mut();
    let mut st: *mut u8 = null_mut(); // user struct stat*
    if unsafe { argfd(0, null_mut(), &mut f) } < 0
        || unsafe { argptr(1, &mut st, size_of::<Stat>() as i32) } < 0
    {
        return -1;
    }
    // Fill a kernel Stat, then copy it out to the user pointer (xv6 lets
    // stati() write user memory directly; we go through copyout).
    // SAFETY: Stat is plain data; zeroed is valid.
    let mut kst: Stat = unsafe { zeroed() };
    if unsafe { filestat(f, &mut kst) } < 0 {
        return -1;
    }
    let p = myproc();
    // SAFETY: st was argptr-validated for sizeof(Stat) bytes within proc->sz.
    if unsafe {
        vm::copyout(
            (*p).pgdir,
            st as u32,
            &kst as *const Stat as *const u8,
            size_of::<Stat>() as u32,
        )
    } < 0
    {
        return -1;
    }
    0
}

pub unsafe fn sys_exec() -> i32 {
    let mut path: *const u8 = null_mut();
    let mut uargv: i32 = 0;
    if unsafe { argstr(0, &mut path) } < 0 || unsafe { argint(1, &mut uargv) } < 0 {
        return -1;
    }
    let mut argv: [*const u8; MAXARG] = [null_mut(); MAXARG];
    let mut i: usize = 0;
    loop {
        if i >= argv.len() {
            return -1;
        }
        let mut uarg: i32 = 0;
        if unsafe { fetchint(uargv as u32 + 4 * i as u32, &mut uarg) } < 0 {
            return -1;
        }
        if uarg == 0 {
            argv[i] = null_mut();
            break;
        }
        let mut s: *const u8 = null_mut();
        if unsafe { fetchstr(uarg as u32, &mut s) } < 0 {
            return -1;
        }
        argv[i] = s;
        i += 1;
    }
    unsafe { exec(path, &argv) }
}

pub unsafe fn sys_pipe() -> i32 {
    let mut fd: *mut u8 = null_mut(); // user int fd[2]
    if unsafe { argptr(0, &mut fd, 2 * 4) } < 0 {
        return -1;
    }
    let mut rf: *mut File = null_mut();
    let mut wf: *mut File = null_mut();
    if unsafe { pipealloc(&mut rf, &mut wf) } < 0 {
        return -1;
    }
    let fd0 = unsafe { fdalloc(rf) };
    let fd1 = if fd0 < 0 { -1 } else { unsafe { fdalloc(wf) } };
    if fd0 < 0 || fd1 < 0 {
        let p = myproc();
        //SAFETY: roll back whatever fdalloc claimed before failing.
        unsafe {
            if fd0 >= 0 {
                (*p).ofile[fd0 as usize] = null_mut();
            }
            fileclose(rf);
            fileclose(wf);
        }
        return -1;
    }
    //SAFETY: fd points at 8 validated user bytes; user pgdir is active.
    unsafe {
        *(fd as *mut i32) = fd0;
        *(fd.add(4) as *mut i32) = fd1;
    }
    0
}
