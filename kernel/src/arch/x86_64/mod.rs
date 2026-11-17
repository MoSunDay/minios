pub mod boot;
pub mod console;
pub mod cpu;
pub mod io;

pub const KERNEL_VMA: usize = 0xffff_ffff_8000_0000;
pub const PAGE_PRESENT: u64 = 1;
pub const PAGE_WRITE: u64 = 1 << 1;
pub const PAGE_NX: u64 = 1 << 63;
/// Direct map of all physical RAM: PML4[509], 256 MiB of 2 MiB pages.
pub const DIRECT_VMA: usize = 0xffff_ff00_0000_0000;
/// Top of direct-mapped physical RAM (kernel image must fit below it).
pub const PHYS_TOP: usize = 0x1000_0000;

#[no_mangle]
pub static RODATA_SENTINEL: u8 = 0x5a;

#[no_mangle]
pub static mut DATA_SENTINEL: u8 = 0xa5;
