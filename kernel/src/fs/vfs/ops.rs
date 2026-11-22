//! ops.rs: the four VFS ops vtables (23 §1) and the common object
//! headers they dispatch over — struct + fn pointers, never traits or
//! classes, so an fs binds by embedding the header at the head of its
//! private object and recovering the tail by offset (the RcuHead
//! container_of discipline this kernel already uses).
//!
//! The M4 Core surface is frozen here: SuperOps {read_super, put_super,
//! statfs, remount?, sync_fs}, InodeOps {lookup, create, link, unlink,
//! mkdir, rmdir, rename, readlink, symlink, permission?}, FileOps
//! {read, write, iterate, llseek, ioctl?, fsync, mmap?, poll?} (AIO/
//! sendfile/inotify are M10 and must not grow back into this table),
//! AddressSpaceOps {readpage, writepage, write_begin, write_end} — the
//! page-typed signatures pinned in s3 (mapping is *mut AddressSpace,
//! pages are *mut PageDesc; the write family stays NotSup until s5).

use super::dentry::Dentry;
use super::namei::Cred;
use super::pagecache::{AddressSpace, PageDesc};
use super::{VfsErr, VfsResult};

/// Header embedded at the head of every superblock (fs-private tail
/// follows; sb keeps a back pointer `s_private` for container_of).
#[repr(C)]
pub struct SuperBlock {
    /// Device the fs was mounted from (0 for virtual fs).
    pub s_dev: u32,
    pub s_op: *const SuperOps,
    /// Root dentry of this mount (dcache-owned).
    pub s_root: *mut Dentry,
    /// fs-private tail (recover with offset arithmetic).
    pub s_private: *mut u8,
    /// Mount-time flags (s2 mount registry).
    pub s_flags: u64,
}

/// Header embedded at the head of every fs inode. `i_private` lets the
/// owning fs find its tail; the icache links (icache.rs) live inside
/// this header so inode cache management never needs the tail.
#[repr(C)]
pub struct InodeCommon {
    /// icache hash links (dev, ino key). Must stay first for the
    /// intrusive list discipline.
    pub i_hash_next: *mut InodeCommon,
    pub i_hash_pprev: *mut *mut InodeCommon,
    /// icache idle-LRU links (_unref'd inodes, head = oldest).
    pub i_lru_next: *mut InodeCommon,
    pub i_lru_prev: *mut InodeCommon,
    /// Reference count; 0 == idle (on the LRU, still hashed).
    pub i_count: u32,
    pub i_dev: u32,
    pub i_ino: u32,
    /// S_IF* type + rwx mode bits (mod.rs constants).
    pub i_mode: u16,
    pub i_uid: u32,
    pub i_gid: u32,
    pub i_nlink: u32,
    pub i_size: u64,
    pub i_op: *const InodeOps,
    pub i_fop: *const FileOps,
    pub i_sb: *mut SuperBlock,
    /// I_DIRTY-style state bits (writeback tags them in s5).
    pub i_state: u32,
    pub i_private: *mut u8,
    /// This inode's page cache (Linux: i_mapping usually == &i_data;
    /// a distinct target is tmpfs/shm-era, s6).
    pub i_mapping: *mut AddressSpace,
    /// The embedded address space i_mapping points at (m4r-s3).
    pub i_data: AddressSpace,
}

impl InodeCommon {
    pub const fn empty() -> InodeCommon {
        InodeCommon {
            i_hash_next: core::ptr::null_mut(),
            i_hash_pprev: core::ptr::null_mut(),
            i_lru_next: core::ptr::null_mut(),
            i_lru_prev: core::ptr::null_mut(),
            i_count: 0,
            i_dev: 0,
            i_ino: 0,
            i_mode: 0,
            i_uid: 0,
            i_gid: 0,
            i_nlink: 0,
            i_size: 0,
            i_op: core::ptr::null(),
            i_fop: core::ptr::null(),
            i_sb: core::ptr::null_mut(),
            i_state: 0,
            i_private: core::ptr::null_mut(),
            i_mapping: core::ptr::null_mut(),
            i_data: AddressSpace::EMPTY,
        }
    }

    #[inline]
    pub fn is_dir(&self) -> bool {
        super::imode_has_type(self.i_mode, super::S_IFDIR)
    }

    #[inline]
    pub fn is_symlink(&self) -> bool {
        super::imode_has_type(self.i_mode, super::S_IFLNK)
    }
}

/// Open-file header (the fd table wraps this; position and creds ride
/// per-open, mode gates read/write dispatch).
#[repr(C)]
pub struct FileCommon {
    pub f_count: u32,
    /// FMODE_READ | FMODE_WRITE (open flags normalization, s2).
    pub f_mode: u32,
    pub f_op: *const FileOps,
    pub f_pos: u64,
    pub f_inode: *mut InodeCommon,
    pub f_private: *mut u8,
}

pub const FMODE_READ: u32 = 0x1;
pub const FMODE_WRITE: u32 = 0x2;

/// Superblock ops (23 §1). `remount` is optional (NotSup default).
pub struct SuperOps {
    pub read_super: fn(dev: u32, sb: *mut SuperBlock) -> VfsResult<()>,
    pub put_super: fn(sb: *mut SuperBlock),
    pub statfs: fn(sb: *mut SuperBlock, out: *mut StatFs) -> VfsResult<()>,
    pub remount: Option<fn(sb: *mut SuperBlock, flags: u64) -> VfsResult<()>>,
    pub sync_fs: fn(sb: *mut SuperBlock, wait: bool) -> VfsResult<()>,
}

/// statfs(2) result snapshot (f_bsize-style core fields only).
#[repr(C)]
pub struct StatFs {
    pub f_type: u64,
    pub f_bsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
}

/// Inode ops (23 §1). `lookup` returns Ok(null) is NOT used: a miss is
/// `VfsErr::NoEnt` so the dcache layer can cache it negative without a
/// sentinel protocol. `permission` is optional: absent means the
/// generic DAC check (namei.rs) decides.
pub struct InodeOps {
    pub lookup:
        fn(dir: *mut InodeCommon, name: *const u8, len: usize) -> VfsResult<*mut InodeCommon>,
    pub create: fn(
        dir: *mut InodeCommon,
        name: *const u8,
        len: usize,
        mode: u16,
    ) -> VfsResult<*mut InodeCommon>,
    pub link: fn(
        inode: *mut InodeCommon,
        dir: *mut InodeCommon,
        name: *const u8,
        len: usize,
    ) -> VfsResult<()>,
    pub unlink: fn(dir: *mut InodeCommon, name: *const u8, len: usize) -> VfsResult<()>,
    pub mkdir: fn(
        dir: *mut InodeCommon,
        name: *const u8,
        len: usize,
        mode: u16,
    ) -> VfsResult<*mut InodeCommon>,
    pub rmdir: fn(dir: *mut InodeCommon, name: *const u8, len: usize) -> VfsResult<()>,
    pub rename: fn(
        old_dir: *mut InodeCommon,
        old_name: *const u8,
        old_len: usize,
        new_dir: *mut InodeCommon,
        new_name: *const u8,
        new_len: usize,
    ) -> VfsResult<()>,
    pub readlink: fn(inode: *mut InodeCommon, out: *mut u8, out_cap: usize) -> VfsResult<usize>,
    pub symlink: fn(
        dir: *mut InodeCommon,
        name: *const u8,
        len: usize,
        target: *const u8,
        target_len: usize,
    ) -> VfsResult<*mut InodeCommon>,
    /// MAY_* mask (namei.rs). None → generic DAC.
    pub permission: Option<fn(inode: *mut InodeCommon, mask: u32, cred: &Cred) -> VfsResult<()>>,
}

/// File ops — the M4 Core minimal set (23 §1 freezes it; AIO, sendfile,
/// inotify & co. are M10 Parity and banned from this table).
pub struct FileOps {
    pub read: fn(file: *mut FileCommon, buf: *mut u8, len: usize) -> VfsResult<usize>,
    pub write: fn(file: *mut FileCommon, buf: *const u8, len: usize) -> VfsResult<usize>,
    pub iterate: Option<
        fn(
            file: *mut FileCommon,
            dirent: *mut u8,
            filldir: fn(*mut u8, u64, *const u8, usize) -> VfsResult<()>,
        ) -> VfsResult<()>,
    >,
    pub llseek: fn(file: *mut FileCommon, off: i64, whence: u32) -> VfsResult<i64>,
    pub ioctl: Option<fn(file: *mut FileCommon, cmd: u32, arg: usize) -> VfsResult<isize>>,
    pub fsync: fn(file: *mut FileCommon) -> VfsResult<()>,
    pub mmap: Option<fn(file: *mut FileCommon, vma: *mut u8) -> VfsResult<()>>,
    pub poll: Option<fn(file: *mut FileCommon, mask: u32) -> VfsResult<u32>>,
}

/// Address-space (page cache) ops, signatures pinned in m4r-s3.
/// readpage fills page->data (one page at `index`; zero the tail past
/// EOF and set uptodate). The write family is s5 (NotSup until then).
pub struct AddressSpaceOps {
    pub readpage: fn(mapping: *mut AddressSpace, index: u64, page: *mut PageDesc) -> VfsResult<()>,
    pub writepage: fn(mapping: *mut AddressSpace, page: *mut PageDesc) -> VfsResult<()>,
    pub write_begin:
        fn(mapping: *mut AddressSpace, index: u64, page: *mut *mut PageDesc) -> VfsResult<()>,
    pub write_end:
        fn(mapping: *mut AddressSpace, page: *mut PageDesc, copied: usize) -> VfsResult<()>,
}

/// Dispatch helper: run `inode->i_op->permission` when the fs overrode
/// it, else the generic DAC check (namei.rs). Pure plumbing so both
/// call sites (path walk s2, open checks) share one spelling.
pub fn check_permission(inode: &InodeCommon, mask: u32, cred: &Cred) -> VfsResult<()> {
    let op = inode.i_op;
    if !op.is_null() {
        // SAFETY: i_op, when set, references a static vtable the fs owns.
        let over = unsafe { (*op).permission };
        if let Some(f) = over {
            // SAFETY: inode is live for the call (borrowed above).
            return f(inode as *const InodeCommon as *mut InodeCommon, mask, cred);
        }
    }
    if super::namei::dac_allows(inode.i_mode, inode.i_uid, inode.i_gid, cred, mask) {
        Ok(())
    } else {
        Err(VfsErr::Access)
    }
}
