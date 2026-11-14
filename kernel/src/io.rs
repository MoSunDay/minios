//! x86 port I/O and halt primitives.
//!
//! These `unsafe asm` wrappers are the only instruction-set boundary in
//! minios; every driver goes through them.

#![allow(dead_code)] // wider accessors are kept ready for the M1 drivers

use core::arch::asm;

/// Read one byte from an I/O port.
pub fn inb(port: u16) -> u8 {
    let value;
    // SAFETY: `in` only reads the named port; the caller must pass a port
    // that is safe to read on this platform.
    unsafe {
        asm!("inb %dx, %al", in("dx") port, out("al") value, options(nostack, preserves_flags, att_syntax))
    };
    value
}

/// Write one byte to an I/O port.
pub fn outb(port: u16, value: u8) {
    // SAFETY: `out` only writes the named port; the caller must pass a port
    // that is safe to write on this platform.
    unsafe {
        asm!("outb %al, %dx", in("al") value, in("dx") port, options(nostack, preserves_flags, att_syntax))
    };
}

/// Read one word (16-bit) from an I/O port.
pub fn inw(port: u16) -> u16 {
    let value;
    // SAFETY: same contract as `inb`, with a 16-bit operand.
    unsafe {
        asm!("inw %dx, %ax", in("dx") port, out("ax") value, options(nostack, preserves_flags, att_syntax))
    };
    value
}

/// Write one word (16-bit) to an I/O port.
pub fn outw(port: u16, value: u16) {
    // SAFETY: same contract as `outb`, with a 16-bit operand.
    unsafe {
        asm!("outw %ax, %dx", in("ax") value, in("dx") port, options(nostack, preserves_flags, att_syntax))
    };
}

/// Read one double word (32-bit) from an I/O port.
pub fn inl(port: u16) -> u32 {
    let value;
    // SAFETY: same contract as `inb`, with a 32-bit operand.
    unsafe {
        asm!("inl %dx, %eax", in("dx") port, out("eax") value, options(nostack, preserves_flags, att_syntax))
    };
    value
}

/// Write one double word (32-bit) to an I/O port.
pub fn outl(port: u16, value: u32) {
    // SAFETY: same contract as `outb`, with a 32-bit operand.
    unsafe {
        asm!("outl %eax, %dx", in("eax") value, in("dx") port, options(nostack, preserves_flags, att_syntax))
    };
}

/// Halt until the next interrupt arrives.
pub fn hlt() {
    // SAFETY: `hlt` parks the CPU; with interrupts disabled it never wakes,
    // which is exactly the idle behaviour we want.
    unsafe { asm!("hlt", options(nostack, preserves_flags, att_syntax)) };
}
