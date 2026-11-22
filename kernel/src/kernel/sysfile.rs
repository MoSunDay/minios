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

use crate::fs::file_table::{fileclose, filedup, fileread, filestat, filewrite, File, FileState};
use crate::fs::xv6fs::layout::{Stat, BSIZE, T_FILE};
use crate::ipc::pipe::pipealloc;
use crate::mm::uaccess;
use crate::proc::proc::myproc;
use crate::proc::NOFILE;

use abi::{EBADF, EINVAL, EIO, EMFILE};

use super::syspath::fdalloc;

/// read(2): fd-table dispatch with a per-block kernel bounce.
/// Regular-file descriptors ride the page cache (m4r-s4a): same
/// BSIZE chunking, same EOF/EFAULT/partial semantics — byte-equality
/// against the legacy readi path was proven by the s4a shadow-diff
/// battery run before the flip (ADR-0025). Pipes and device/dir
/// inodes keep the legacy fileread.
pub fn sys_read(fd: usize, buf: usize, count: u64) -> i64 {
    if count > i32::MAX as u64 {
        return EINVAL;
    }
    let f = match fd_file(fd) {
        Some(f) => f,
        None => return EBADF,
    };
    // SAFETY: the File's inode reference keeps the type stable.
    let cached_file = unsafe {
        match (*f).state {
            FileState::Inode { ip, .. } => (*ip).type_ == T_FILE,
            _ => false,
        }
    };
    if cached_file {
        return sys_read_cached(f, buf, count);
    }
    sys_read_legacy(f, buf, count)
}

/// The legacy read path, unchanged (pipes, device and directory
/// inodes — and the shadow-diff baseline while s4a proved the cache).
fn sys_read_legacy(f: *mut File, buf: usize, count: u64) -> i64 {
    let mut chunk = [0u8; BSIZE];
    let mut total: u32 = 0;
    // SAFETY: chunk is a kernel bounce; copyout faults as EFAULT.
    unsafe {
        while total < count as u32 {
            let want = core::cmp::min(BSIZE as u32, count as u32 - total);
            let r = fileread(f, chunk.as_mut_ptr(), want as i32);
            if r < 0 {
                // Pass the raw code through: EIO lies about EPIPE, and
                // -ERESTARTSYS (a signal interrupted an empty pipe
                // read) must reach the dispatch tail for the rewind —
                // a partial transfer (total > 0) is the POSIX short
                // read, never restarted (21 §7).
                return if total > 0 { total as i64 } else { r as i64 };
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

/// The cached read path: identical outer chunking to the legacy loop
/// (BSIZE at a time, EOF short-read break, partial-count EFAULT), so
/// the byte stream and return values match the frozen battery shape.
/// Off the File's shared offset advances BEFORE copyout — the legacy
/// fileread advanced inside its ilock window, consuming the chunk
/// even when the user copy later faults.
fn sys_read_cached(f: *mut File, buf: usize, count: u64) -> i64 {
    let ip = match unsafe { (*f).state } {
        FileState::Inode { ip, .. } => ip,
        _ => return EBADF,
    };
    let mut chunk = [0u8; BSIZE];
    let mut total: u32 = 0;
    // SAFETY: chunk is a kernel bounce; copyout faults as EFAULT.
    unsafe {
        while total < count as u32 {
            let want = core::cmp::min(BSIZE as u32, count as u32 - total) as usize;
            let off = match (*f).state {
                FileState::Inode { off, .. } => off as u64,
                _ => return EBADF,
            };
            match crate::fs::vfs_glue::pcache::kern_read_cached_ip(ip, off, &mut chunk[..want]) {
                Err(_) => {
                    // The legacy readi -1 spelling (off > size class);
                    // partial chunks before it stay POSIX-visible.
                    return if total > 0 { total as i64 } else { -1 };
                }
                Ok(0) => break, // EOF
                Ok(c) => {
                    // Consume first (the legacy ilock-window shape).
                    if let FileState::Inode { off, .. } = &mut (*f).state {
                        *off += c as u32;
                    }
                    let rc = uaccess::copyout((buf + total as usize) as *mut u8, chunk.as_ptr(), c);
                    if rc != 0 {
                        return if total > 0 { total as i64 } else { rc };
                    }
                    total += c as u32;
                    if c < want {
                        break; // short read (EOF)
                    }
                }
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
                // Same passthrough as sys_read: EPIPE and -ERESTARTSYS
                // keep their identities (partial writes stand).
                return if total > 0 { total as i64 } else { r as i64 };
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
        (*p).ofile_flags[fd] = 0; // POSIX close(2) clears FD_CLOEXEC
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

/// dup2(2) (m4r-s2b, Linux semantics): install `oldfd`'s file at
/// `newfd`, closing whatever holds it first. `oldfd == newfd` tests
/// validity and returns newfd untouched (no close-reopen race). The
/// new descriptor starts with FD_CLOEXEC clear (POSIX dup2).
pub fn sys_dup2(oldfd: usize, newfd: usize) -> i64 {
    let f = match fd_file(oldfd) {
        Some(f) => f,
        None => return EBADF,
    };
    if newfd >= NOFILE {
        return EBADF;
    }
    if newfd == oldfd {
        return newfd as i64;
    }
    // SAFETY: slot ownership moves between fds; filedup balances the
    // new ref before any failure point, close_new only on success.
    unsafe {
        let p = myproc();
        if !(*p).ofile[newfd].is_null() {
            let old = (*p).ofile[newfd];
            (*p).ofile[newfd] = core::ptr::null_mut();
            (*p).ofile_flags[newfd] = 0;
            fileclose(old);
        }
        (*p).ofile[newfd] = filedup(f);
        (*p).ofile_flags[newfd] = 0;
        newfd as i64
    }
}

/// fcntl(2) fd-flag subset (m4r-s2b): F_GETFD/F_SETFD over the
/// per-fd close-on-exec bit. Other commands are EINVAL (the M4 Core
/// surface; F_GETFL/F_SETFL arrive with the file-flags work).
pub fn sys_fcntl(fd: usize, cmd: u64, arg: u64) -> i64 {
    // SAFETY: fd slot reads under the caller's process context.
    unsafe {
        let p = myproc();
        if fd >= NOFILE || (*p).ofile[fd].is_null() {
            return EBADF;
        }
        match cmd {
            c if c == abi::F_GETFD => ((*p).ofile_flags[fd] as u64 & abi::FD_CLOEXEC) as i64,
            c if c == abi::F_SETFD => {
                (*p).ofile_flags[fd] = (arg & abi::FD_CLOEXEC) as u8;
                0
            }
            _ => EINVAL,
        }
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
