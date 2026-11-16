//! Machine-specific layer for the x86_64 Core target.
pub mod x86_64;
pub use x86_64::{console, cpu, io, DATA_SENTINEL, KERNEL_VMA, PAGE_NX, PAGE_PRESENT, PAGE_WRITE};
