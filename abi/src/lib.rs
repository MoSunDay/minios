//! Shared minios ABI (M1R): the Linux 2.6.39.4 x86-64 syscall whitelist,
//! errno values, and auxv keys used by both the kernel and the Rust user
//! runtime. Neither side may keep a private copy of these tables
//! (plans/linux26/02 §5, plans/linux26/04 §1).

#![no_std]

/// Linux 2.6.39.4 x86-64 syscall numbers implemented (whitelist subset).
/// Numbers outside this table answer `-ENOSYS`; the table matches the
/// authoritative syscall_64.tbl numbering, not xv6's.
pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_BRK: u64 = 12;
pub const SYS_PIPE: u64 = 22;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_GETPID: u64 = 39;
pub const SYS_FORK: u64 = 57;
pub const SYS_EXIT: u64 = 60;
pub const SYS_FSTAT: u64 = 5;
pub const SYS_DUP: u64 = 32;
pub const SYS_MKDIR: u64 = 83;
pub const SYS_CHDIR: u64 = 80;
pub const SYS_LINK: u64 = 86;
pub const SYS_UNLINK: u64 = 87;
pub const SYS_NANOSLEEP: u64 = 35;
pub const SYS_MKNOD: u64 = 133;
pub const SYS_WAIT4: u64 = 61;
pub const SYS_KILL: u64 = 62;
pub const SYS_EXIT_GROUP: u64 = 231;

/// errno values (asm-generic errno-base.h) used by the M1R whitelist.
pub const EPERM: i64 = -1;
pub const ENOENT: i64 = -2;
pub const ESRCH: i64 = -3;
pub const EINTR: i64 = -4;
pub const EIO: i64 = -5;
pub const ENOEXEC: i64 = -8;
pub const EBADF: i64 = -9;
pub const ECHILD: i64 = -10;
pub const EAGAIN: i64 = -11;
pub const ENOMEM: i64 = -12;
pub const EACCES: i64 = -13;
pub const EFAULT: i64 = -14;
pub const EBUSY: i64 = -16;
pub const EEXIST: i64 = -17;
pub const ENOTDIR: i64 = -20;
pub const EISDIR: i64 = -21;
pub const EINVAL: i64 = -22;
pub const EMFILE: i64 = -24;
pub const ENOSPC: i64 = -28;
pub const EPIPE: i64 = -32;
pub const ERANGE: i64 = -34;
pub const ENAMETOOLONG: i64 = -36;
pub const ENOSYS: i64 = -38;
pub const ENOTEMPTY: i64 = -39;

/// open(2) flags (asm-generic, x86-64): the M1R subset. xv6's only
/// extension O_CREATE maps to Linux O_CREAT (same semantic: create if
/// missing); O_EXCL/O_TRUNC/O_APPEND parse but only affect create logic.
pub const O_RDONLY: u64 = 0o0;
pub const O_WRONLY: u64 = 0o1;
pub const O_RDWR: u64 = 0o2;
pub const O_CREAT: u64 = 0o100;
pub const O_EXCL: u64 = 0o200;
pub const O_TRUNC: u64 = 0o1000;
pub const O_APPEND: u64 = 0o2000;

/// Linux syscall return contract: values in -4095..=-1 carry an errno
/// (plans/linux26/04 §3); the user runtime converts exactly this range.
#[inline]
pub const fn is_neg_errno(v: i64) -> bool {
    v >= -4095 && v < 0
}

/// auxv keys (x86-64 ELF ABI). M1R exec seeds a minimal chain terminated
/// by AT_NULL; richer keys arrive with later milestones.
pub const AT_NULL: u64 = 0;

/// fstat/stat result layout (the kernel's xv6 Stati snapshot, shared by
/// the syscall surface so user programs decode the same bytes; repr(C),
/// 20 bytes on x86-64).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Stat {
    /// Type of file (T_DIR=1, T_FILE=2, T_DEV=3).
    pub type_: i16,
    /// File system's disk device.
    pub dev: i32,
    /// Inode number.
    pub ino: u32,
    /// Number of links to file.
    pub nlink: i16,
    /// Size of file in bytes.
    pub size: u32,
}

impl Stat {
    pub const T_DIR: i16 = 1;
    pub const T_FILE: i16 = 2;
    pub const T_DEV: i16 = 3;
}
