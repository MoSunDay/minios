//! bitmap: bounds-checked bit helpers shared by allocators and fs code.
//!
//! Pure functions over byte slices: no globals, no I/O. Bit 0 is the least
//! significant bit of byte 0 (little-endian bit order, matching xv6's on-disk
//! free bitmap and Linux's `test_bit` on x86).

#![allow(dead_code)] // M2 buddy/slab and future fs allocators consume these helpers.

/// Number of bits in one bitmap word (byte).
pub const BITS_PER_WORD: usize = 8;

/// Test bit `bit`, or false when out of range.
#[inline]
pub fn test(bitmap: &[u8], bit: usize) -> bool {
    match bitmap.get(bit / BITS_PER_WORD) {
        Some(byte) => byte & (1 << (bit % BITS_PER_WORD)) != 0,
        None => false,
    }
}

/// Set bit `bit`; out-of-range bits are ignored.
#[inline]
pub fn set(bitmap: &mut [u8], bit: usize) {
    if let Some(byte) = bitmap.get_mut(bit / BITS_PER_WORD) {
        *byte |= 1 << (bit % BITS_PER_WORD);
    }
}

/// Clear bit `bit`; out-of-range bits are ignored.
#[inline]
pub fn clear(bitmap: &mut [u8], bit: usize) {
    if let Some(byte) = bitmap.get_mut(bit / BITS_PER_WORD) {
        *byte &= !(1 << (bit % BITS_PER_WORD));
    }
}

/// Index of the first zero bit, scanning low to high, or None if all set.
pub fn find_first_zero(bitmap: &[u8]) -> Option<usize> {
    for (i, &byte) in bitmap.iter().enumerate() {
        if byte != 0xFF {
            return Some(i * BITS_PER_WORD + (!byte).trailing_zeros() as usize);
        }
    }
    None
}

/// Index of the first set bit, scanning low to high, or None if all clear.
pub fn find_first_set(bitmap: &[u8]) -> Option<usize> {
    for (i, &byte) in bitmap.iter().enumerate() {
        if byte != 0 {
            return Some(i * BITS_PER_WORD + byte.trailing_zeros() as usize);
        }
    }
    None
}

/// Count clear bits in the first `bits` positions.
pub fn count_zeros(bitmap: &[u8], bits: usize) -> usize {
    let full = bits / BITS_PER_WORD;
    let tail = bits % BITS_PER_WORD;
    let mut zeros = 0;
    for &byte in bitmap.iter().take(full) {
        zeros += byte.count_zeros() as usize;
    }
    if tail != 0 {
        if let Some(&byte) = bitmap.get(full) {
            let mask = (1u8 << tail) - 1;
            zeros += (byte | !mask).count_zeros() as usize;
        }
    }
    zeros
}
