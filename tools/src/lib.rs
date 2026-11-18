//! Rust mkfs engine (plans/linux26/04 §5: M1R–M4 images are xv6 roots
//! built by the Rust toolchain; decision D2 retires tools/mkfs.c).
//!
//! The on-disk format is the classic xv6 one frozen in tools/fs.h /
//! kernel `fs/xv6fs/layout.rs`: block 0 unused, superblock at block 1,
//! inode blocks from block 2, then the free bitmap, then log blocks and
//! data blocks. The builder is a faithful port of mkfs.c: same block
//! accounting (nblocks + usedblocks + nlog == size), same allocation
//! order (inodes ascending from ROOTINO, data blocks ascending after
//! the metadata), same root-directory rounding, and byte-identical
//! output for identical inputs.
//!
//! Pure function core: [`build_image`] turns an ordered file list into a
//! complete image buffer; no I/O, no global state (project rule: no
//! classes, no hidden mutation).

/// Block size in bytes (BSIZE).
pub const BSIZE: usize = 512;
/// Root i-number (ROOTINO).
pub const ROOTINO: u32 = 1;
/// Direct block pointers per inode (NDIRECT).
pub const NDIRECT: usize = 12;
/// Pointers in one indirect block (NINDIRECT).
pub const NINDIRECT: usize = BSIZE / 4;
/// Maximum file blocks (MAXFILE).
pub const MAXFILE: usize = NDIRECT + NINDIRECT;
/// Dirent name length (DIRSIZ).
pub const DIRSIZ: usize = 14;

/// File types (stat.h): directory / file / device.
pub const T_DIR: u16 = 1;
pub const T_FILE: u16 = 2;
pub const T_DEV: u16 = 3;

/// On-disk superblock (24 bytes, little-endian on disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superblock {
    pub size: u32,
    pub nblocks: u32,
    pub ninodes: u32,
    pub nlog: u32,
    /// minios extension: first inode block is always 2.
    pub inodestart: u32,
    /// minios extension: first bitmap block (ninodes/IPB + 3).
    pub bmapstart: u32,
}

/// On-disk inode (64 bytes, little-endian on disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dinode {
    pub type_: u16,
    pub major: u16,
    pub minor: u16,
    pub nlink: u16,
    pub size: u32,
    pub addrs: [u32; NDIRECT + 1],
}

/// One input file: directory-entry name plus full contents.
pub struct InputFile<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// Classic mkfs.c geometry (965 data + 29 used + 30 log == 1024 blocks).
const fn classic_geometry() -> (u32, u32, u32, u32) {
    let nblocks = 965;
    let nlog = 30; // LOGSIZE in the kernel log.rs
    let ninodes = 200;
    let size = 1024;
    (nblocks, nlog, ninodes, size)
}

/// Inodes per block (IPB = 512/64 = 8).
const IPB: u32 = (BSIZE / 64) as u32;
/// Bitmap blocks (size/(512*8) + 1).
fn bitblocks(size: u32) -> u32 {
    size / (BSIZE as u32 * 8) + 1
}

/// Block holding inode `i` (IBLOCK).
fn iblock(i: u32) -> u32 {
    i / IPB + 2
}

/// Serialize a superblock into `dst` (little-endian, exact 24 bytes).
fn put_superblock(sb: &Superblock, dst: &mut [u8]) {
    let mut o = 0;
    for v in [
        sb.size,
        sb.nblocks,
        sb.ninodes,
        sb.nlog,
        sb.inodestart,
        sb.bmapstart,
    ] {
        dst[o..o + 4].copy_from_slice(&v.to_le_bytes());
        o += 4;
    }
}

/// Serialize a dinode into `dst` (little-endian, exact 64 bytes).
fn put_dinode(dip: &Dinode, dst: &mut [u8]) {
    let mut o = 0;
    for v in [dip.type_, dip.major, dip.minor, dip.nlink] {
        dst[o..o + 2].copy_from_slice(&v.to_le_bytes());
        o += 2;
    }
    dst[o..o + 4].copy_from_slice(&dip.size.to_le_bytes());
    o += 4;
    for a in &dip.addrs {
        dst[o..o + 4].copy_from_slice(&a.to_le_bytes());
        o += 4;
    }
}

/// Parse a dinode back (unit tests read the image they just built).
pub fn parse_dinode(block: &[u8], idx: usize) -> Dinode {
    let b = &block[idx * 64..idx * 64 + 64];
    let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut addrs = [0u32; NDIRECT + 1];
    for (i, a) in addrs.iter_mut().enumerate() {
        *a = u32at(12 + i * 4);
    }
    Dinode {
        type_: u16at(0),
        major: u16at(2),
        minor: u16at(4),
        nlink: u16at(6),
        size: u32at(8),
        addrs,
    }
}

/// Serialize a dirent (16 bytes: u16 inum + name bytes, NUL padded).
fn put_dirent(inum: u32, name: &str, dst: &mut [u8]) {
    dst[..2].copy_from_slice(&(inum as u16).to_le_bytes());
    let n = name.as_bytes();
    let n = n.len().min(DIRSIZ);
    dst[2..2 + n].copy_from_slice(&name.as_bytes()[..n]);
}

/// Build the complete image. `files` become root-directory entries in
/// order, after `.` and `..`. Returns the image plus the used-block
/// count (for the summary line). Panics on layout violations exactly
/// where mkfs.c asserted (oversized file, block exhaustion).
pub fn build_image(files: &[InputFile<'_>]) -> (Vec<u8>, Superblock, u32) {
    let (nblocks, nlog, ninodes, size) = classic_geometry();
    let bitblks = bitblocks(size);
    let mut usedblocks = ninodes / IPB + 3 + bitblks;
    let mut freeblock = usedblocks;

    let total_blocks = nblocks + usedblocks + nlog;
    assert_eq!(total_blocks, size, "geometry must tile the whole disk");
    let mut img = vec![0u8; size as usize * BSIZE];

    let bmapstart = ninodes / IPB + 3;
    let sb = Superblock {
        size,
        nblocks,
        ninodes,
        nlog,
        inodestart: 2,
        bmapstart,
    };
    {
        let mut sbb = [0u8; BSIZE];
        put_superblock(&sb, &mut sbb);
        write_block(&mut img, 1, &sbb);
    }

    let mut freeinode: u32 = ROOTINO;
    // inode read-modify-write helpers over the image buffer
    let read_inode = |img: &[u8], inum: u32| -> Dinode {
        let blk = iblock(inum);
        parse_dinode(&img[blk as usize * BSIZE..], (inum % IPB) as usize)
    };
    let write_inode = |img: &mut [u8], inum: u32, dip: &Dinode| {
        let blk = iblock(inum);
        let off = blk as usize * BSIZE + (inum % IPB) as usize * 64;
        let mut buf = [0u8; 64];
        put_dinode(dip, &mut buf);
        img[off..off + 64].copy_from_slice(&buf);
    };
    let ialloc = |img: &mut [u8], freeinode: &mut u32, type_: u16| -> u32 {
        let inum = *freeinode;
        *freeinode += 1;
        write_inode(
            img,
            inum,
            &Dinode {
                type_,
                major: 0,
                minor: 0,
                nlink: 1,
                size: 0,
                addrs: [0; NDIRECT + 1],
            },
        );
        inum
    };

    let rootino = ialloc(&mut img, &mut freeinode, T_DIR);
    assert_eq!(rootino, ROOTINO);

    let append =
        |img: &mut [u8], usedblocks: &mut u32, freeblock: &mut u32, inum: u32, data: &[u8]| {
            let mut dip = read_inode(img, inum);
            let mut off = dip.size;
            let mut p = 0;
            while p < data.len() {
                let fbn = off / BSIZE as u32;
                assert!(fbn < MAXFILE as u32, "file too big (MAXFILE)");
                if fbn < NDIRECT as u32 {
                    if dip.addrs[fbn as usize] == 0 {
                        dip.addrs[fbn as usize] = *freeblock;
                        *freeblock += 1;
                        *usedblocks += 1;
                    }
                } else {
                    if dip.addrs[NDIRECT] == 0 {
                        dip.addrs[NDIRECT] = *freeblock;
                        *freeblock += 1;
                        *usedblocks += 1;
                    }
                    let indblk = dip.addrs[NDIRECT] as usize * BSIZE;
                    let idx = fbn as usize - NDIRECT;
                    let cur = u32::from_le_bytes(
                        img[indblk + idx * 4..indblk + idx * 4 + 4]
                            .try_into()
                            .unwrap(),
                    );
                    if cur == 0 {
                        let nb = *freeblock;
                        *freeblock += 1;
                        *usedblocks += 1;
                        img[indblk + idx * 4..indblk + idx * 4 + 4]
                            .copy_from_slice(&nb.to_le_bytes());
                    }
                }
                let bn = if fbn < NDIRECT as u32 {
                    dip.addrs[fbn as usize]
                } else {
                    let indblk = dip.addrs[NDIRECT] as usize * BSIZE;
                    let idx = fbn as usize - NDIRECT;
                    u32::from_le_bytes(
                        img[indblk + idx * 4..indblk + idx * 4 + 4]
                            .try_into()
                            .unwrap(),
                    )
                };
                let n1 = (BSIZE as u32 - off % BSIZE as u32).min((data.len() - p) as u32);
                let start = bn as usize * BSIZE + (off % BSIZE as u32) as usize;
                img[start..start + n1 as usize].copy_from_slice(&data[p..p + n1 as usize]);
                off += n1;
                p += n1 as usize;
            }
            dip.size = off;
            write_inode(img, inum, &dip);
        };

    let mut de = [0u8; 16];
    put_dirent(rootino, ".", &mut de);
    append(&mut img, &mut usedblocks, &mut freeblock, rootino, &de);
    let mut de = [0u8; 16];
    put_dirent(rootino, "..", &mut de);
    append(&mut img, &mut usedblocks, &mut freeblock, rootino, &de);

    for f in files {
        let name = f.name;
        assert!(!name.contains('/'), "plain names only: {name}");
        assert!(name.len() <= DIRSIZ, "name longer than DIRSIZ: {name}");
        let inum = ialloc(&mut img, &mut freeinode, T_FILE);
        let mut de = [0u8; 16];
        put_dirent(inum, name, &mut de);
        append(&mut img, &mut usedblocks, &mut freeblock, rootino, &de);
        append(&mut img, &mut usedblocks, &mut freeblock, inum, f.data);
    }

    // Round the root directory size up to a whole block (mkfs.c).
    let mut din = read_inode(&img, rootino);
    din.size = (din.size / BSIZE as u32 + 1) * BSIZE as u32;
    write_inode(&mut img, rootino, &din);

    // Mark every used block in the bitmap (balloc(usedblocks)).
    let bm = bmapstart as usize * BSIZE;
    for b in 0..usedblocks {
        img[bm + (b / 8) as usize] |= 1 << (b % 8);
    }
    (img, sb, usedblocks)
}

/// Write one 512-byte block into the image buffer.
fn write_block(img: &mut [u8], block: u32, src: &[u8]) {
    let off = block as usize * BSIZE;
    img[off..off + BSIZE].copy_from_slice(src);
}

#[cfg(test)]
mod tests;
