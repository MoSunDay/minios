//! fs: on-disk file system layer. S4 (plans/linux26/04 §3/§5) brings the
//! xv6 root image online: `buffer` is the block cache (bio.c), the xv6fs
//! sublayer (log/inode/namei) and the file table land with the file
//! syscalls.

pub mod buffer;
pub mod file_table;
pub mod xv6fs;

/// Block size in bytes (tools/fs.h BSIZE; shared with mkfs).
pub const BSIZE: usize = 512;

/// fcntl.h open flags (O_CREATE is xv6's only extension). Used by the
/// S4 file syscalls.
#[allow(dead_code)] // S4d: sys_open lands with the file table
pub const O_RDONLY: i32 = 0x000;
#[allow(dead_code)] // S4d
pub const O_WRONLY: i32 = 0x001;
#[allow(dead_code)] // S4d
pub const O_RDWR: i32 = 0x002;
#[allow(dead_code)] // S4d
pub const O_CREATE: i32 = 0x200;
