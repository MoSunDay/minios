//! fsops.rs: xv6 sysfile.c part 2 — path-based system calls
//! (open/create/link/unlink/mkdir/mknod/chdir). Split from file.rs for the
//! 400-line file limit.

use core::mem::zeroed;
use core::ptr::null_mut;

use crate::fs::file_table::{filealloc, fileclose, FileState};
use crate::fs::xv6fs::inode::{
    ialloc, ilock, iput, iunlock, iunlockput, iupdate, readi, writei, Inode,
};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::fs::xv6fs::namei::{dirlink, dirlookup, namecmp, namei, nameiparent};
use crate::fs::{Dirent, O_CREATE, O_RDONLY, O_RDWR, O_WRONLY, T_DEV, T_DIR, T_FILE};
use crate::kernel::sched::myproc;
use crate::lib::string::memset;
use crate::usercall::file::fdalloc;
use crate::usercall::{argint, argstr};

/// Is the directory dp empty except for "." and ".."?
unsafe fn isdirempty(dp: *mut Inode) -> i32 {
    let mut de: Dirent = unsafe { zeroed() };
    //SAFETY: dp is ilock()ed by the caller.
    let mut off = 2 * core::mem::size_of::<Dirent>() as u32;
    while off < unsafe { (*dp).size } {
        if unsafe {
            readi(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                core::mem::size_of::<Dirent>() as u32,
            )
        } != core::mem::size_of::<Dirent>() as i32
        {
            panic!("isdirempty: readi");
        }
        if de.inum != 0 {
            return 0;
        }
        off += core::mem::size_of::<Dirent>() as u32;
    }
    1
}

/// Create the path `path` as type with the given major/minor dev numbers;
/// returns the inode ilock()ed (caller does iunlockput).
unsafe fn create(path: *const u8, type_: i16, major: i16, minor: i16) -> *mut Inode {
    let mut name = [0u8; crate::fs::DIRSIZ];
    let dp = unsafe { nameiparent(path, &mut name) };
    if dp.is_null() {
        return null_mut();
    }
    unsafe { ilock(dp) };

    let mut off: u32 = 0;
    //SAFETY: dp ilock()ed; name is DIRSIZ bytes from nameiparent.
    let ip = unsafe { dirlookup(dp, name.as_ptr(), &mut off) };
    if !ip.is_null() {
        unsafe {
            iunlockput(dp);
            ilock(ip);
            if type_ == T_FILE && (*ip).type_ == T_FILE {
                return ip;
            }
            iunlockput(ip);
        }
        return null_mut();
    }

    //SAFETY: dp ilock()ed holds dev.
    let ip = unsafe { ialloc((*dp).dev, type_) };
    if ip.is_null() {
        panic!("create: ialloc");
    }

    unsafe {
        ilock(ip);
        (*ip).major = major;
        (*ip).minor = minor;
        (*ip).nlink = 1;
        iupdate(ip);

        if type_ == T_DIR {
            // Create . and .. entries.
            (*dp).nlink += 1; // for ".."
            iupdate(dp);
            // No ip->nlink++ for ".": avoid cyclic ref count.
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
    }
    ip
}

pub unsafe fn sys_open() -> i32 {
    let mut path: *const u8 = null_mut();
    let mut omode: i32 = 0;
    if unsafe { argstr(0, &mut path) } < 0 || unsafe { argint(1, &mut omode) } < 0 {
        return -1;
    }

    let ip: *mut Inode;
    if omode & O_CREATE != 0 {
        begin_op();
        ip = unsafe { create(path, T_FILE, 0, 0) };
        end_op();
        if ip.is_null() {
            return -1;
        }
    } else {
        ip = unsafe { namei(path) };
        if ip.is_null() {
            return -1;
        }
        unsafe { ilock(ip) };
        if unsafe { (*ip).type_ } == T_DIR && omode != O_RDONLY {
            unsafe { iunlockput(ip) };
            return -1;
        }
    }

    let f = unsafe { filealloc() };
    let fd = if f.is_null() {
        -1
    } else {
        unsafe { fdalloc(f) }
    };
    if f.is_null() || fd < 0 {
        if !f.is_null() {
            unsafe { fileclose(f) };
        }
        unsafe { iunlockput(ip) };
        return -1;
    }
    unsafe { iunlock(ip) };

    //SAFETY: f is exclusively ours between filealloc and fd store.
    unsafe {
        (*f).state = FileState::Inode { ip, off: 0 };
        (*f).readable = if omode & O_WRONLY != 0 { 0 } else { 1 };
        (*f).writable = if (omode & O_WRONLY) != 0 || (omode & O_RDWR) != 0 {
            1
        } else {
            0
        };
    }
    fd
}

/// Create the path new as a link to the same inode as old.
pub unsafe fn sys_link() -> i32 {
    let mut name = [0u8; crate::fs::DIRSIZ];
    let mut new: *const u8 = null_mut();
    let mut old: *const u8 = null_mut();

    if unsafe { argstr(0, &mut old) } < 0 || unsafe { argstr(1, &mut new) } < 0 {
        return -1;
    }

    begin_op();

    let ip = unsafe { namei(old as *mut u8) };
    if ip.is_null() {
        end_op();
        return -1;
    }

    unsafe { ilock(ip) };
    if unsafe { (*ip).type_ } == T_DIR {
        unsafe {
            iunlockput(ip);
            end_op();
        }
        return -1;
    }

    // Compensation path (xv6 `bad:` label): re-lock ip, drop the extra
    // nlink we are about to add, then put everything back.
    let bad = |ip: *mut Inode| unsafe {
        ilock(ip);
        (*ip).nlink -= 1;
        iupdate(ip);
        iunlockput(ip);
        end_op();
    };

    unsafe {
        (*ip).nlink += 1;
        iupdate(ip);
        iunlock(ip);
    }

    let dp = unsafe { nameiparent(new, &mut name) };
    if dp.is_null() {
        bad(ip);
        return -1;
    }
    unsafe { ilock(dp) };
    //SAFETY: cannot link across devices; dirlink adds the dirent.
    if unsafe { (*dp).dev != (*ip).dev } || unsafe { dirlink(dp, name.as_ptr(), (*ip).inum) } < 0 {
        unsafe {
            iunlockput(dp);
            bad(ip);
        }
        return -1;
    }

    unsafe {
        iunlockput(dp);
        iput(ip);
        end_op();
    }
    0
}

pub unsafe fn sys_unlink() -> i32 {
    let mut name = [0u8; crate::fs::DIRSIZ];
    let mut path: *const u8 = null_mut();
    let mut de: Dirent = unsafe { zeroed() };

    if unsafe { argstr(0, &mut path) } < 0 {
        return -1;
    }
    let dp = unsafe { nameiparent(path, &mut name) };
    if dp.is_null() {
        return -1;
    }

    let ok = 'body: {
        begin_op();
        unsafe { ilock(dp) };

        // Cannot unlink "." or "..".
        //SAFETY: name is a NUL-free DIRSIZ buffer from nameiparent.
        if unsafe { namecmp(name.as_ptr(), b".\0".as_ptr()) } == 0
            || unsafe { namecmp(name.as_ptr(), b"..\0".as_ptr()) } == 0
        {
            break 'body false;
        }

        let mut off: u32 = 0;
        //SAFETY: dp ilock()ed.
        let ip = unsafe { dirlookup(dp, name.as_ptr(), &mut off) };
        if ip.is_null() {
            break 'body false;
        }
        unsafe { ilock(ip) };

        if unsafe { (*ip).nlink } < 1 {
            panic!("unlink: nlink < 1");
        }
        if unsafe { (*ip).type_ } == T_DIR && unsafe { isdirempty(ip) } == 0 {
            unsafe { iunlockput(ip) };
            break 'body false;
        }

        unsafe {
            memset(
                &mut de as *mut Dirent as *mut u8,
                0,
                core::mem::size_of::<Dirent>(),
            );
            if writei(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                core::mem::size_of::<Dirent>() as u32,
            ) != core::mem::size_of::<Dirent>() as i32
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
            // ip is still I_BUSY here: unlock before dropping the reference.
            iunlockput(ip);
            end_op();
        }
        return 0;
    };

    if !ok {
        unsafe {
            iunlockput(dp);
            end_op();
        }
    }
    -1
}

pub unsafe fn sys_mkdir() -> i32 {
    let mut path: *const u8 = null_mut();
    begin_op();
    if unsafe { argstr(0, &mut path) } < 0 {
        end_op();
        return -1;
    }
    let ip = unsafe { create(path, T_DIR, 0, 0) };
    if ip.is_null() {
        end_op();
        return -1;
    }
    unsafe {
        iunlockput(ip);
        end_op();
    }
    0
}

pub unsafe fn sys_mknod() -> i32 {
    let mut path: *const u8 = null_mut();
    let mut major: i32 = 0;
    let mut minor: i32 = 0;
    begin_op();
    if unsafe { argstr(0, &mut path) } < 0
        || unsafe { argint(1, &mut major) } < 0
        || unsafe { argint(2, &mut minor) } < 0
    {
        end_op();
        return -1;
    }
    let ip = unsafe { create(path, T_DEV, major as i16, minor as i16) };
    if ip.is_null() {
        end_op();
        return -1;
    }
    unsafe {
        iunlockput(ip);
        end_op();
    }
    0
}

pub unsafe fn sys_chdir() -> i32 {
    let mut path: *const u8 = null_mut();
    if unsafe { argstr(0, &mut path) } < 0 {
        return -1;
    }
    let ip = unsafe { namei(path) };
    if ip.is_null() {
        return -1;
    }
    unsafe { ilock(ip) };
    if unsafe { (*ip).type_ } != T_DIR {
        unsafe { iunlockput(ip) };
        return -1;
    }
    unsafe { iunlock(ip) };
    let p = myproc();
    //SAFETY: cwd swap with correct refcounts (iput drops old cwd's ref).
    unsafe {
        iput((*p).cwd);
        (*p).cwd = ip;
    }
    0
}
