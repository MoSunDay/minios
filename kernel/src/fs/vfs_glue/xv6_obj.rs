//! xv6_obj (m4r-s2b): the object-level xv6fs vtables — FileOps and
//! SuperOps. Consumers land in s3+ (the fd table still runs the
//! legacy fileread/filewrite path this milestone; read_super runs
//! from the s2 mount registry later), so these are the minimal
//! honest bindings over the legacy inode primitives.

use super::wrapper::VfsInode;
use crate::fs::vfs::ops::SuperOps;
use crate::fs::vfs::ops::{FileCommon, FileOps, StatFs, SuperBlock};
use crate::fs::vfs::{VfsErr, VfsResult};
use crate::fs::xv6fs::inode::{ilock, iunlock, iupdate, readi, writei};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::fs::xv6fs::{readsb, Superblock as Xv6Sb};

// ---- FileOps (consumers land in s3+: the fd table still runs the
// legacy fileread/filewrite path this milestone) ----

fn xv6_fread(file: *mut FileCommon, buf: *mut u8, len: usize) -> VfsResult<usize> {
    // SAFETY: f_inode is a wrapper; readi dispatches T_DEV internally.
    unsafe {
        let ip = (*((*file).f_inode as *mut VfsInode)).xv6_ip;
        ilock(ip);
        let n = readi(ip, buf, (*file).f_pos as u32, len as u32);
        iunlock(ip);
        if n < 0 {
            return Err(VfsErr::Io);
        }
        (*file).f_pos += n as u64;
        Ok(n as usize)
    }
}

fn xv6_fwrite(file: *mut FileCommon, buf: *const u8, len: usize) -> VfsResult<usize> {
    // SAFETY: as xv6_fread.
    unsafe {
        let ip = (*((*file).f_inode as *mut VfsInode)).xv6_ip;
        ilock(ip);
        let n = writei(ip, buf, (*file).f_pos as u32, len as u32);
        iunlock(ip);
        if n < 0 {
            return Err(VfsErr::Io);
        }
        (*file).f_pos += n as u64;
        Ok(n as usize)
    }
}

fn xv6_llseek(file: *mut FileCommon, off: i64, whence: u32) -> VfsResult<i64> {
    // SAFETY: caller-owned file header; raw-pointer field access.
    unsafe {
        let base = match whence {
            0 => 0,
            1 => (*file).f_pos as i64,
            // SAFETY: size read under the inode lock.
            2 => {
                let ip = (*((*file).f_inode as *mut VfsInode)).xv6_ip;
                ilock(ip);
                let s = (*ip).size as i64;
                iunlock(ip);
                s
            }
            _ => return Err(VfsErr::Inval),
        };
        let pos = base + off;
        if pos < 0 {
            return Err(VfsErr::Inval);
        }
        (*file).f_pos = pos as u64;
        Ok(pos)
    }
}

fn xv6_fsync(file: *mut FileCommon) -> VfsResult<()> {
    // SAFETY: dinode rewrite inside a transaction (iupdate logs a
    // block write).
    unsafe {
        let ip = (*((*file).f_inode as *mut VfsInode)).xv6_ip;
        begin_op();
        ilock(ip);
        iupdate(ip);
        iunlock(ip);
        end_op();
    }
    Ok(())
}

// ---- SuperOps ----

fn xv6_read_super(dev: u32, sb: *mut SuperBlock) -> VfsResult<()> {
    if dev != crate::fs::xv6fs::layout::ROOTDEV {
        return Err(VfsErr::NoEnt); // the only mount this milestone
    }
    // SAFETY: caller-owned superblock header.
    unsafe { (*sb).s_dev = dev };
    Ok(())
}

fn xv6_put_super(_sb: *mut SuperBlock) {
    // the root mount never unmounts in M4
}

fn xv6_statfs(sb: *mut SuperBlock, out: *mut StatFs) -> VfsResult<()> {
    // SAFETY: caller-owned header + snapshot; readsb blocks (boot or
    // process context).
    unsafe {
        let xsb: Xv6Sb = readsb((*sb).s_dev);
        (*out).f_type = 0; // xv6fs carries no magic
        (*out).f_bsize = crate::fs::xv6fs::layout::BSIZE as u64;
        (*out).f_blocks = xsb.size as u64;
        (*out).f_files = xsb.ninodes as u64;
        // xv6fs keeps its free map on disk only; unknowns stay 0.
        (*out).f_bfree = 0;
        (*out).f_bavail = 0;
        (*out).f_ffree = 0;
    }
    Ok(())
}

fn xv6_sync_fs(_sb: *mut SuperBlock, _wait: bool) -> VfsResult<()> {
    // xv6fs flushes through the log at end_op; nothing extra here.
    Ok(())
}

pub static XV6_FILE_OPS: FileOps = FileOps {
    read: xv6_fread,
    write: xv6_fwrite,
    iterate: None, // s3+ (getdents consumer)
    llseek: xv6_llseek,
    ioctl: None,
    fsync: xv6_fsync,
    mmap: None, // s4
    poll: None, // s9 (tty)
};

pub static XV6_SUPER_OPS: SuperOps = SuperOps {
    read_super: xv6_read_super,
    put_super: xv6_put_super,
    statfs: xv6_statfs,
    remount: None,
    sync_fs: xv6_sync_fs,
};
