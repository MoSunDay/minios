//! mm: boot memory map, buddy frames, direct map, SLUB, vmalloc and the
//! 4-level page tables (M1R surface + M2 physical-page foundation).

pub mod bootmem;
pub mod buddy;
pub mod directmap;
pub mod frame;
pub mod mmap;
pub mod pgtable;
pub mod slub;
pub mod uaccess;
pub mod vm;
pub mod vma;
pub mod vmalloc;
