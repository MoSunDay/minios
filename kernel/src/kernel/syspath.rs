//! syspath.rs: the namespace syscalls over the m4r-s2b VFS glue —
//! kern_namei/kern_create/kern_unlink/kern_link walk the dcache and
//! dispatch through the ops vtables (vfs_glue), folding VfsErr to
//! errnos at this edge. What stays here: the copyinstr path bounce,
//! fd allocation, and open(2)'s descriptor plumbing (the file
//! read/write path stays legacy this milestone; s3 redirects it).
//!
//! Legacy spellings kept byte-for-byte: every create-family failure
//! folds to EEXIST (xv6's create() returns null for both a missing
//! parent and a type conflict), O_TRUNC/O_EXCL stay ignored, and
//! chdir's ENOTDIR is a direct type check on the resolved inode (not
//! the walk's NotDir→ENOENT fold).

use crate::fs::file_table::{filealloc, fileclose, File, FileState};
use crate::fs::vfs_glue::{
    dput_unlocked, errno_of, kern_create, kern_link, kern_namei, kern_namei_d, kern_unlink,
    CreateKind,
};
use crate::fs::xv6fs::inode::{ilock, iput, iunlock, iunlockput};
use crate::fs::xv6fs::layout::T_DIR;
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::proc::proc::myproc;

use abi::{EEXIST, EISDIR, ENOTDIR, O_CREAT, O_RDWR, O_WRONLY};

/// Kernel-side path buffer bound (xv6 has no MAXPATH; paths are only ever
/// component-bound by DIRSIZ in namei, so a modest bound suffices).
pub(crate) const MAXPATH: usize = 128;

/// copyinstr a NUL-terminated path from user memory into `dst`; 0 or
/// -EFAULT/-ENAMETOOLONG.
pub(crate) fn copy_path(dst: &mut [u8; MAXPATH], src: usize) -> i64 {
    if src == 0 {
        return crate::mm::uaccess::EFAULT;
    }
    let r = crate::mm::uaccess::copyinstr(dst, src as *const u8);
    if r != 0 {
        return r;
    }
    0
}

/// Length of a NUL-terminated path inside a MAXPATH buffer.
fn path_len(c: &[u8]) -> usize {
    c.iter().position(|&b| b == 0).unwrap_or(c.len())
}

/// open(2) (sysfile.c sys_open tail): resolve or create through the
/// glue, then the legacy descriptor plumbing.
pub fn sys_open(path: usize, flags: u64) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    let omode = flags as u32;
    let n = path_len(&cpath);
    // SAFETY: cpath NUL-terminated; inode/file protocol balanced.
    unsafe {
        let ip = if omode & (O_CREAT as u32) != 0 {
            // Legacy create() folds EVERY failure to null→EEXIST
            // (missing parent included).
            match kern_create(&cpath[..n], CreateKind::File) {
                Ok(ip) => ip,
                Err(_) => return EEXIST,
            }
        } else {
            match kern_namei(&cpath[..n]) {
                Ok(ip) => ip,
                Err(e) => return errno_of(e), // NoEnt/NotDir → ENOENT
            }
        };
        ilock(ip);
        if (*ip).type_ == T_DIR && omode & 3 != 0 {
            iunlockput(ip);
            return EISDIR;
        }
        let f = filealloc();
        if f.is_null() {
            iunlockput(ip);
            return abi::EMFILE;
        }
        let fd = fdalloc(f);
        if fd < 0 {
            fileclose(f);
            iunlockput(ip);
            return abi::EMFILE;
        }
        (*f).state = FileState::Inode { ip, off: 0 };
        (*f).readable = if omode & (O_WRONLY as u32) == 0 { 1 } else { 0 };
        (*f).writable = if omode & (O_WRONLY as u32) != 0 || omode & (O_RDWR as u32) != 0 {
            1
        } else {
            0
        };
        iunlock(ip);
        fd as i64
    }
}

/// xv6 fdalloc (sysfile.c): map a file into the lowest free fd slot.
///
/// SAFETY: process context; caller owns the file reference.
pub(crate) unsafe fn fdalloc(f: *mut File) -> i32 {
    let p = myproc();
    for fd in 0..crate::proc::NOFILE {
        // SAFETY: myproc() slot; ofile guarded by single-threaded proc.
        unsafe {
            if (*p).ofile[fd].is_null() {
                (*p).ofile[fd] = f;
                return fd as i32;
            }
        }
    }
    -1
}

/// mknod(2) (sysfile.c sys_mknod): create a device inode through the
/// dedicated adapter spelling (the frozen vtable create carries no
/// rdev pair).
pub fn sys_mknod(path: usize, major: u64, minor: u64) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    match kern_create(
        &cpath[..path_len(&cpath)],
        CreateKind::Dev(major as i16, minor as i16),
    ) {
        Ok(_) => 0,
        Err(_) => EEXIST, // legacy create-null spelling
    }
}

/// mkdir(2) (sysfile.c sys_mkdir).
pub fn sys_mkdir(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    match kern_create(&cpath[..path_len(&cpath)], CreateKind::Dir) {
        Ok(_) => 0,
        Err(_) => EEXIST, // legacy create-null spelling
    }
}

/// chdir(2) (sysfile.c sys_chdir): swap myproc()->cwd (inode + dentry
/// shadow together).
pub fn sys_chdir(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    let (ip, d) = match kern_namei_d(&cpath[..path_len(&cpath)]) {
        Ok(x) => x,
        Err(e) => return errno_of(e),
    };
    // SAFETY: namei/ilock/iput protocol balanced; cwd refs transferred.
    unsafe {
        ilock(ip);
        if (*ip).type_ != T_DIR {
            iunlockput(ip);
            dput_unlocked(d);
            return ENOTDIR;
        }
        iunlock(ip);
        let p = myproc();
        let old = (*p).cwd;
        let old_d = (*p).cwd_d;
        (*p).cwd = ip;
        (*p).cwd_d = d;
        if !old.is_null() {
            begin_op();
            iput(old);
            end_op();
        }
        if !old_d.is_null() {
            dput_unlocked(old_d);
        }
    }
    0
}

/// unlink(2) (sysfile.c sys_unlink; folds rmdir).
pub fn sys_unlink(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    match kern_unlink(&cpath[..path_len(&cpath)]) {
        Ok(()) => 0,
        Err(e) => errno_of(e),
    }
}

/// link(2) (sysfile.c sys_link).
pub fn sys_link(oldpath: usize, newpath: usize) -> i64 {
    let mut cold = [0u8; MAXPATH];
    let mut cnew = [0u8; MAXPATH];
    let r = copy_path(&mut cold, oldpath);
    if r != 0 {
        return r;
    }
    let r = copy_path(&mut cnew, newpath);
    if r != 0 {
        return r;
    }
    match kern_link(&cold[..path_len(&cold)], &cnew[..path_len(&cnew)]) {
        Ok(()) => 0,
        Err(e) => errno_of(e),
    }
}
