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

/// Read `count` u32 words from an I/O port into memory (xv6 insl).
///
/// LLVM's i686 backend reserves %esi, so the `rep insl` form cannot be
/// expressed as inline asm here; a loop of `inl` is functionally identical
/// (PIO transfers are only 128 dwords per sector).
pub fn insl(port: u16, dst: *mut u32, count: usize) {
    let mut i = 0;
    while i < count {
        // SAFETY: the caller guarantees `dst` covers `count` writable words.
        unsafe { dst.add(i).write_volatile(inl(port)) };
        i += 1;
    }
}

/// Write `count` u32 words from memory to an I/O port (xv6 outsl).
pub fn outsl(port: u16, src: *const u32, count: usize) {
    let mut i = 0;
    while i < count {
        // SAFETY: the caller guarantees `src` covers `count` readable words.
        let word = unsafe { src.add(i).read_volatile() };
        outl(port, word);
        i += 1;
    }
}

/// Halt until the next interrupt arrives.
pub fn hlt() {
    // SAFETY: `hlt` parks the CPU; with interrupts disabled it never wakes,
    // which is exactly the idle behaviour we want.
    unsafe { asm!("hlt", options(nostack, preserves_flags, att_syntax)) };
}
