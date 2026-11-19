use super::io::{in8, out8};
use core::sync::atomic::{AtomicU16, Ordering};

const COM1: u16 = 0x3f8;
const VGA_PHYS: usize = 0x000b_8000;
// Boot.S pre-maps phys 0..256MiB at PML4[510]; the high half maps the
// kernel image (phys base 0x500000), so 0xffffffff800b8000 would alias
// kernel .data, not VGA. Route through the direct map instead.
const VGA_VMA: usize = 0xffff_ff00_0000_0000 + VGA_PHYS;
static VGA_CURSOR: AtomicU16 = AtomicU16::new(0);

pub fn init_serial() {
    out8(COM1 + 1, 0x00);
    out8(COM1 + 3, 0x80);
    out8(COM1, 0x01);
    out8(COM1 + 1, 0x00);
    out8(COM1 + 3, 0x03);
    out8(COM1 + 2, 0xc7);
    out8(COM1 + 4, 0x0b);
}

pub fn write_byte(byte: u8) {
    if byte == b'\n' {
        send_serial(b'\r');
    }
    send_serial(byte);
    write_vga(byte);
}

fn send_serial(byte: u8) {
    while in8(COM1 + 5) & 0x20 == 0 {
        core::hint::spin_loop();
    }
    out8(COM1, byte);
}

fn write_vga(byte: u8) {
    let old_cursor = VGA_CURSOR.load(Ordering::Relaxed);
    let mut cursor = old_cursor;
    if byte == b'\n' {
        cursor = (cursor / 80 + 1) * 80;
    } else if byte == b'\r' {
        cursor -= cursor % 80;
    } else {
        // SAFETY: VGA text memory is pre-mapped by the boot page tables
        // (direct-map PML4[510], present before rust_main in every mode).
        unsafe {
            let cell = (VGA_VMA + cursor as usize * 2) as *mut u16;
            core::ptr::write_volatile(cell, 0x0f00 | u16::from(byte));
        }
        cursor += 1;
    }
    if cursor >= 80 * 25 {
        cursor = 0;
    }
    VGA_CURSOR.store(cursor, Ordering::Relaxed);
}

pub fn write_str(text: &str) {
    for byte in text.as_bytes() {
        write_byte(*byte);
    }
}

pub fn write_hex(value: usize) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    write_str("0x");
    for shift in (0..usize::BITS).step_by(4).rev() {
        let digit = (value >> shift) & 0xf;
        write_byte(HEX[digit]);
    }
}
