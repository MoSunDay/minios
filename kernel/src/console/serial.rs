//! COM1 (0x3F8) 16550 UART polling driver.

use crate::io::{inb, outb};

const COM1: u16 = 0x3F8;

// Register offsets from the COM1 base port.
const DATA: u16 = 0; // receive / transmit buffer (DLAB = 0)
const INTERRUPT_ENABLE: u16 = 1; // also divisor high byte when DLAB = 1
const FIFO_CONTROL: u16 = 2;
const LINE_CONTROL: u16 = 3;
const MODEM_CONTROL: u16 = 4;
const LINE_STATUS: u16 = 5;

const LSR_THR_EMPTY: u8 = 1 << 5;

/// Program COM1: 115200 baud, 8N1, FIFOs on, DTR and RTS asserted.
pub fn init() {
    outb(COM1 + INTERRUPT_ENABLE, 0x00); // polled mode, no UART interrupts
    outb(COM1 + LINE_CONTROL, 0x80); // DLAB = 1 to program the divisor
    outb(COM1 + DATA, 0x01); // divisor low byte: 115200 baud
    outb(COM1 + INTERRUPT_ENABLE, 0x00); // divisor high byte
    outb(COM1 + LINE_CONTROL, 0x03); // DLAB = 0, 8 data bits, no parity
    outb(COM1 + FIFO_CONTROL, 0xC7); // enable and clear FIFOs
    outb(COM1 + MODEM_CONTROL, 0x0B); // DTR | RTS | OUT2
}

/// Send one byte, waiting for the transmit holding register to drain.
pub fn send_byte(byte: u8) {
    while inb(COM1 + LINE_STATUS) & LSR_THR_EMPTY == 0 {
        core::hint::spin_loop();
    }
    outb(COM1 + DATA, byte);
}
