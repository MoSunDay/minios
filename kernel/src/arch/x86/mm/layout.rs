//! Memory layout constants (ported from xv6 memlayout.h).
//! Kernel is linked at VMA 0x80100000, loaded at physical 0x100000.

pub const KERNBASE: usize = 0x8000_0000; // 2 GB
pub const PHYSTOP: usize = 0x0E00_0000; // 224 MB: top physical memory
pub const DEVSPACE: usize = 0xFE00_0000; // device mappings (unused in M1)

extern "C" {
    static _kernel_end: u8;
    static _kernel_start: u8;
}

/// Virtual address just past the kernel image.
pub fn kernel_end() -> usize {
    // SAFETY: linker symbol, read as integer only.
    unsafe { &_kernel_end as *const u8 as usize }
}

pub fn kernel_start() -> usize {
    // SAFETY: linker symbol, read as integer only.
    unsafe { &_kernel_start as *const u8 as usize }
}

pub const fn v2p(va: usize) -> usize {
    va - KERNBASE
}

pub const fn p2v(pa: usize) -> usize {
    pa + KERNBASE
}
