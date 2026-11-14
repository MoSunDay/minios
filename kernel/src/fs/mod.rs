//! fs: file system layer — xv6 file.c/types live here; the on-disk file
//! system implementation (bio.c/fs.c/iget etc.) is crate::fs::xv6fs and
//! crate::fs::buffer. This module holds:
//!   * fcntl.h constants (open flags)
//!   * param.h file-table constants (NFILE/NOFILE — xv6-public values;
//!     layout.rs intentionally carries no NFILE)
//!   * file.c port in file_table (File/FileTable + filealloc/filedup/
//!     fileclose/filestat/fileread/filewrite)
//!
//! Inode/Stat/Dirent/DIRSIZ/T_* /BSIZE/NDIRECT are re-exported from
//! xv6fs::layout so there is exactly one definition in the crate.
//!
//! CONTRACTS (as landed):
//!   * crate::fs::xv6fs::log::{begin_op(), end_op()} — safe fns;
//!     loginit(dev) (= xv6 initlog) also re-exported from log.
//!   * crate::fs::xv6fs::namei::{namei(*const u8), nameiparent(*const u8,
//!     &mut [u8; DIRSIZ]), namecmp} — all unsafe fns.
//!   * crate::fs::xv6fs::inode::{iget, ialloc, ilock, iunlock, iunlockput,
//!     iput, idup, iupdate, stati, readi, writei, register_dev} — unsafe fns
//!     taking raw pointers; begin_op/end_op and the namei family are
//!     re-exported through inode for one-stop imports.
//!   * Locking uses crate::kernel::sched::{RawLock, acquire, release,
//!     sleep, wakeup}: each xv6 embedded-spinlock struct becomes a data
//!     `static mut` plus a sibling lock static (BCACHE_LOCK, ICACHE_LOCK,
//!     LOG_LOCK, IDELOCK — the sched TICKS/TICKS_LOCK idiom), so waiters
//!     call sched::sleep(chan, &lock) exactly like xv6.

pub mod buffer;
pub mod file_table;
pub mod xv6fs;

pub use xv6fs::layout::{Dirent, Stat, DIRSIZ, T_DEV, T_DIR, T_FILE};

// fcntl.h
pub const O_RDONLY: i32 = 0x000;
pub const O_WRONLY: i32 = 0x001;
pub const O_RDWR: i32 = 0x002;
pub const O_CREATE: i32 = 0x200;

// param.h (xv6-public values; file.c consumers)
pub const NFILE: usize = 100; // open files per system
pub const NOFILE: usize = 16; // open files per process
