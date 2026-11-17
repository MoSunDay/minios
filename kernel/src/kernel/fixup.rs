//! UserPtr fault-fixup table (M1R): map a faulting kernel RIP to its
//! recovery continuation, converting #PF/#GP inside copyin/copyout-style
//! primitives into -EFAULT returns.

use super::fixup_table::FixupEntry;
use super::traps::Trapframe;

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

/// If `tf.rip` is registered in the fixup table, redirect execution to the
/// recovery continuation and report the fault type (for the expected-fault
/// suite observer).
pub fn apply(tf: &mut Trapframe) -> bool {
    let table = table();
    if table.is_empty() {
        return false;
    }
    // Entries are few (one pair per guarded instruction); linear scan keeps
    // the section order-independent.
    for e in table {
        if e.fault_rip == tf.rip {
            tf.rip = e.fixup_rip;
            return true;
        }
    }
    false
}

