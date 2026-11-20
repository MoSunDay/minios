//! Host-side unit tests for portable kernel code.
//!
//! The modules below are the real `kernel/src/lib` files, included by path so
//! there is no test-only copy to drift. `cfg(test)` is active for the whole
//! crate here, which also disables the kernel's `#[no_mangle]` string symbols
//! so they cannot collide with the host libc.

#[path = "../../kernel/src/lib/bitmap.rs"]
pub mod bitmap;
#[path = "../../kernel/src/mm/bootmem.rs"]
pub mod bootmem;
#[path = "../../kernel/src/mm/buddy.rs"]
pub mod buddy;
#[path = "../../kernel/src/proc/cfs.rs"]
pub mod cfs;
#[path = "../../kernel/src/lib/elf.rs"]
pub mod elf;
#[path = "../../kernel/src/kernel/fixup_table.rs"]
pub mod fixup_table;
#[path = "../../kernel/src/kernel/ksyms.rs"]
pub mod ksyms;
#[path = "../../kernel/src/proc/rbtree.rs"]
pub mod rbtree;
#[path = "../../kernel/src/proc/rt.rs"]
pub mod rt;
#[path = "../../kernel/src/mm/slub.rs"]
pub mod slub;
#[path = "../../kernel/src/lib/string.rs"]
pub mod string;
#[path = "../../kernel/src/kernel/sysarg.rs"]
pub mod sysarg;
#[path = "../../kernel/src/kernel/tf_layout.rs"]
pub mod tf_layout;
#[path = "../../kernel/src/mm/vma.rs"]
pub mod vma;

#[cfg(test)]
mod m2tests;
#[cfg(test)]
mod m3tests;
#[cfg(test)]
mod rbtreetests;
#[cfg(test)]
mod rttests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vmatests;
