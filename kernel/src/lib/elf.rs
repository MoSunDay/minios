//! elf.rs: ELF32 image-header parsing, extracted from xv6 exec.c/elf.h.
//!
//! `repr(C)` header structs keep xv6's on-disk field layout. The parsers are
//! pure and bounds-checked; `exec` reads header bytes with readi and then
//! calls them, so no zeroed header or raw struct cast is needed.

/// elf.h: must equal 0x464C457F ("\x7FELF" little endian).
pub const ELF_MAGIC: u32 = 0x464C_457F;
/// elf.h: Proghdr type == loadable segment.
pub const ELF_PROG_LOAD: u32 = 1;
/// Size of ELF32 ehdr on disk.
pub const EHDR_SIZE: usize = 52;
/// Size of ELF32 phdr on disk.
pub const PHDR_SIZE: usize = 32;

/// elf.h `struct elfhdr` (ELF32 image header), repr(C) identical layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf32_Ehdr {
    pub magic: u32, // must equal ELF_MAGIC
    pub e_ident: [u8; 12],
    pub type_: u16,
    pub machine: u16,
    pub version: u32,
    pub entry: u32, // where the program starts
    pub phoff: u32, // program header table offset
    pub shoff: u32,
    pub flags: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16, // number of program headers
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

/// elf.h `struct proghdr` (program-section header).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf32_Phdr {
    pub type_: u32,
    pub off: u32,   // Offset of segment in file
    pub vaddr: u32, // Virtual address in memory
    pub paddr: u32,
    pub filesz: u32, // Size of segment in file
    pub memsz: u32,  // Size of segment in memory (bigger than filesz -> bss)
    pub flags: u32,
    pub align: u32,
}

#[inline]
fn u16_at(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

#[inline]
fn u32_at(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// Parse an ELF32 header from at least `EHDR_SIZE` bytes.
pub fn parse_ehdr(bytes: &[u8]) -> Option<Elf32_Ehdr> {
    if bytes.len() < EHDR_SIZE {
        return None;
    }
    let mut e_ident = [0u8; 12];
    e_ident.copy_from_slice(&bytes[4..16]);
    Some(Elf32_Ehdr {
        magic: u32_at(bytes, 0),
        e_ident,
        type_: u16_at(bytes, 16),
        machine: u16_at(bytes, 18),
        version: u32_at(bytes, 20),
        entry: u32_at(bytes, 24),
        phoff: u32_at(bytes, 28),
        shoff: u32_at(bytes, 32),
        flags: u32_at(bytes, 36),
        ehsize: u16_at(bytes, 40),
        phentsize: u16_at(bytes, 42),
        phnum: u16_at(bytes, 44),
        shentsize: u16_at(bytes, 46),
        shnum: u16_at(bytes, 48),
        shstrndx: u16_at(bytes, 50),
    })
}

/// Parse an ELF32 program header from at least `PHDR_SIZE` bytes.
pub fn parse_phdr(bytes: &[u8]) -> Option<Elf32_Phdr> {
    if bytes.len() < PHDR_SIZE {
        return None;
    }
    Some(Elf32_Phdr {
        type_: u32_at(bytes, 0),
        off: u32_at(bytes, 4),
        vaddr: u32_at(bytes, 8),
        paddr: u32_at(bytes, 12),
        filesz: u32_at(bytes, 16),
        memsz: u32_at(bytes, 20),
        flags: u32_at(bytes, 24),
        align: u32_at(bytes, 28),
    })
}

/// xv6 exec only checked the first word; keep that contract explicit.
#[inline]
pub fn valid_magic(ehdr: &Elf32_Ehdr) -> bool {
    ehdr.magic == ELF_MAGIC
}
