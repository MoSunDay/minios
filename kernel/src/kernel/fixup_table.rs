//! UserPtr fault-fixup table types and pure lookup (host-testable; the
//! linker-bounded table itself lives in fixup.rs).

/// One .m1r.fixup entry: (fault_rip, fixup_rip).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FixupEntry {
    pub fault_rip: u64,
    pub fixup_rip: u64,
}

/// Pure predicate form used by host unit tests over an explicit table.
#[allow(dead_code)] // pure form exercised by host-tests
pub fn lookup(table: &[FixupEntry], rip: u64) -> Option<u64> {
    table
        .iter()
        .find(|e| e.fault_rip == rip)
        .map(|e| e.fixup_rip)
}
