//! Host-side unit tests for portable kernel code.
//!
//! The modules below are the real `kernel/src/lib` files, included by path so
//! there is no test-only copy to drift. `cfg(test)` is active for the whole
//! crate here, which also disables the kernel's `#[no_mangle]` string symbols
//! so they cannot collide with the host libc.

#[path = "../../kernel/src/lib/bitmap.rs"]
pub mod bitmap;
#[path = "../../kernel/src/lib/elf.rs"]
pub mod elf;
#[path = "../../kernel/src/lib/string.rs"]
pub mod string;

#[cfg(test)]
mod tests;
