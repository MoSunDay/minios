//! VGA text mode driver: 80x25 cells at physical 0xB8000.
//!
//! Each cell is a u16: low byte = character code, high byte = attribute
//! (low nibble foreground, high nibble background).

#![allow(dead_code)] // full palette kept ready for upcoming console work

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::io::outb;

const BUFFER_ADDR: usize = 0xB8000;
pub const COLS: usize = 80;
pub const ROWS: usize = 25;

// CRT controller registers for the hardware cursor.
const CRTC_INDEX: u16 = 0x3D4;
const CRTC_DATA: u16 = 0x3D5;
const CRTC_CURSOR_HIGH: u8 = 0x0E;
const CRTC_CURSOR_LOW: u8 = 0x0F;

// Standard VGA text colors (IRGB).
pub const COLOR_BLACK: u8 = 0x0;
pub const COLOR_BLUE: u8 = 0x1;
pub const COLOR_GREEN: u8 = 0x2;
pub const COLOR_CYAN: u8 = 0x3;
pub const COLOR_RED: u8 = 0x4;
pub const COLOR_MAGENTA: u8 = 0x5;
pub const COLOR_BROWN: u8 = 0x6;
pub const COLOR_LIGHT_GREY: u8 = 0x7;
pub const COLOR_DARK_GREY: u8 = 0x8;
pub const COLOR_LIGHT_BLUE: u8 = 0x9;
pub const COLOR_LIGHT_GREEN: u8 = 0xA;
pub const COLOR_LIGHT_CYAN: u8 = 0xB;
pub const COLOR_LIGHT_RED: u8 = 0xC;
pub const COLOR_LIGHT_MAGENTA: u8 = 0xD;
pub const COLOR_YELLOW: u8 = 0xE;
pub const COLOR_WHITE: u8 = 0xF;

static ROW: AtomicUsize = AtomicUsize::new(0);
static COL: AtomicUsize = AtomicUsize::new(0);
static ATTR: AtomicU8 = AtomicU8::new(COLOR_LIGHT_GREY);

/// Encode one character cell.
fn cell(byte: u8, attr: u8) -> u16 {
    u16::from(byte) | (u16::from(attr) << 8)
}

/// Address of the cell at `index` in linear VGA memory.
fn cell_ptr(index: usize) -> *mut u16 {
    (BUFFER_ADDR + index * 2) as *mut u16
}

/// Read one cell from VGA memory.
fn load_cell(index: usize) -> u16 {
    // SAFETY: VGA text memory 0xB8000..0xB8FA0 is always mapped on PC
    // hardware, and callers keep `index` within COLS * ROWS.
    unsafe { read_volatile(cell_ptr(index)) }
}

/// Write one cell to VGA memory.
fn store_cell(index: usize, value: u16) {
    // SAFETY: same mapping and bounds argument as `load_cell`.
    unsafe { write_volatile(cell_ptr(index), value) }
}

/// Select the foreground/background attribute for new text.
pub fn set_color(foreground: u8, background: u8) {
    ATTR.store(foreground | (background << 4), Ordering::Relaxed);
}

/// Clear the screen and home the cursor.
pub fn clear() {
    let blank = cell(b' ', ATTR.load(Ordering::Relaxed));
    for index in 0..COLS * ROWS {
        store_cell(index, blank);
    }
    ROW.store(0, Ordering::Relaxed);
    COL.store(0, Ordering::Relaxed);
    sync_cursor();
}

/// Write a single byte, handling newline and carriage return.
pub fn put_byte(byte: u8) {
    match byte {
        b'\n' => newline(),
        b'\r' => {
            COL.store(0, Ordering::Relaxed);
            sync_cursor();
        }
        _ => {
            let row = ROW.load(Ordering::Relaxed);
            let col = COL.load(Ordering::Relaxed);
            store_cell(row * COLS + col, cell(byte, ATTR.load(Ordering::Relaxed)));
            if col + 1 < COLS {
                COL.store(col + 1, Ordering::Relaxed);
            } else {
                newline();
            }
            sync_cursor();
        }
    }
}

/// Move to the start of the next line, scrolling when full.
fn newline() {
    COL.store(0, Ordering::Relaxed);
    let row = ROW.load(Ordering::Relaxed);
    if row + 1 < ROWS {
        ROW.store(row + 1, Ordering::Relaxed);
    } else {
        scroll();
    }
}

/// Shift every line up by one and blank the last line.
fn scroll() {
    let last = (ROWS - 1) * COLS;
    for index in 0..last {
        store_cell(index, load_cell(index + COLS));
    }
    let blank = cell(b' ', ATTR.load(Ordering::Relaxed));
    for index in last..COLS * ROWS {
        store_cell(index, blank);
    }
}

/// Point the blinking hardware cursor at (ROW, COL).
fn sync_cursor() {
    let pos = ROW.load(Ordering::Relaxed) * COLS + COL.load(Ordering::Relaxed);
    outb(CRTC_INDEX, CRTC_CURSOR_LOW);
    outb(CRTC_DATA, pos as u8);
    outb(CRTC_INDEX, CRTC_CURSOR_HIGH);
    outb(CRTC_DATA, (pos >> 8) as u8);
}
