use super::{bitmap, elf, string};

#[test]
fn bitmap_set_clear_test() {
    let mut map = [0u8; 3];
    assert!(!bitmap::test(&map, 0));
    bitmap::set(&mut map, 0);
    bitmap::set(&mut map, 8);
    bitmap::set(&mut map, 23);
    assert!(bitmap::test(&map, 0));
    assert!(bitmap::test(&map, 8));
    assert!(bitmap::test(&map, 23));
    assert_eq!(map, [0x01, 0x01, 0x80]);
    bitmap::clear(&mut map, 8);
    assert!(!bitmap::test(&map, 8));
    // Out-of-range operations are no-ops, not panics.
    bitmap::set(&mut map, 999);
    assert!(!bitmap::test(&map, 999));
    bitmap::clear(&mut map, 999);
}

#[test]
fn bitmap_scan_and_count() {
    assert_eq!(bitmap::find_first_zero(&[0xFF, 0xFF]), None);
    assert_eq!(bitmap::find_first_zero(&[0xFF, 0b1111_1101]), Some(9));
    assert_eq!(bitmap::find_first_zero(&[0x00]), Some(0));
    assert_eq!(bitmap::find_first_set(&[0x00, 0x00]), None);
    assert_eq!(bitmap::find_first_set(&[0x00, 0b0010_0000]), Some(13));
    assert_eq!(bitmap::find_first_set(&[0x80]), Some(7));
    assert_eq!(bitmap::count_zeros(&[0xFF, 0b1111_1100], 8), 0);
    assert_eq!(bitmap::count_zeros(&[0xFF, 0b1111_1100], 10), 2);
    assert_eq!(bitmap::count_zeros(&[0x00, 0x00], 16), 16);
}

#[test]
fn string_copy_compare_len() {
    let mut dst = [0u8; 8];
    // Contract: dst has 8 writable bytes; length is within bounds.
    string::memset(dst.as_mut_ptr(), 0x5A, dst.len());
    assert_eq!(dst, [0x5A; 8]);

    let src = *b"abcdef";
    // Contract: src and dst are 6-byte regions.
    string::memcpy(dst.as_mut_ptr(), src.as_ptr(), 6);
    assert_eq!(&dst[..6], b"abcdef");
    assert_eq!(string::memcmp(dst.as_ptr(), src.as_ptr(), 6), 0);

    // Overlapping forward move must behave like memmove.
    let mut buf = [0u8, 1, 2, 3, 4, 5, 6, 7];
    // Contract: source [0,6) and destination [2,8) overlap and are in bounds.
    let p = buf.as_mut_ptr();
    // SAFETY: p+2..p+8 is inside the same 8-byte buffer.
    let dst = unsafe { p.add(2) };
    string::memmove(dst, p, 6);
    assert_eq!(buf, [0, 1, 0, 1, 2, 3, 4, 5]);

    let hello = b"hello\0";
    // Contract: hello is NUL-terminated.
    assert_eq!(string::strlen(hello.as_ptr()), 5);
    // Contract: both strings are valid for 3 bytes.
    assert!(string::strncmp(b"abc\0".as_ptr(), b"abd\0".as_ptr(), 3) < 0);
    assert_eq!(string::strncmp(b"abc\0".as_ptr(), b"abc\0".as_ptr(), 3), 0);

    let mut name = [0xFFu8; 4];
    // Contract: name is 4 writable bytes; src is NUL-terminated.
    string::safestrcpy(name.as_mut_ptr(), b"abc\0".as_ptr(), 4);
    assert_eq!(name, *b"abc\0");
}

#[test]
fn elf_parse_ehdr_and_phdr() {
    let mut hdr = [0u8; elf::EHDR_SIZE];
    hdr[0..4].copy_from_slice(&elf::ELF_MAGIC.to_le_bytes());
    hdr[4..16].copy_from_slice(&[1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    hdr[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    hdr[18..20].copy_from_slice(&3u16.to_le_bytes()); // EM_386
    hdr[20..24].copy_from_slice(&1u32.to_le_bytes());
    hdr[24..28].copy_from_slice(&0x804_8000u32.to_le_bytes());
    hdr[28..32].copy_from_slice(&52u32.to_le_bytes());
    hdr[32..36].copy_from_slice(&0x9000u32.to_le_bytes());
    hdr[36..40].copy_from_slice(&0x1234u32.to_le_bytes());
    hdr[40..42].copy_from_slice(&52u16.to_le_bytes());
    hdr[42..44].copy_from_slice(&32u16.to_le_bytes());
    hdr[44..46].copy_from_slice(&2u16.to_le_bytes());
    hdr[46..48].copy_from_slice(&40u16.to_le_bytes());
    hdr[48..50].copy_from_slice(&3u16.to_le_bytes());
    hdr[50..52].copy_from_slice(&2u16.to_le_bytes());

    let e = elf::parse_ehdr(&hdr).expect("header parses");
    assert!(elf::valid_magic(&e));
    assert_eq!(e.magic, elf::ELF_MAGIC);
    assert_eq!(e.e_ident, [1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(e.type_, 2);
    assert_eq!(e.machine, 3);
    assert_eq!(e.entry, 0x804_8000);
    assert_eq!(e.phoff, 52);
    assert_eq!(e.phnum, 2);
    assert_eq!(e.ehsize, 52);
    assert_eq!(e.phentsize, 32);

    let mut ph = [0u8; elf::PHDR_SIZE];
    ph[0..4].copy_from_slice(&elf::ELF_PROG_LOAD.to_le_bytes());
    ph[4..8].copy_from_slice(&0x100u32.to_le_bytes());
    ph[8..12].copy_from_slice(&0x804_8000u32.to_le_bytes());
    ph[12..16].copy_from_slice(&0x804_8000u32.to_le_bytes());
    ph[16..20].copy_from_slice(&0x200u32.to_le_bytes());
    ph[20..24].copy_from_slice(&0x300u32.to_le_bytes());
    ph[24..28].copy_from_slice(&5u32.to_le_bytes());
    ph[28..32].copy_from_slice(&0x1000u32.to_le_bytes());
    let p = elf::parse_phdr(&ph).expect("phdr parses");
    assert_eq!(p.type_, elf::ELF_PROG_LOAD);
    assert_eq!(p.off, 0x100);
    assert_eq!(p.vaddr, 0x804_8000);
    assert_eq!(p.filesz, 0x200);
    assert_eq!(p.memsz, 0x300);
    assert_eq!(p.flags, 5);
    assert_eq!(p.align, 0x1000);

    assert!(elf::parse_ehdr(&hdr[..elf::EHDR_SIZE - 1]).is_none());
    assert!(elf::parse_phdr(&ph[..elf::PHDR_SIZE - 1]).is_none());
    let mut bad = e;
    bad.magic = 0;
    assert!(!elf::valid_magic(&bad));
}

// ---- M1R kernel core modules (ksyms / fixup_table / tf_layout) ------------

use super::fixup_table::{self, FixupEntry};
use super::ksyms::{self, Ksym, KSYM_CAP, KSYM_NAME};
use super::tf_layout::Trapframe;

fn sym(addr: u64, name: &str) -> Ksym {
    let mut n = [0u8; KSYM_NAME];
    n[..name.len()].copy_from_slice(name.as_bytes());
    Ksym { addr, name: n }
}

#[test]
fn ksyms_lookup_bounds_and_offsets() {
    let table = [
        sym(0x1000, "first"),
        sym(0x2000, "mid"),
        sym(0x3000, "last"),
    ];
    // Exact hits and in-between addresses resolve to the previous symbol.
    let (off, s) = ksyms::lookup(&table, 0x1000).expect("first");
    assert_eq!((off, s.name_str()), (0, "first"));
    let (off, s) = ksyms::lookup(&table, 0x2abc).expect("mid");
    assert_eq!((off, s.name_str()), (0xabc, "mid"));
    let (off, _) = ksyms::lookup(&table, 0x3fff).expect("last");
    assert_eq!(off, 0xfff);
    // Before the first symbol or empty table: no hit.
    assert!(ksyms::lookup(&table, 0xfff).is_none());
    assert!(ksyms::lookup(&[], 0x2000).is_none());
    // Capacity contract from the generator (fixed-size, convergent table).
    assert_eq!(KSYM_CAP, 2048);
    assert_eq!(KSYM_NAME, 56);
}

#[test]
fn ksyms_symbolize_format() {
    let table = [sym(0xffff_8000_0010_0000, "rust_main")];
    let mut out = [0u8; 96];
    let n = ksyms::symbolize_into(&table, 0xffff_8000_0010_0042, &mut out);
    assert_eq!(&out[..n], b"rust_main+0x42");
    let n = ksyms::symbolize_into(&table, 0x0, &mut out);
    // Unknown addresses print as ?0xADDR (hex, no sign issues).
    assert!(out[..n].starts_with(b"?0x"), "{:?}", &out[..n]);
}

#[test]
fn fixup_table_lookup_exact_rip() {
    let table = [
        FixupEntry {
            fault_rip: 0x1000,
            fixup_rip: 0x2000,
        },
        FixupEntry {
            fault_rip: 0x1800,
            fixup_rip: 0x2800,
        },
    ];
    assert_eq!(fixup_table::lookup(&table, 0x1000), Some(0x2000));
    assert_eq!(fixup_table::lookup(&table, 0x1800), Some(0x2800));
    // Only exact RIPs match (linear scan, no ranges).
    assert_eq!(fixup_table::lookup(&table, 0x1001), None);
    assert_eq!(fixup_table::lookup(&[], 0x1000), None);
}

#[test]
fn tf_layout_frozen_offsets() {
    // The asm twins in trapasm.S read these exact offsets.
    assert_eq!(core::mem::size_of::<Trapframe>(), 176);
    assert_eq!(core::mem::offset_of!(Trapframe, rdi), 0);
    assert_eq!(core::mem::offset_of!(Trapframe, r15), 112);
    assert_eq!(core::mem::offset_of!(Trapframe, vector), 120);
    assert_eq!(core::mem::offset_of!(Trapframe, err), 128);
    assert_eq!(core::mem::offset_of!(Trapframe, rip), 136);
    assert_eq!(core::mem::offset_of!(Trapframe, cs), 144);
    assert_eq!(core::mem::offset_of!(Trapframe, rflags), 152);
    assert_eq!(core::mem::offset_of!(Trapframe, rsp), 160);
    assert_eq!(core::mem::offset_of!(Trapframe, ss), 168);
    let mut tf = Trapframe {
        rdi: 0,
        rsi: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        r11: 0,
        rbx: 0,
        rbp: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        vector: 0x80,
        err: 0,
        rip: 0,
        cs: 0x33,
        rflags: 0x202,
        rsp: 0,
        ss: 0x2b,
    };
    assert!(tf.from_user());
    tf.cs = 0x08;
    assert!(!tf.from_user());
}
