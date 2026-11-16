pub fn out32(port: u16, value: u32) {
    // SAFETY: port I/O is the x86 hardware boundary used here for debug-exit.
    unsafe {
        core::arch::asm!("outl %eax, %dx", in("dx") port, in("eax") value, options(att_syntax, nomem, nostack))
    }
}

// First used by PIC/PIT programming in M3; keep the full port-width boundary here.
#[expect(dead_code)]
pub fn out16(port: u16, value: u16) {
    // SAFETY: port I/O is the x86 hardware boundary used here for serial and debug-exit.
    unsafe {
        core::arch::asm!("outw %ax, %dx", in("dx") port, in("ax") value, options(att_syntax, nomem, nostack))
    }
}

pub fn out8(port: u16, value: u8) {
    // SAFETY: port I/O is the x86 hardware boundary used here for serial output.
    unsafe {
        core::arch::asm!("outb %al, %dx", in("dx") port, in("al") value, options(att_syntax, nomem, nostack))
    }
}

pub fn in8(port: u16) -> u8 {
    let value: u8;
    // SAFETY: port I/O is the x86 hardware boundary; this read has no memory alias.
    unsafe {
        core::arch::asm!("inb %dx, %al", in("dx") port, out("al") value, options(att_syntax, nomem, nostack))
    }
    value
}
