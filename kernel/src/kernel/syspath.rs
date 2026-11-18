//! syspath.rs: the namespace half of xv6 sysfile.c — open/mknod/mkdir/
//! chdir/unlink/link plus the shared create() helper. Writers wrap their
//! inode updates in begin_op/end_op exactly like xv6-public's
//! begin_trans/commit_trans pairs.
//!
//! User path strings bounce through copyinstr (EFAULT on fault) per this
//! kernel's uaccess discipline; xv6's blanket -1 failures are spelled as
//! Linux errnos (plans/linux26/02 §5).

use core::mem::size_of;

use crate::fs::file_table::{filealloc, fileclose, File, FileState};
use crate::fs::xv6fs::inode::{
    ialloc, ilock, iput, iunlock, iunlockput, iupdate, readi, writei, Inode,
};
use crate::fs::xv6fs::layout::{Dirent, DIRSIZ, T_DEV, T_DIR, T_FILE};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::fs::xv6fs::namei::{dirlink, dirlookup, namecmp, namei, nameiparent};
use crate::mm::uaccess;
use crate::proc::proc::myproc;

use abi::{EEXIST, EINVAL, EISDIR, ENOENT, ENOTDIR, ENOTEMPTY, O_CREAT, O_RDWR, O_WRONLY};

/// Kernel-side path buffer bound (xv6 has no MAXPATH; paths are only ever
/// component-bound by DIRSIZ in namei, so a modest bound suffices).
pub(crate) const MAXPATH: usize = 128;

/// copyinstr a NUL-terminated path from user memory into `dst`; 0 or
/// -EFAULT/-ENAMETOOLONG.
pub(crate) fn copy_path(dst: &mut [u8; MAXPATH], src: usize) -> i64 {
    if src == 0 {
        return uaccess::EFAULT;
    }
    let r = uaccess::copyinstr(dst, src as *const u8);
    if r != 0 {
        return r;
    }
    0
}

/// sysfile.c create(): allocate an inode of `type_` for `path` (or return
/// the existing one when the type matches); callers must already be inside
/// a begin_op/end_op transaction. Returns a LOCKED inode or null.
///
/// SAFETY: process context inside an open transaction.
unsafe fn create(path: *const u8, type_: i16, major: i16, minor: i16) -> *mut Inode {
    let mut name = [0u8; DIRSIZ];
    // SAFETY: nameiparent balances locks; dirlookup/ialloc per protocol.
    unsafe {
        let dp = nameiparent(path, &mut name);
        if dp.is_null() {
            return core::ptr::null_mut();
        }
        ilock(dp);
        let ip = dirlookup(dp, name.as_ptr(), core::ptr::null_mut());
        if !ip.is_null() {
            // sysfile.c create(): an existing entry is reused only for
            // file-on-file; mkdir on anything existing (or open(O_CREAT)
            // over a dir) fails. This is what usertests fourteen relies on.
            iunlockput(dp);
            ilock(ip);
            if type_ == T_FILE && (*ip).type_ == T_FILE {
                return ip;
            }
            iunlockput(ip);
            return core::ptr::null_mut();
        }

        let ip = ialloc((*dp).dev, type_);
        if ip.is_null() {
            panic!("create: ialloc");
        }

        ilock(ip);
        (*ip).major = major;
        (*ip).minor = minor;
        (*ip).nlink = 1;
        iupdate(ip);

        if type_ == T_DIR {
            // Create . and .. entries; ".." bumps dp->nlink (fs.c create).
            (*dp).nlink += 1;
            iupdate(dp);
            if dirlink(ip, b".\0".as_ptr(), (*ip).inum) < 0
                || dirlink(ip, b"..\0".as_ptr(), (*dp).inum) < 0
            {
                panic!("create dots");
            }
        }

        if dirlink(dp, name.as_ptr(), (*ip).inum) < 0 {
            panic!("create: dirlink");
        }

        iunlockput(dp);
        ip
    }
}

/// open(2) (sysfile.c sys_open). Linux flag bits (abi crate).
pub fn sys_open(path: usize, flags: u64) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    let omode = flags as u32;
    // SAFETY: cpath NUL-terminated; inode/file protocol balanced.
    unsafe {
        let ip: *mut Inode;
        if omode & (O_CREAT as u32) != 0 {
            begin_op();
            ip = create(cpath.as_ptr(), T_FILE, 0, 0);
            end_op();
            if ip.is_null() {
                return EEXIST;
            }
        } else {
            ip = namei(cpath.as_ptr());
            if ip.is_null() {
                return ENOENT;
            }
            ilock(ip);
            if (*ip).type_ == T_DIR && omode & 3 != 0 {
                iunlockput(ip);
                return EISDIR;
            }
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

/// mknod(2) (sysfile.c sys_mknod): create a device inode.
pub fn sys_mknod(path: usize, major: u64, minor: u64) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    // SAFETY: wrapped in a transaction like xv6's begin/commit pair.
    unsafe {
        begin_op();
        let ip = create(cpath.as_ptr(), T_DEV, major as i16, minor as i16);
        end_op();
        if ip.is_null() {
            return EEXIST;
        }
        iunlockput(ip);
    }
    0
}

/// mkdir(2) (sysfile.c sys_mkdir).
pub fn sys_mkdir(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    // SAFETY: wrapped in a transaction; create links the dots.
    unsafe {
        begin_op();
        let ip = create(cpath.as_ptr(), T_DIR, 0, 0);
        end_op();
        if ip.is_null() {
            return EEXIST;
        }
        iunlockput(ip);
    }
    0
}

/// chdir(2) (sysfile.c sys_chdir): swap myproc()->cwd.
pub fn sys_chdir(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    // SAFETY: namei/ilock/iput protocol balanced; cwd ref transferred.
    unsafe {
        let ip = namei(cpath.as_ptr());
        if ip.is_null() {
            return ENOENT;
        }
        ilock(ip);
        if (*ip).type_ != T_DIR {
            iunlockput(ip);
            return ENOTDIR;
        }
        iunlock(ip);
        let p = myproc();
        let old = (*p).cwd;
        (*p).cwd = ip;
        if !old.is_null() {
            begin_op();
            iput(old);
            end_op();
        }
    }
    0
}

/// Is directory `dp` (locked) empty apart from "." and ".."?
/// (sysfile.c isdirempty)
///
/// SAFETY: dp locked.
unsafe fn isdirempty(dp: *mut Inode) -> bool {
    let mut de = Dirent::default();
    let mut off: u32 = 2 * size_of::<Dirent>() as u32;
    // SAFETY: dp locked T_DIR; readi through the file layer core.
    unsafe {
        while off < (*dp).size {
            if readi(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                size_of::<Dirent>() as u32,
            ) != size_of::<Dirent>() as i32
            {
                panic!("isdirempty: readi");
            }
            if de.inum != 0 {
                return false;
            }
            off += size_of::<Dirent>() as u32;
        }
    }
    true
}

/// unlink(2) (sysfile.c sys_unlink).
pub fn sys_unlink(path: usize) -> i64 {
    let mut cpath = [0u8; MAXPATH];
    let r = copy_path(&mut cpath, path);
    if r != 0 {
        return r;
    }
    // SAFETY: one transaction around the dirent removal + nlink updates.
    unsafe {
        let mut name = [0u8; DIRSIZ];
        begin_op();
        let dp = nameiparent(cpath.as_ptr(), &mut name);
        if dp.is_null() {
            end_op();
            return ENOENT;
        }
        ilock(dp);
        // Cannot unlink "." or ".." (fs.c sys_unlink): the entry resolves
        // to `dp` itself, so a later ilock(ip) would self-deadlock.
        if namecmp(name.as_ptr(), b".\0".as_ptr()) == 0
            || namecmp(name.as_ptr(), b"..\0".as_ptr()) == 0
        {
            iunlockput(dp);
            end_op();
            return EINVAL;
        }
        let mut off: u32 = 0;
        let ip = dirlookup(dp, name.as_ptr(), &raw mut off);
        if ip.is_null() {
            iunlockput(dp);
            end_op();
            return ENOENT;
        }
        ilock(ip);
        if (*ip).nlink < 1 {
            panic!("unlink: nlink < 1");
        }
        if (*ip).type_ == T_DIR && !isdirempty(ip) {
            iunlockput(ip);
            iunlockput(dp);
            end_op();
            return ENOTEMPTY;
        }
        let mut de = Dirent::default();
        if writei(
            dp,
            &mut de as *mut Dirent as *const u8,
            off,
            size_of::<Dirent>() as u32,
        ) != size_of::<Dirent>() as i32
        {
            panic!("unlink: writei");
        }
        if (*ip).type_ == T_DIR {
            (*dp).nlink -= 1;
            iupdate(dp);
        }
        iunlockput(dp);
        (*ip).nlink -= 1;
        iupdate(ip);
        iunlockput(ip);
        end_op();
    }
    0
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
    // SAFETY: transaction wraps the nlink bump + new dirent.
    unsafe {
        begin_op();
        let ip = namei(cold.as_ptr());
        if ip.is_null() {
            end_op();
            return ENOENT;
        }
        ilock(ip);
        if (*ip).type_ == T_DIR {
            iunlockput(ip);
            end_op();
            return abi::EPERM;
        }
        (*ip).nlink += 1;
        iupdate(ip);
        iunlock(ip);

        let mut name = [0u8; DIRSIZ];
        let dp = nameiparent(cnew.as_ptr(), &mut name);
        if dp.is_null() {
            link_bad_unwind(ip);
            return ENOENT;
        }
        ilock(dp);
        if (*dp).dev != (*ip).dev || dirlink(dp, name.as_ptr(), (*ip).inum) < 0 {
            iunlockput(dp);
            link_bad_unwind(ip);
            return abi::EIO;
        }
        iunlockput(dp);
        iput(ip);
        end_op();
    }
    0
}

/// link-failure unwind (sysfile.c `bad:` label): drop the extra nlink.
///
/// SAFETY: ip holds the bumped-but-unlinked nlink, unlocked.
unsafe fn link_bad_unwind(ip: *mut Inode) {
    unsafe {
        ilock(ip);
        (*ip).nlink -= 1;
        iupdate(ip);
        iunlockput(ip);
        end_op();
    }
}
