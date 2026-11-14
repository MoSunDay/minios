//! Multiboot 1 boot protocol: header constants and info block layout.
//!
//! `MULTIBOOT_HEADER` lives in the `.multiboot` section which the linker
//! script places at the very start of the image, within the first 8 KiB as
//! the specification requires.

/// Magic written in the kernel header (header -> loader).
pub const HEADER_MAGIC: u32 = 0x1BADB002;

/// Magic passed in EAX by the loader (loader -> kernel).
pub const BOOT_LOADER_MAGIC: u32 = 0x2BADB002;

// Header flag bits:
//   bit 0 (0x01): loader must page-align boot modules.
//   bit 1 (0x02): loader must pass memory information (mem_lower/mem_upper).
//   bit 2 (0x04): loader must pass video mode information.
pub const FLAG_ALIGN_MODULES: u32 = 1 << 0;
pub const FLAG_MEMINFO: u32 = 1 << 1;
#[allow(dead_code)] // documented for completeness, not requested yet
pub const FLAG_VIDEO_MODE: u32 = 1 << 2;

pub const HEADER_FLAGS: u32 = FLAG_ALIGN_MODULES | FLAG_MEMINFO;

/// Checksum such that magic + flags + checksum == 0.
pub const HEADER_CHECKSUM: u32 = 0u32.wrapping_sub(HEADER_MAGIC + HEADER_FLAGS);

#[used]
#[link_section = ".multiboot"]
#[no_mangle]
pub static MULTIBOOT_HEADER: [u32; 3] = [HEADER_MAGIC, HEADER_FLAGS, HEADER_CHECKSUM];

/// Bit 0 of `MbiInfo::flags`: mem_lower/mem_upper are valid.
const MBI_FLAG_MEM: u32 = 1 << 0;

// Multiboot info `flags` bits (which fields are valid):
//   bit 0:  mem_lower / mem_upper
//   bit 1:  boot_device
//   bit 2:  cmdline
//   bit 3:  mods_count / mods_addr
//   bits 4-5: a.out symbol table (exclusive with bits 6-7)
//   bits 6-7: ELF section header table
//   bit 8:  mmap_length / mmap_addr (E820-style memory map)
//   bit 9:  drives_length / drives_addr
//   bit 10: config_table
//   bit 11: boot_loader_name
//   bit 12: apm_table
//   bit 13: VBE video information
//
// The full layout is declared now; parsing is added milestone by milestone.
#[repr(C)]
#[allow(dead_code)]
pub struct MbiInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub boot_device: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
    pub syms: [u32; 4],
    pub mmap_length: u32,
    pub mmap_addr: u32,
    pub drives_length: u32,
    pub drives_addr: u32,
    pub config_table: u32,
    pub boot_loader_name: u32,
    pub apm_table: u32,
    pub vbe_control_info: u32,
    pub vbe_mode_info: u32,
    pub vbe_mode: u16,
    pub vbe_interface_seg: u16,
    pub vbe_interface_off: u16,
    pub vbe_interface_len: u16,
}

/// Return `(mem_lower, mem_upper)` in KiB when the loader provided them.
pub fn mem_info(mbi_addr: u32) -> Option<(u32, u32)> {
    // SAFETY: the loader hands us a valid Multiboot info block in
    // identity-mapped low memory (paging is still off), so the physical
    // address can be dereferenced directly.
    let mbi = unsafe { &*(mbi_addr as usize as *const MbiInfo) };
    if mbi.flags & MBI_FLAG_MEM != 0 {
        Some((mbi.mem_lower, mbi.mem_upper))
    } else {
        None
    }
}
