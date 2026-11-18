//! Engine self-tests: layout invariants, determinism, and round-trip of
//! file contents through the on-disk structures we just wrote.

use super::*;

fn build(files: &[InputFile<'_>]) -> (Vec<u8>, Superblock, u32) {
    build_image(files)
}

fn dirent_at(img: &[u8], idx: usize) -> (u16, [u8; DIRSIZ]) {
    let dip = parse_dinode(&img[2 * BSIZE..], 1);
    let b0 = dip.addrs[0] as usize * BSIZE;
    let off = idx * 16;
    let inum = u16::from_le_bytes([img[b0 + off], img[b0 + off + 1]]);
    let mut name = [0u8; DIRSIZ];
    name.copy_from_slice(&img[b0 + off + 2..b0 + off + 2 + DIRSIZ]);
    (inum, name)
}

#[test]
fn geometry_tiles_disk() {
    let (img, sb, used) = build(&[]);
    assert_eq!(img.len(), 1024 * 512);
    assert_eq!(sb.size, 1024);
    assert_eq!(sb.ninodes, 200);
    assert_eq!(sb.nlog, 30);
    // mkfs.c tiles the disk before any data allocation: metadata blocks
    // (29) + nblocks + nlog == size; file data then comes out of the
    // nblocks budget. Empty root still spends one data block on "."/"..".
    assert_eq!(sb.nblocks + 29 + sb.nlog, 1024);
    assert_eq!(used, 30);
    // superblock bytes at block 1 (little-endian, 24 bytes then zeros)
    assert_eq!(&img[512..516], &1024u32.to_le_bytes());
}

#[test]
fn root_dir_shape_and_roundtrip() {
    let payload: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
    let files = [InputFile {
        name: "hello",
        data: &payload,
    }];
    let (img, _sb, used) = build(&files);
    // '.' and '..' first, then "hello"
    let (dot, _) = dirent_at(&img, 0);
    let (dotdot, _) = dirent_at(&img, 1);
    let (hello_inum, name) = dirent_at(&img, 2);
    assert_eq!(dot, 1);
    assert_eq!(dotdot, 1);
    assert_eq!(hello_inum, 2);
    assert_eq!(&name[..5], b"hello");
    // inode 2: T_FILE, nlink 1, size 1000, first data block holds the head
    let dip = parse_dinode(&img[2 * BSIZE..], 2);
    assert_eq!(dip.type_, 2);
    assert_eq!(dip.nlink, 1);
    assert_eq!(dip.size, 1000);
    let b0 = dip.addrs[0] as usize * BSIZE;
    assert_eq!(&img[b0..b0 + 8], &payload[..8]);
    // bitmap marks exactly `used` bits (bmapstart = ninodes/IPB + 3 = 28)
    let bm = 28 * BSIZE;
    let bits: u32 = (0..used as usize)
        .filter(|b| img[bm + b / 8] & (1 << (b % 8)) != 0)
        .count() as u32;
    assert_eq!(bits, used);
    // blocks beyond `used` are free
    assert_eq!(img[bm + used as usize / 8] >> (used % 8), 0);
}

#[test]
fn indirect_blocks_used_for_large_file() {
    // > NDIRECT blocks forces the indirect path (single indirect block
    // holds up to 128 pointers).
    let payload = vec![0xABu8; (NDIRECT + 5) * BSIZE];
    let files = [InputFile {
        name: "big",
        data: &payload,
    }];
    let (img, _sb, used) = build(&files);
    let dip = parse_dinode(&img[2 * BSIZE..], 2);
    assert_eq!(dip.size as usize, payload.len());
    assert_ne!(dip.addrs[NDIRECT], 0);
    let ind = dip.addrs[NDIRECT] as usize * BSIZE;
    let last_ptr = u32::from_le_bytes(img[ind + 4 * (4)..ind + 4 * 4 + 4].try_into().unwrap());
    let tail = last_ptr as usize * BSIZE;
    assert_eq!(&img[tail..tail + BSIZE], &[0xAB; BSIZE]);
    // 29 metadata + 1 root-dir block + 17 data + 1 indirect
    assert_eq!(used, 48);
}

#[test]
fn deterministic_output() {
    let a: Vec<u8> = (0..4096).map(|i| (i * 7) as u8).collect();
    let b: Vec<u8> = (0..17).map(|i| i as u8).collect();
    let files1 = [
        InputFile {
            name: "a",
            data: &a,
        },
        InputFile {
            name: "b",
            data: &b,
        },
    ];
    let files2 = [
        InputFile {
            name: "a",
            data: &a,
        },
        InputFile {
            name: "b",
            data: &b,
        },
    ];
    let (img1, _, _) = build_image(&files1);
    let (img2, _, _) = build_image(&files2);
    assert_eq!(img1, img2);
}

#[test]
#[should_panic(expected = "name longer than DIRSIZ")]
fn oversize_name_rejected() {
    let d = [0u8; 4];
    let files = [InputFile {
        name: "0123456789ABCDE",
        data: &d,
    }];
    let _ = build_image(&files);
}
