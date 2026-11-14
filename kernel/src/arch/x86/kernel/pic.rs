//! pic: 8259A programmable interrupt controllers, ported from xv6
//! picirq.c. minios uses the legacy PIC (no IOAPIC) as its interrupt
//! controller, so EOI goes here instead of to the LAPIC.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::io::outb;

pub const IO_PIC1: u16 = 0x20; // master PIC base port
pub const IO_PIC2: u16 = 0xA0; // slave PIC base port
pub const IRQ_SLAVE: u32 = 2; // IRQ at which slave connects to master

/// Current IRQ mask (picirq.c:14). Initial mask has interrupt 2 enabled
/// (for the slave 8259A). Stored extended to 32 bits for atomics.
static IRQMASK: AtomicU32 = AtomicU32::new(0xFFFF & !(1 << IRQ_SLAVE));

/// picsetmask (picirq.c:17): write the mask to both controllers.
fn picsetmask(mask: u32) {
    IRQMASK.store(mask, Ordering::SeqCst);
    outb(IO_PIC1 + 1, mask as u8);
    outb(IO_PIC2 + 1, (mask >> 8) as u8);
}

/// picenable (picirq.c:26): enable (unmask) one IRQ line.
pub fn picenable(irq: u32) {
    picsetmask(IRQMASK.load(Ordering::SeqCst) & !(1 << irq));
}

/// u8-flavored alias expected by drivers (e.g. ide.rs).
pub fn picirq_enable(irq: u8) {
    picenable(irq as u32);
}

/// picinit (picirq.c:33): initialize the 8259A pair, cascade IRQ 2,
/// vectors T_IRQ0..T_IRQ0+15, x86 mode, AEOI. Leaves the initial mask.
pub fn picinit() {
    // mask all interrupts
    outb(IO_PIC1 + 1, 0xFF);
    outb(IO_PIC2 + 1, 0xFF);

    // Set up master (8259A-1)
    // ICW1: 0001g0hi — edge triggering, cascaded PICs, ICW4 required
    outb(IO_PIC1, 0x11);
    // ICW2: vector offset
    outb(IO_PIC1 + 1, crate::arch::x86::kernel::T_IRQ0 as u8);
    // ICW3: (master) bit mask of IR lines wired to slaves
    outb(IO_PIC1 + 1, (1 << IRQ_SLAVE) as u8);
    // ICW4: 000nbmap — x86 mode, automatic EOI
    outb(IO_PIC1 + 1, 0x3);

    // Set up slave (8259A-2)
    outb(IO_PIC2, 0x11); // ICW1
    outb(IO_PIC2 + 1, crate::arch::x86::kernel::T_IRQ0 as u8 + 8); // ICW2
    outb(IO_PIC2 + 1, IRQ_SLAVE as u8); // ICW3
                                        // NB Automatic EOI mode doesn't tend to work on the slave.
                                        // Linux source code says it's "to be investigated".
    outb(IO_PIC2 + 1, 0x3); // ICW4

    // OCW3: 0ef01prs — clear specific mask, read IRR by default
    outb(IO_PIC1, 0x68);
    outb(IO_PIC1, 0x0a);
    outb(IO_PIC2, 0x68);
    outb(IO_PIC2, 0x0a);

    let mask = IRQMASK.load(Ordering::SeqCst);
    if mask != 0xFFFF {
        picsetmask(mask);
    }
}

/// End-of-interrupt for the PIC (replaces xv6 lapiceoi, lapic.c:193).
/// IRQs >= 8 arrive via the slave, which must also be acknowledged.
pub fn eoi(irq: u32) {
    if irq >= 8 {
        outb(IO_PIC2, 0x20); // specific EOI on slave
    }
    outb(IO_PIC1, 0x20); // specific EOI on master
}

/// Spurious-IRQ check for IRQ 7/15 (PIC nuance xv6 only needed for the
/// IOAPIC/LAPIC path): a spurious IRQ7 shows nothing in the master ISR.
/// Returns true when the interrupt was spurious (caller must NOT EOI).
pub fn spurious(irq: u32) -> bool {
    use crate::io::inb;
    if irq != 7 && irq != 15 {
        return false;
    }
    let port = if irq == 7 { IO_PIC1 } else { IO_PIC2 };
    outb(port, 0x0b); // OCW3: read ISR on next read
    let isr = inb(port); // SAFETY: plain port read
    outb(port, 0x0a); // OCW3: back to reading IRR
    isr & (1 << (irq & 7)) == 0
}
