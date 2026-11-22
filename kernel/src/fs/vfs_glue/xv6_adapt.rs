//! xv6_adapt (m4r-s2b): the xv6fs binding for the s1 vtables. Every
//! fs-op replicates its sysfile.c/fs.c counterpart's on-disk sequence
//! verbatim (the battery pins those bytes) with one structural
//! change: reference accounting. The glue owns the directory and old
//! inode references (dentry pins → wrappers → iget refs), so the
//! adapter consumes only the references IT creates (dirlookup,
//! ialloc) and hands each to a fresh wrapper — legacy's
//! iunlockput(dp) tail becomes iunlock(dp), and the returned inode is
//! UNLOCKED (legacy returned it locked because the caller iput'd the
//! create ref; here the wrapper owns it and kern_create idups for the
//! caller).
//!
//! Each mutating op carries its own begin_op/end_op transaction (the
//! glue walk holds no DENTRY_LOCK across these calls). The
//! authoritative existing-entry recheck runs under ilock(dp) inside
//! the tx — the glue's phase-A peek is advisory only. The
//! object-level tables (FileOps/SuperOps) sit in xv6_obj.rs.

use super::wrapper::{wrap_new_unlocked, VfsInode};
use crate::fs::vfs::ops::{InodeCommon, InodeOps};
use crate::fs::vfs::{VfsErr, VfsResult, S_IFDIR, S_IFMT, S_IFREG};
use crate::fs::xv6fs::inode::{
    ialloc, ilock, iput, iunlock, iunlockput, iupdate, readi, writei, Inode,
};
use crate::fs::xv6fs::layout::{Dirent, DIRSIZ, T_DEV, T_DIR, T_FILE};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::fs::xv6fs::namei::{dirlink, dirlookup, namecmp};

/// Copy a vtable name argument (ptr+len, NOT NUL-terminated) into a
/// NUL-terminated DIRSIZ scratch buffer. Zero-length and over-DIRSIZ
/// names are misses (the legacy skipelem budget).
unsafe fn name_buf(name: *const u8, len: usize) -> VfsResult<[u8; DIRSIZ + 1]> {
    if len == 0 || len > DIRSIZ {
        return Err(VfsErr::NoEnt);
    }
    let mut b = [0u8; DIRSIZ + 1];
    // SAFETY: the caller guarantees `len` readable bytes.
    unsafe { core::ptr::copy_nonoverlapping(name, b.as_mut_ptr(), len) };
    Ok(b)
}

/// sysfile.c create() core over a resolved parent wrapper. Runs its
/// own transaction; returns a wrapper owning the new (or reused)
/// inode's reference, inode unlocked.
unsafe fn create_common(
    dir: *mut InodeCommon,
    name: *const u8,
    len: usize,
    type_: i16,
    major: i16,
    minor: i16,
) -> VfsResult<*mut InodeCommon> {
    // SAFETY: tx + ilock protocol per sysfile.c create; the wrapper
    // pin on `dir` keeps dp alive across every sleep below.
    unsafe {
        let nb = name_buf(name, len)?;
        let dp = (*(dir as *mut VfsInode)).xv6_ip;
        begin_op();
        ilock(dp);
        // Legacy create(): a non-directory parent fails before any
        // dirent scan (usertests subdir probes "file/file" paths).
        if (*dp).type_ != T_DIR {
            iunlock(dp);
            end_op();
            return Err(VfsErr::NotDir);
        }
        let ip = dirlookup(dp, nb.as_ptr(), core::ptr::null_mut());
        if !ip.is_null() {
            // Existing entry: reuse only file-on-file (fourteen relies
            // on this); everything else is EEXIST.
            ilock(ip);
            let ok = type_ == T_FILE && (*ip).type_ == T_FILE;
            iunlock(ip);
            iunlock(dp);
            end_op();
            if ok {
                // The dirlookup ref transfers to the fresh wrapper.
                return wrap_new_unlocked(ip).map(|w| w as *mut InodeCommon);
            }
            // Kind mismatch: give the dirlookup ref back (leaking it
            // pins a table slot). Plain iput — the ilock/iunlock pair
            // above already closed; nlink >= 1 means no truncation.
            iput(ip);
            return Err(VfsErr::Exist);
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
            // ". and .." live in the new directory; ".." bumps dp's
            // nlink first (fs.c create ordering).
            (*dp).nlink += 1;
            iupdate(dp);
            if dirlink(ip, b".\0".as_ptr(), (*ip).inum) < 0
                || dirlink(ip, b"..\0".as_ptr(), (*dp).inum) < 0
            {
                panic!("create dots");
            }
        }
        if dirlink(dp, nb.as_ptr(), (*ip).inum) < 0 {
            panic!("create: dirlink");
        }
        iunlock(ip);
        iunlock(dp);
        end_op();
        wrap_new_unlocked(ip).map(|w| w as *mut InodeCommon)
    }
}

fn xv6_create(
    dir: *mut InodeCommon,
    name: *const u8,
    len: usize,
    mode: u16,
) -> VfsResult<*mut InodeCommon> {
    if mode & S_IFMT != S_IFREG {
        return Err(VfsErr::Inval);
    }
    // SAFETY: create_common contract.
    unsafe { create_common(dir, name, len, T_FILE, 0, 0) }
}

fn xv6_mkdir(
    dir: *mut InodeCommon,
    name: *const u8,
    len: usize,
    mode: u16,
) -> VfsResult<*mut InodeCommon> {
    if mode & S_IFMT != S_IFDIR {
        return Err(VfsErr::Inval);
    }
    // SAFETY: create_common contract.
    unsafe { create_common(dir, name, len, T_DIR, 0, 0) }
}

/// mknod spelling: the frozen vtable create carries no rdev pair, so
/// device inodes take this dedicated entry (kern_create Dev kind).
pub fn create_dev(
    dir: *mut InodeCommon,
    name: *const u8,
    len: usize,
    major: i16,
    minor: i16,
) -> VfsResult<*mut InodeCommon> {
    // SAFETY: create_common contract.
    unsafe { create_common(dir, name, len, T_DEV, major, minor) }
}

fn xv6_lookup(dir: *mut InodeCommon, name: *const u8, len: usize) -> VfsResult<*mut InodeCommon> {
    // SAFETY: the wrapper pins dp; ilock may sleep but the glue walk
    // holds no dcache lock across i_op->lookup.
    unsafe {
        let nb = name_buf(name, len)?;
        let dp = (*(dir as *mut VfsInode)).xv6_ip;
        ilock(dp);
        if (*dp).type_ != T_DIR {
            iunlock(dp);
            return Err(VfsErr::NotDir);
        }
        let next = dirlookup(dp, nb.as_ptr(), core::ptr::null_mut());
        if next.is_null() {
            iunlock(dp);
            return Err(VfsErr::NoEnt);
        }
        // Validate the child under its lock (iget only promised the
        // cache slot), then hand the dirlookup ref to a wrapper.
        ilock(next);
        let w = wrap_new_unlocked(next);
        iunlock(next);
        iunlock(dp);
        w.map(|x| x as *mut InodeCommon)
    }
}

fn xv6_unlink(dir: *mut InodeCommon, name: *const u8, len: usize) -> VfsResult<()> {
    // SAFETY: sys_unlink's sequence from ilock(dp); the dirlookup ref
    // dies here (iunlockput), the wrapper refs stay with the glue.
    unsafe {
        let nb = name_buf(name, len)?;
        let dp = (*(dir as *mut VfsInode)).xv6_ip;
        begin_op();
        ilock(dp);
        // "." / ".." would hand ilock(ip) the already-locked dp.
        if namecmp(nb.as_ptr(), b".\0".as_ptr()) == 0 || namecmp(nb.as_ptr(), b"..\0".as_ptr()) == 0
        {
            iunlock(dp);
            end_op();
            return Err(VfsErr::Inval);
        }
        if (*dp).type_ != T_DIR {
            iunlock(dp);
            end_op();
            return Err(VfsErr::NotDir);
        }
        let mut off: u32 = 0;
        let ip = dirlookup(dp, nb.as_ptr(), &raw mut off);
        if ip.is_null() {
            iunlock(dp);
            end_op();
            return Err(VfsErr::NoEnt);
        }
        ilock(ip);
        if (*ip).nlink < 1 {
            panic!("unlink: nlink < 1");
        }
        if (*ip).type_ == T_DIR && !isdirempty(ip) {
            iunlock(ip);
            iunlock(dp);
            // The dirlookup ref goes back (ref-drop only: the pair
            // above closed, and a non-empty dir never truncates).
            iput(ip);
            end_op();
            return Err(VfsErr::NotEmpty);
        }
        let mut de = Dirent::default();
        if writei(
            dp,
            &mut de as *mut Dirent as *const u8,
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
        iunlock(dp);
        (*ip).nlink -= 1;
        iupdate(ip);
        // A live wrapper keeps the inode cached (no truncation); the
        // glue's phase C then zombie-defers its last iput.
        iunlockput(ip);
        end_op();
        Ok(())
    }
}

fn xv6_link(
    inode: *mut InodeCommon,
    dir: *mut InodeCommon,
    name: *const u8,
    len: usize,
) -> VfsResult<()> {
    // SAFETY: sys_link's sequence; the tx consumes no wrapper refs
    // (the glue pins both dentries across the call), and dirlink
    // stays the authoritative same-name check.
    unsafe {
        let nb = name_buf(name, len)?;
        let ip = (*(inode as *mut VfsInode)).xv6_ip;
        let dp = (*(dir as *mut VfsInode)).xv6_ip;
        begin_op();
        ilock(ip);
        if (*ip).type_ == T_DIR {
            // legacy EPERM; folds to EACCES at the syscall edge
            // (divergence recorded — no battery coverage).
            iunlock(ip);
            end_op();
            return Err(VfsErr::Access);
        }
        (*ip).nlink += 1;
        iupdate(ip);
        iunlock(ip);
        ilock(dp);
        // Legacy sys_link: a non-directory new-parent fails before
        // dirlink (subdir's link("dd/ff/ff") probe).
        if (*dp).type_ != T_DIR {
            iunlock(dp);
            ilock(ip);
            (*ip).nlink -= 1;
            iupdate(ip);
            iunlock(ip);
            end_op();
            return Err(VfsErr::NotDir);
        }
        if (*dp).dev != (*ip).dev || dirlink(dp, nb.as_ptr(), (*ip).inum) < 0 {
            iunlock(dp);
            // the sysfile.c `bad:` unwind — drop the bumped nlink.
            ilock(ip);
            (*ip).nlink -= 1;
            iupdate(ip);
            iunlock(ip);
            end_op();
            return Err(VfsErr::Io);
        }
        iunlock(dp);
        end_op();
        Ok(())
    }
}

/// Is directory `dp` (locked) empty apart from "." and ".."?
/// (sysfile.c isdirempty, migrated from syspath.rs.)
///
/// # Safety
/// `dp` locked and T_DIR.
unsafe fn isdirempty(dp: *mut Inode) -> bool {
    let mut de = Dirent::default();
    let mut off: u32 = 2 * core::mem::size_of::<Dirent>() as u32;
    // SAFETY: dp locked T_DIR; readi through the file layer core.
    unsafe {
        while off < (*dp).size {
            if readi(
                dp,
                &mut de as *mut Dirent as *mut u8,
                off,
                core::mem::size_of::<Dirent>() as u32,
            ) != core::mem::size_of::<Dirent>() as i32
            {
                panic!("isdirempty: readi");
            }
            if de.inum != 0 {
                return false;
            }
            off += core::mem::size_of::<Dirent>() as u32;
        }
    }
    true
}

fn notsup_rmdir(_dir: *mut InodeCommon, _name: *const u8, _len: usize) -> VfsResult<()> {
    Err(VfsErr::NotSup) // unlink folds rmdir (xv6 semantics)
}

fn notsup_rename(
    _od: *mut InodeCommon,
    _on: *const u8,
    _ol: usize,
    _nd: *mut InodeCommon,
    _nn: *const u8,
    _nl: usize,
) -> VfsResult<()> {
    Err(VfsErr::NotSup) // s6/tmpfs era
}

fn notsup_readlink(_i: *mut InodeCommon, _o: *mut u8, _c: usize) -> VfsResult<usize> {
    Err(VfsErr::NotSup) // no symlinks in xv6fs
}

fn notsup_symlink(
    _d: *mut InodeCommon,
    _n: *const u8,
    _l: usize,
    _t: *const u8,
    _tl: usize,
) -> VfsResult<*mut InodeCommon> {
    Err(VfsErr::NotSup)
}

/// FileOps/SuperOps live in xv6_obj.rs (object-level ops); the
/// re-export keeps the adapter surface one name (`xv6_adapt::XV6_*`).
pub use super::xv6_obj::{XV6_ASPACE_OPS, XV6_FILE_OPS, XV6_SUPER_OPS};

/// The namespace vtable (statics: fn-pointer structs, no
/// traits/classes); file/super tables in xv6_obj.rs.
pub static XV6_INODE_OPS: InodeOps = InodeOps {
    lookup: xv6_lookup,
    create: xv6_create,
    link: xv6_link,
    unlink: xv6_unlink,
    mkdir: xv6_mkdir,
    rmdir: notsup_rmdir,
    rename: notsup_rename,
    readlink: notsup_readlink,
    symlink: notsup_symlink,
    permission: None, // generic DAC over 0o777 always allows
};
