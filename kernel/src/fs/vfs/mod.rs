//! fs/vfs: VFS object model, pure core (M4 s1, 23 §1).
//!
//! Zero kernel dependencies by design: the module tree is included by
//! path in host tests (no test-only copy to drift), exactly like the
//! m3 rcu_engine core. Kernel glue — locks, mount registration, the
//! RCU-deferred negative-dentry release, the xv6fs adapter — lands with
//! s2 and wraps these definitions; nothing here may reach back into
//! kernel services. The lock hierarchy for the UP kernel (dcache vs
//! icache vs inode locks) is ADR-0023 material and lives there.
//!
//! Error model: pure-core failures are `VfsErr` variants; the syscall
//! glue maps them onto `abi` errnos (02 §5 spelling of xv6's blanket
//! -1) so this tree stays host-buildable without the abi crate.
//!
//! No classes (global rule): ops dispatch is a struct of fn pointers
//! (`ops.rs`), the C vtable discipline this kernel already uses for
//! devsw and RcuHead callbacks.

#![allow(dead_code)] // s2 glue is the first kernel consumer

pub mod dcache;
pub mod dentry;
pub mod devict;
pub mod icache;
pub mod namei;
pub mod ops;
pub mod pagecache;

/// stat.h file-type bits (Linux values; shared by DAC and type checks).
pub const S_IFMT: u16 = 0o170_000;
pub const S_IFLNK: u16 = 0o120_000;
pub const S_IFREG: u16 = 0o100_000;
pub const S_IFDIR: u16 = 0o040_000;
pub const S_IFCHR: u16 = 0o020_000;
pub const S_IFBLK: u16 = 0o060_000;

#[inline]
pub const fn imode_has_type(mode: u16, ty: u16) -> bool {
    mode & S_IFMT == ty
}

/// Pure-core failure spelling (mapped to abi errnos by the s2 glue).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VfsErr {
    /// ENOENT: missing name (lookup miss outside a negative cache).
    NoEnt,
    /// ENOTDIR: a path component used as a directory is not one.
    NotDir,
    /// EISDIR: write/open-for-write on a directory.
    IsDir,
    /// EACCES: DAC mode/caps refusal.
    Access,
    /// ELOOP: symlink budget exhausted.
    SymLoop,
    /// ENAMETOOLONG: component or full path over budget.
    NameTooLong,
    /// EEXIST: exclusive create hit an existing name.
    Exist,
    /// ENOTEMPTY: rmdir/rename-over a non-empty directory.
    NotEmpty,
    /// EINVAL: malformed operation (e.g. rename under own subtree).
    Inval,
    /// EBUSY: object still referenced (dentry/inode cannot retire).
    Busy,
    /// EIO: backing store refusal (s5/s7 error propagation).
    Io,
    /// ENOSPC: allocation refusal below the VFS.
    NoSpc,
    /// ENOSYS/EOPNOTSUPP: op absent from this vtable (M4 minimal set).
    NotSup,
}

/// Convenience alias for pure-core fallible operations.
pub type VfsResult<T> = Result<T, VfsErr>;
