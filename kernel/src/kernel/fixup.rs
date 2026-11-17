//! UserPtr fault-fixup table (M1R): map a faulting kernel RIP to its
//! recovery continuation, converting #PF/#GP inside copyin/copyout-style
//! primitives into -EFAULT returns.

use super::fixup_table::{self, FixupEntry};

unsafe extern "C" {
    static __fixup_start: u8;
    static __fixup_end: u8;
}

pub fn table() -> &'static [FixupEntry] {
    // SAFETY: linker-bounded section of plain u64 pairs, read-only.
    unsafe {
        let start = core::ptr::addr_of!(__fixup_start) as usize;
        let end = core::ptr::addr_of!(__fixup_end) as usize;
        let len = (end - start) / core::mem::size_of::<FixupEntry>();
        if end < start {
            return &[];
        }
        core::slice::from_raw_parts(start as *const FixupEntry, len)
    }
}

/// Exact-RIP lookup: the recovery continuation registered for `rip`, or
/// None when this fault site is not guarded. Exact-only by design (22 §2):
/// fixups are per-instruction, never region-based.
pub fn lookup(rip: u64) -> Option<u64> {
    fixup_table::lookup(table(), rip)
}
