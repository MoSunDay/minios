//! Kernel panic handler: report over VGA and serial, then halt forever.

use core::panic::PanicInfo;

use crate::console;

/// Abort-only builds never unwind, but the precompiled libcore objects
/// still carry a reference to the personality routine; satisfy it with a
/// stub that must never be called.
#[no_mangle]
extern "C" fn rust_eh_personality() {
    unreachable!("unwinding is disabled in minios")
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::write_str("\n=== kernel panic ===\n");
    if let Some(location) = info.location() {
        console::_print(format_args!(
            "at {}:{}:{}\n",
            location.file(),
            location.line(),
            location.column()
        ));
    }
    console::_print(format_args!("{}\n", info.message()));
    console::write_str("system halted\n");
    crate::halt()
}
