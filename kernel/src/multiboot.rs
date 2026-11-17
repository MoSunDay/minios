//! Multiboot 1 boot protocol: info block layout parsing.
//!
//! The ONE header lives in boot.S (`.multiboot_header`, frozen flags 0x3);
//! this module only parses the loader-provided info block.

/// Magic passed in EAX by the loader (loader -> kernel).
#[allow(dead_code)] // m0r compares the raw literal for byte-identity
pub const BOOT_LOADER_MAGIC: u32 = 0x2BADB002;

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

// ---- M1R: cmdline + memory-map parsing ----------------------------------

const MBI_FLAG_CMDLINE: u32 = 1 << 2;
const MBI_FLAG_MMAP: u32 = 1 << 8;

const MMAP_AVAILABLE: u32 = 1;

#[repr(C)]
#[allow(dead_code)]
pub struct MmapEntry {
    pub size: u32,
    pub addr_low: u32,
    pub addr_high: u32,
    pub len_low: u32,
    pub len_high: u32,
    pub mtype: u32,
}

/// Multiboot MBI flag for "cmdline present". GRUB passes a cmdline whenever
/// one is written on the `multiboot` module line (no header-flag request is
/// needed or possible for cmdlines in Multiboot1); the boot header flags
/// stay at the frozen M0R value 0x3 (align + meminfo).

/// Boot command line, or "" when absent. Valid until the next call; the
/// string lives in the loader's low-memory block, which stays mapped.
pub fn cmdline(mbi_phys: usize) -> &'static str {
    // SAFETY: the loader-provided MBI is identity mapped and immutable.
    let mbi = unsafe { &*(mbi_phys as *const MbiInfo) };
    if mbi.flags & MBI_FLAG_CMDLINE == 0 {
        return "";
    }
    let p = mbi.cmdline as usize as *const u8;
    let mut len = 0usize;
    // SAFETY: Multiboot cmdlines are NUL-terminated C strings.
    unsafe {
        while len < 256 && *p.add(len) != 0 {
            len += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p, len)).unwrap_or("")
    }
}

/// Call `f(addr, len)` for every available RAM range from the loader's map.
pub fn for_each_available(mbi_phys: usize, mut f: impl FnMut(u64, u64)) {
    // SAFETY: the loader-provided MBI and mmap array are identity mapped.
    unsafe {
        let mbi = &*(mbi_phys as *const MbiInfo);
        if mbi.flags & MBI_FLAG_MMAP == 0 {
            return;
        }
        let mut off = 0usize;
        while off < mbi.mmap_length as usize {
            let entry = &*((mbi.mmap_addr as usize + off) as *const MmapEntry);
            off += entry.size as usize + 4;
            if entry.mtype == MMAP_AVAILABLE {
                let base = (entry.addr_low as u64) | ((entry.addr_high as u64) << 32);
                let len = (entry.len_low as u64) | ((entry.len_high as u64) << 32);
                f(base, len);
            }
        }
    }
}
