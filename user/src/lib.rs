//! minios Rust user runtime (M1R): no_std, no libc, custom `_start`,
//! inline-asm syscalls, brk allocator (plans/linux26/04 §3). Every binary
//! defines `extern "C" fn main(argc, argv) -> i32`.

#![no_std]

use core::panic::PanicInfo;
use io::STDERR;

pub mod alloc;
pub mod args;
pub mod io;
pub mod mem;
pub mod rt;
pub mod sys;
pub mod utest;

/// The M1R user heap (04 §3): brk-backed, allocation via the Rust `alloc`
/// crate surface (`extern crate alloc` in the bins); free is a no-op.
#[global_allocator]
static HEAP: alloc::BrkAlloc = alloc::BRK_ALLOC;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    // The word "panic" in serial output intentionally fails the suite
    // verdict; a user panic is a real failure, not noise.
    let _ = io::eprint_str("m1r-user panic at ");
    if let Some(loc) = info.location() {
        let _ = io::eprint_str(loc.file());
        let _ = io::eprint_dec(STDERR, loc.line() as u64);
    } else {
        let _ = io::eprint_str("unknown");
    }
    let _ = io::eprint_str("\n");
    sys::exit(0xff)
}
