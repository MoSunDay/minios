//! Kernel console facade: x86_64 serial console (COM1), the only sink.
//!
//! The legacy VGA/serial pair is retired with the i386 tree; all kernel
//! printing funnels through the serial driver under arch/x86_64.
pub use crate::arch::x86_64::console::{init_serial, write_byte, write_hex, write_str};
