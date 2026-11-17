//! lib: freestanding helpers (string.c/elf.h/bitmap counterparts).
//!
//! elf/bitmap are compiled only by host-tests (#[path] includes); the
//! kernel has its own ELF64 loader (proc/exec.rs). printk.rs is legacy
//! xv6 formatting, unreferenced since the M0R facade (D2 deletes it).

pub mod string;
pub mod sync;
