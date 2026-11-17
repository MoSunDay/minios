//! 8259A PIC: remap to 0x20..0x2F, mask/unmask, EOI (UP only).

use crate::arch::x86_64::io;

pub const PIC1_CMD: u16 = 0x20;
pub const PIC1_DATA: u16 = 0x21;
pub const PIC2_CMD: u16 = 0xA0;
pub const PIC2_DATA: u16 = 0xA1;

/// Remap both PICs so IRQ0..15 appear as vectors 0x20..0x2F.
pub fn init() {
    io::out8(PIC1_CMD, 0x11); io::io_wait();
    io::out8(PIC2_CMD, 0x11); io::io_wait();
    io::out8(PIC1_DATA, 0x20); io::io_wait(); // ICW2: master base 0x20
    io::out8(PIC2_DATA, 0x28); io::io_wait(); // ICW2: slave base 0x28
    io::out8(PIC1_DATA, 0x04); io::io_wait(); // ICW3: slave on IRQ2
    io::out8(PIC2_DATA, 0x02); io::io_wait();
    io::out8(PIC1_DATA, 0x01); io::io_wait(); // ICW4: 8086 mode
    io::out8(PIC2_DATA, 0x01); io::io_wait();
    // Mask everything; unmask per line as drivers come up.
    io::out8(PIC1_DATA, 0xFF);
    io::out8(PIC2_DATA, 0xFF);
}

/// Unmask one IRQ line (0..15).
pub fn unmask(irq: u8) {
    // SAFETY: mask register access.
    if irq < 8 {
        let m = io::in8(PIC1_DATA) & !(1 << irq);
        io::out8(PIC1_DATA, m);
    } else {
        let m = io::in8(PIC2_DATA) & !(1 << (irq - 8));
        io::out8(PIC2_DATA, m);
    }
}

/// Mask one IRQ line (0..15).
#[allow(dead_code)]
pub fn mask(irq: u8) {
    // SAFETY: mask register access.
    if irq < 8 {
        let m = io::in8(PIC1_DATA) | (1 << irq);
        io::out8(PIC1_DATA, m);
    } else {
        let m = io::in8(PIC2_DATA) | (1 << (irq - 8));
        io::out8(PIC2_DATA, m);
    }
}

/// End-of-interrupt for vector v (IRQ vector 0x20..0x2F).
pub fn eoi(v: u8) {
    // SAFETY: fixed ISA ports.
    if v >= 0x28 {
        io::out8(PIC2_CMD, 0x20);
    }
    io::out8(PIC1_CMD, 0x20);
}
