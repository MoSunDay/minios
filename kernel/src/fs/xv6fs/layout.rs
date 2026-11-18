//! On-disk xv6 file-system format: port of `fs.h` + `stat.h` + the
//! file-system parts of `param.h` (xv6-public).
//!
//! On-disk layout (fs.h):
//!   Block 0 is unused.
//!   Block 1 is super block.
//!   Blocks 2 through sb.ninodes/IPB hold inodes.
//!   Then free bitmap blocks holding sb.size bits.
//!   Then sb.nblocks data blocks.
//!   Then sb.nlog log blocks (at the very end of the disk, see log.rs).
//!
//! Everything here is `const`-friendly so the kernel can use IBLOCK/BBLOCK
//! computations exactly like the C macros.

use core::mem::size_of;

// ---- fs.h ---------------------------------------------------------------

/// Root i-number (fs.h: `#define ROOTINO 1`).
pub const ROOTINO: u32 = 1;
/// Block size (fs.h: `#define BSIZE 512`).
pub const BSIZE: usize = 512;

/// File system super block (fs.h `struct superblock`).
///
/// NOTE: classic xv6-public only stores the first four words on disk;
/// `inodestart`/`bmapstart` are part of the minios on-disk format (mirroring
/// the later xv6 revision) and are written by `tools/mkfs.c`.  The kernel
/// keeps computing the same positions via [`ipb`]/[`iblock`]/[`bblock`] to
/// stay faithful to fs.c, where the super block fields are ignored.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Superblock {
    /// Size of file system image (blocks).
    pub size: u32,
    /// Number of data blocks.
    pub nblocks: u32,
    /// Number of inodes.
    pub ninodes: u32,
    /// Number of log blocks.
    pub nlog: u32,
    /// Block number of first inode block (minios extension, = 2).
    pub inodestart: u32,
    /// Block number of first free-map block (minios extension).
    pub bmapstart: u32,
}

/// Number of direct block addresses in an inode (fs.h).
pub const NDIRECT: usize = 12;
/// Indirect blocks per single-indirect block (fs.h: `BSIZE / sizeof(uint)`).
pub const NINDIRECT: usize = BSIZE / size_of::<u32>();
/// Maximum file size in blocks (fs.h).
pub const MAXFILE: usize = NDIRECT + NINDIRECT;

/// On-disk inode structure (fs.h `struct dinode`).
///
/// `addrs[NDIRECT]` holds the indirect block number; total 13 words.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DInode {
    /// File type (`T_DIR`/`T_FILE`/`T_DEV`); 0 == free.
    pub type_: i16,
    /// Major device number (T_DEV only).
    pub major: i16,
    /// Minor device number (T_DEV only).
    pub minor: i16,
    /// Number of links to inode in file system.
    pub nlink: i16,
    /// Size of file (bytes).
    pub size: u32,
    /// Data block addresses.
    pub addrs: [u32; NDIRECT + 1],
}

// ---- stat.h --------------------------------------------------------------

/// File types (stat.h / fs.h).
pub const T_DIR: i16 = 1; // Directory
pub const T_FILE: i16 = 2; // File
pub const T_DEV: i16 = 3; // Device

/// `struct stat` (stat.h), as filled in by `stati()`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
#[allow(dead_code)] // S4d: sys_fstat
pub struct Stat {
    /// Type of file.
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

// ---- directories (fs.h) --------------------------------------------------

/// Directory entry name size (fs.h: `#define DIRSIZ 14`).
pub const DIRSIZ: usize = 14;

/// A directory is a file containing a sequence of these (fs.h `struct dirent`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Dirent {
    pub inum: u16,
    pub name: [u8; DIRSIZ],
}

impl Default for Dirent {
    fn default() -> Self {
        Dirent {
            inum: 0,
            name: [0; DIRSIZ],
        }
    }
}

// ---- param.h (file-system subset; values per minios spec) ---------------

/// Open files in the whole system (file.h NFILE).
pub const NFILE: usize = 100;
/// Console device major number (file.h CONSOLE).
pub const CONSOLE: i16 = 1;
/// Maximum number of active i-nodes (param.h NINODE).
pub const NINODE: usize = 50;
/// Size of disk block cache (param.h NBUF).
#[allow(dead_code)] // bio.c port in fs/buffer.rs owns the live constant
pub const NBUF: usize = 30;
// NFILE (open files per system) lives in crate::fs (param.h side owned by
// file_table); layout.rs deliberately does not define it.
/// Max blocks in on-disk log / max data sectors per transaction (param.h LOGSIZE).
pub const LOGSIZE: usize = 30;
/// Max size of file-system operation in blocks (xv6-riscv param.h MAXOPBLOCKS;
/// used by the log allocator).
#[allow(dead_code)] // classic log.c hard-codes LOGSIZE instead (xv6-public)
pub const MAXOPBLOCKS: i32 = 10;
/// Maximum major device number (param.h NDEV).
pub const NDEV: usize = 10;
/// Device number of file system root disk (param.h ROOTDEV).
pub const ROOTDEV: u32 = 0; // minios: fs disk is IDE index 0 (ide.rs DEV_ROOT)
/// File system image size in blocks (advisory; `sb.size` on disk is
/// authoritative — the classic `tools/mkfs.c` builds a 1024-block image).
#[allow(dead_code)] // header value; mkfs.c owns the actual image size
pub const FSSIZE: u32 = 1000;

// ---- helper "macros" as const fns (fs.h) --------------------------------

/// Inodes per block (fs.h: `#define IPB (BSIZE / sizeof(struct dinode))`).
pub const fn ipb() -> usize {
    BSIZE / size_of::<DInode>()
}

/// Block containing inode i (fs.h: `#define IBLOCK(i) ((i) / IPB + 2)`).
pub const fn iblock(i: u32) -> u32 {
    i / ipb() as u32 + 2
}

/// Bitmap bits per block (fs.h: `#define BPB (BSIZE*8)`).
pub const fn bpb() -> u32 {
    (BSIZE * 8) as u32
}

/// Block containing bit for block b (fs.h: `#define BBLOCK(b, ninodes)`).
pub const fn bblock(b: u32, ninodes: u32) -> u32 {
    b / bpb() + ninodes / ipb() as u32 + 3
}

// ---- bit-flag accessors ---------------------------------------------------
// Rust has no bitfields; xv6 uses `flags & B_DIRTY` style masks. These tiny
// helpers are the accessor functions used across buffer.rs / inode.rs.

/// Is `bit` set in `flags`? (C: `(x->flags & bit) != 0`)
pub const fn has_flag(flags: i32, bit: i32) -> bool {
    flags & bit != 0
}

// Compile-time layout checks (C: static_assert in mkfs.c / panic in kernel).
const _: () = assert!(size_of::<DInode>() == 64); // BSIZE % sizeof(dinode) == 0
const _: () = assert!(size_of::<Dirent>() == 16); // BSIZE % sizeof(dirent) == 0
const _: () = assert!(BSIZE % size_of::<DInode>() == 0);
const _: () = assert!(BSIZE % size_of::<Dirent>() == 0);
const _: () = assert!(size_of::<Superblock>() <= BSIZE);
const _: () = assert!(NDIRECT + NINDIRECT == MAXFILE);
