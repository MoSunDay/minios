pub mod boot;
pub mod console;
pub mod cpu;
pub mod io;

pub const KERNEL_VMA: usize = 0xffff_ffff_8000_0000;
pub const PAGE_PRESENT: u64 = 1;
pub const PAGE_WRITE: u64 = 1 << 1;
pub const PAGE_NX: u64 = 1 << 63;

#[no_mangle]
pub static RODATA_SENTINEL: u8 = 0x5a;

#[no_mangle]
pub static mut DATA_SENTINEL: u8 = 0xa5;
