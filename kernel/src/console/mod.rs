//! Kernel console: VGA text screen and COM1 serial written in lockstep.

pub mod serial;
pub mod vga;

use core::fmt;

/// Send a string to every console sink.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        vga::put_byte(byte);
        serial::send_byte(byte);
    }
}

/// Formatting sink backing the print macros.
struct Sinks;

impl fmt::Write for Sinks {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

/// Hidden implementation detail of the `print!`/`println!` macros.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write as _;
    Sinks.write_fmt(args).ok();
}

/// Print text to the kernel consoles.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::console::_print(format_args!($($arg)*));
    }};
}

/// Print text plus a trailing newline to the kernel consoles.
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {{
        $crate::console::_print(format_args!($($arg)*));
        $crate::console::write_str("\n");
    }};
}

/// Bring both sinks up and print the boot banner.
pub fn early_banner() {
    serial::init();
    vga::clear();
    vga::set_color(vga::COLOR_LIGHT_CYAN, vga::COLOR_BLACK);
    println!("minios v0.2.0 (x86, rust)");
    vga::set_color(vga::COLOR_LIGHT_GREY, vga::COLOR_BLACK);
}
