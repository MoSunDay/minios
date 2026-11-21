//! Directories + path names: port of the namei half of `fs.c` (xv6-public).
//!
//! Path arguments are `*const u8` (NUL-terminated, may point into user
//! memory), matching `kernel/src/exec.rs` and `proc.rs` call sites. The
//! current directory for relative paths is `myproc()->cwd`, exactly like
//! xv6's `proc->cwd`.

use crate::fs::xv6fs::inode::{idup, iget, ilock, iput, iunlock, iunlockput, readi, writei, Inode};
use crate::fs::xv6fs::layout::{Dirent, DIRSIZ, ROOTDEV, ROOTINO, T_DIR};

/// Compare directory-entry names (fs.c namecmp: strncmp(s, t, DIRSIZ)).
/// Never reads past DIRSIZ bytes, so the entries need not be NUL-terminated.
///
/// # Safety
/// both pointers must reference at least DIRSIZ readable bytes OR be
/// NUL-terminated before DIRSIZ.
pub unsafe fn namecmp(s: *const u8, t: *const u8) -> i32 {
    // SAFETY: caller guarantees DIRSIZ-readable-or-NUL-terminated buffers.
    unsafe {
        for i in 0..DIRSIZ {
            let a = *s.add(i) as i32;
            let b = *t.add(i) as i32;
            if a != b {
                return a - b;
            }
            if a == 0 {
                return 0;
            }
        }
    }
    0
}

/// Look for a directory entry in a directory.
/// If found, set *poff to byte offset of entry. (fs.c dirlookup)
///
/// # Safety
/// `dp` must be locked and T_DIR; `name` NUL-terminated (or DIRSIZ readable
/// bytes); `poff` null or valid for writes.
pub unsafe fn dirlookup(dp: *mut Inode, name: *const u8, poff: *mut u32) -> *mut Inode {
    unsafe {
        if (*dp).type_ != T_DIR {
            panic!("dirlookup not DIR");
        }

        let mut de = Dirent::default();
        let mut off: u32 = 0;
        while off < (*dp).size {
            if readi(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                core::mem::size_of::<Dirent>() as u32,
            ) != core::mem::size_of::<Dirent>() as i32
            {
                panic!("dirlookup readi");
            }
            if de.inum == 0 {
                off += core::mem::size_of::<Dirent>() as u32;
                continue;
            }
            if namecmp(name, de.name.as_ptr()) == 0 {
                // entry matches name
                if !poff.is_null() {
                    *poff = off;
                }
                let inum = de.inum as u32;
                return iget((*dp).dev, inum);
            }
            off += core::mem::size_of::<Dirent>() as u32;
        }
    }
    core::ptr::null_mut()
}

/// Write a new directory entry (name, inum) into the directory dp.
/// (fs.c dirlink)
///
/// # Safety
/// `dp` must be locked; `name` NUL-terminated; caller must be inside a
/// begin_op/end_op transaction.
#[allow(dead_code)] // S4d: sys_open/create/link append dirents
pub unsafe fn dirlink(dp: *mut Inode, name: *const u8, inum: u32) -> i32 {
    unsafe {
        // Check that name is not present.
        let ip = dirlookup(dp, name, core::ptr::null_mut());
        if !ip.is_null() {
            iput(ip);
            return -1;
        }

        // Look for an empty dirent.
        let mut de = Dirent::default();
        let mut off: u32 = 0;
        while off < (*dp).size {
            if readi(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                core::mem::size_of::<Dirent>() as u32,
            ) != core::mem::size_of::<Dirent>() as i32
            {
                panic!("dirlink read");
            }
            if de.inum == 0 {
                break;
            }
            off += core::mem::size_of::<Dirent>() as u32;
        }

        // strncpy(de.name, name, DIRSIZ): copy then zero-fill, like ulib.c.
        let mut i = 0;
        while i < DIRSIZ && *name.add(i) != 0 {
            de.name[i] = *name.add(i);
            i += 1;
        }
        while i < DIRSIZ {
            de.name[i] = 0;
            i += 1;
        }
        de.inum = inum as u16;
        if writei(
            dp,
            &mut de as *mut Dirent as *mut u8,
            off,
            core::mem::size_of::<Dirent>() as u32,
        ) != core::mem::size_of::<Dirent>() as i32
        {
            panic!("dirlink");
        }

        0
    }
}

//PAGEBREAK!
// Paths

/// Copy the next path element from path into name.
/// Return a pointer to the element following the copied one.
/// The returned path has no leading slashes,
/// so the caller can check *path=='\0' to see if the name is the last one.
/// If no name to remove, return 0. (fs.c skipelem)
///
/// Examples:
///   skipelem("a/bb/c", name) = "bb/c", setting name = "a"
///   skipelem("///a//bb", name) = "bb", setting name = "a"
///   skipelem("a", name) = "", setting name = "a"
///   skipelem("", name) = skipelem("////", name) = 0
///
/// # Safety
/// `path` must be NUL-terminated.
pub unsafe fn skipelem(path: *const u8, name: &mut [u8; DIRSIZ]) -> *const u8 {
    // SAFETY: caller guarantees a readable NUL-terminated path; `name` is a
    // fixed DIRSIZ array, exactly the buffer fs.c uses.
    unsafe {
        let mut path = path;
        while *path == b'/' {
            path = path.add(1);
        }
        if *path == 0 {
            return core::ptr::null();
        }
        let s = path;
        while *path != b'/' && *path != 0 {
            path = path.add(1);
        }
        let len = path as usize - s as usize;
        if len >= DIRSIZ {
            core::ptr::copy_nonoverlapping(s, name.as_mut_ptr(), DIRSIZ);
        } else {
            core::ptr::copy_nonoverlapping(s, name.as_mut_ptr(), len);
            name[len] = 0;
        }
        while *path == b'/' {
            path = path.add(1);
        }
        path
    }
}

/// Look up and return the inode for a path name (fs.c namex).
/// If nameiparent != 0, return the inode for the parent and copy the final
/// path element into name, which must have room for DIRSIZ bytes.
///
/// # Safety
/// `path` NUL-terminated.
unsafe fn namex(path: *const u8, nameiparent: bool, name: &mut [u8; DIRSIZ]) -> *mut Inode {
    unsafe {
        let mut ip: *mut Inode;
        if *path == b'/' {
            ip = iget(ROOTDEV, ROOTINO);
        } else {
            // xv6: ip = idup(cp->cwd). The cwd pointer lands with the S4d
            // file table; until then (no process has a cwd) the lookup
            // fails like fs.c's miss path.
            let p = crate::proc::proc::myproc();
            if p.is_null() {
                return core::ptr::null_mut();
            }
            let cwd = (*p).cwd as *mut Inode;
            if cwd.is_null() {
                return core::ptr::null_mut();
            }
            ip = idup(cwd);
        }

        let mut path = path;
        loop {
            path = skipelem(path, name);
            if path.is_null() {
                break;
            }
            ilock(ip);
            if (*ip).type_ != T_DIR {
                iunlockput(ip);
                return core::ptr::null_mut();
            }
            if nameiparent && *path == 0 {
                // Stop one level early.
                iunlock(ip);
                return ip;
            }
            let next = dirlookup(ip, name.as_ptr(), core::ptr::null_mut());
            if next.is_null() {
                iunlockput(ip);
                return core::ptr::null_mut();
            }
            iunlockput(ip);
            ip = next;
        }
        if nameiparent {
            iput(ip);
            return core::ptr::null_mut();
        }
        ip
    }
}

/// Look up and return the inode for a path name. (fs.c namei)
///
/// # Safety
/// `path` must be a NUL-terminated path (may point into user memory).
#[allow(dead_code)] // m4r-s2b: superseded by vfs_glue::kern_namei; kept as the reference port
pub unsafe fn namei(path: *const u8) -> *mut Inode {
    // SAFETY: caller guarantees a NUL-terminated path; name is DIRSIZ
    // scratch bytes on this stack frame, as in fs.c namei().
    unsafe {
        let mut name = [0u8; DIRSIZ];
        namex(path, false, &mut name)
    }
}

/// Look up and return the inode for a path's parent directory and copy the
/// final path element into name. (fs.c nameiparent)
///
/// # Safety
/// `path` NUL-terminated.
#[allow(dead_code)] // S4d: sys_open/create resolve the parent dir
pub unsafe fn nameiparent(path: *const u8, name: &mut [u8; DIRSIZ]) -> *mut Inode {
    unsafe { namex(path, true, name) }
}
