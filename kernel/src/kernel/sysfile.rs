//! sysfile.rs: the fd-I/O half of xv6 sysfile.c — read/write/close/dup/
//! fstat/pipe on the S4 file table. S3's write(2) fd-1/2 shortcut is
//! gone: descriptors rule, and the suite driver opens its console like
//! xv6's init.c (mknod + open + dup).
//!
//! Deviations from xv6, both deliberate:
//! * user buffers bounce through the fixup-guarded uaccess copies, so a
//!   bad pointer returns -EFAULT instead of faulting inside readi/writei;
//! * xv6's blanket -1 failures are spelled as Linux errnos (02 §5):
//!   EBADF bad fd, EMFILE fdalloc/filealloc exhaustion, EIO device/file
//!   layer refusal, EFAULT copy faults (partial counts preserved).

use core::mem::size_of;

use crate::fs::file_table::{fileclose, filedup, fileread, filestat, filewrite, File};
use crate::fs::xv6fs::layout::{Stat, BSIZE};
use crate::ipc::pipe::pipealloc;
use crate::mm::uaccess;
use crate::proc::proc::myproc;
use crate::proc::NOFILE;

use abi::{EBADF, EINVAL, EIO, EMFILE};

use super::syspath::fdalloc;

/// read(2): fd-table dispatch with a per-block kernel bounce.
pub fn sys_read(fd: usize, buf: usize, count: u64) -> i64 {
    if count > i32::MAX as u64 {
        return EINVAL;
    }
    let f = match fd_file(fd) {
        Some(f) => f,
        None => return EBADF,
    };
    let mut chunk = [0u8; BSIZE];
    let mut total: u32 = 0;
    // SAFETY: chunk is a kernel bounce; copyout faults as EFAULT.
    unsafe {
        while total < count as u32 {
            let want = core::cmp::min(BSIZE as u32, count as u32 - total);
            let r = fileread(f, chunk.as_mut_ptr(), want as i32);
            if r < 0 {
                return if total > 0 { total as i64 } else { EIO };
            }
            if r > 0 {
                let rc = uaccess::copyout(
                    (buf + total as usize) as *mut u8,
                    chunk.as_ptr(),
                    r as usize,
                );
                if rc != 0 {
                    return if total > 0 { total as i64 } else { rc };
                }
                total += r as u32;
            }
            if (r as u32) < want {
                break; // short read (EOF)
            }
        }
    }
    total as i64
}

/// write(2): same bounce shape as read.
pub fn sys_write(fd: usize, buf: usize, count: u64) -> i64 {
    if count > i32::MAX as u64 {
        return EINVAL;
    }
    let f = match fd_file(fd) {
        Some(f) => f,
        None => return EBADF,
    };
    let mut chunk = [0u8; BSIZE];
    let mut total: u32 = 0;
    // SAFETY: chunk is a kernel bounce; copyin faults as EFAULT.
    unsafe {
        while total < count as u32 {
            let want = core::cmp::min(BSIZE as u32, count as u32 - total);
            let rc = uaccess::copyin(
                chunk.as_mut_ptr(),
                (buf + total as usize) as *const u8,
                want as usize,
            );
            if rc != 0 {
                return if total > 0 { total as i64 } else { rc };
            }
            let r = filewrite(f, chunk.as_ptr(), want as i32);
            if r < 0 {
                return if total > 0 { total as i64 } else { EIO };
            }
            total += r as u32;
            if (r as u32) < want {
                break;
            }
        }
    }
    total as i64
}

/// close(2).
pub fn sys_close(fd: usize) -> i64 {
    // SAFETY: fd slot ownership transferred to fileclose.
    unsafe {
        let p = myproc();
        if fd >= NOFILE || (*p).ofile[fd].is_null() {
            return EBADF;
        }
        let f = (*p).ofile[fd];
        (*p).ofile[fd] = core::ptr::null_mut();
        fileclose(f);
    }
    0
}

/// dup(2) (sysfile.c sys_dup): lowest free fd, same file, offset shared.
pub fn sys_dup(fd: usize) -> i64 {
    let f = match fd_file(fd) {
        Some(f) => f,
        None => return EBADF,
    };
    // SAFETY: filedup/fdalloc balance on failure like sys_pipe's unwind.
    unsafe {
        let nf = filedup(f);
        let nfd = fdalloc(nf);
        if nfd < 0 {
            fileclose(nf);
            return EMFILE;
        }
        nfd as i64
    }
}

/// fstat(2) (sysfile.c sys_fstat): stat into a kernel buffer, then copy out.
pub fn sys_fstat(fd: usize, stat_user: usize) -> i64 {
    let f = match fd_file(fd) {
        Some(f) => f,
        None => return EBADF,
    };
    let mut st = Stat::default();
    // SAFETY: st is kernel memory; filestat locks the inode.
    if unsafe { filestat(f, &mut st) } < 0 {
        return EIO;
    }
    uaccess::copyout(
        stat_user as *mut u8,
        &st as *const Stat as *const u8,
        size_of::<Stat>(),
    )
}

/// pipe(2) (sysfile.c sys_pipe): allocate the pair, map both ends.
pub fn sys_pipe(fds_user: usize) -> i64 {
    let mut f0 = core::ptr::null_mut();
    let mut f1 = core::ptr::null_mut();
    // SAFETY: pipealloc owns both refs until mapped into fd slots.
    if unsafe { pipealloc(&mut f0, &mut f1) } < 0 {
        return EIO;
    }
    // SAFETY: fd slots + unwind exactly like sysfile.c sys_pipe.
    let (fd0, fd1) = unsafe {
        let fd0 = fdalloc(f0);
        let fd1 = if fd0 >= 0 { fdalloc(f1) } else { -1 };
        if fd0 < 0 || fd1 < 0 {
            if fd0 < 0 {
                fileclose(f0);
            } else {
                let p = myproc();
                (*p).ofile[fd0 as usize] = core::ptr::null_mut();
            }
            fileclose(f1);
            return EMFILE;
        }
        (fd0, fd1)
    };
    let fds = [(fd0 as u32).to_ne_bytes(), (fd1 as u32).to_ne_bytes()];
    let r = uaccess::copyout(
        fds_user as *mut u8,
        fds.as_ptr() as *const u8,
        size_of::<[u32; 2]>(),
    );
    if r != 0 {
        // Unwind: close both mapped ends on the copyout fault.
        unsafe {
            let p = myproc();
            let fa = (*p).ofile[fd0 as usize];
            let fb = (*p).ofile[fd1 as usize];
            (*p).ofile[fd0 as usize] = core::ptr::null_mut();
            (*p).ofile[fd1 as usize] = core::ptr::null_mut();
            fileclose(fa);
            fileclose(fb);
        }
        return r;
    }
    0
}

/// argfd (sysfile.c): validate an fd index and return the file pointer.
fn fd_file(fd: usize) -> Option<*mut File> {
    if fd >= NOFILE {
        return None;
    }
    // SAFETY: myproc slot read.
    unsafe {
        let p = myproc();
        let f = (*p).ofile[fd];
        if f.is_null() {
            None
        } else {
            Some(f)
        }
    }
}
