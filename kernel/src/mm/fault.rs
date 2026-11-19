//! User #PF classification and resolution (M2, plans/linux26/22 §4):
//! canonical + error-code + VMA-permission validation, stack VMA downward
//! growth bounded by the guard floor, lazy anonymous zero-fill; anything
//! else kills the faulting process.

use super::frame::PGSIZE;
use super::mmap::prot_to_pte;
use super::pgtable;
use super::vm::{Mm, USER_STACK_TOP};
use super::vma::{self, Vma};

/// #PF error-code bits (Intel Vol.3 6.15).
pub const PF_ERR_P: u64 = 1 << 0; // PTE present
pub const PF_ERR_W: u64 = 1 << 1; // write access
pub const PF_ERR_U: u64 = 1 << 2; // CPL3 access
pub const PF_ERR_I: u64 = 1 << 4; // instruction fetch

/// Highest canonical user address (low half; above is kernel/non-canonical).
pub const USER_CANON_TOP: usize = 0x0000_7FFF_FFFF_FFFF;
/// Lowest legal stack VMA start: the 64 MiB growth window under
/// USER_STACK_TOP doubles as the guard band (mmap.rs keeps MMAP_CEILING
/// below it, so growth can never collide with anonymous mappings).
pub const STACK_GUARD_FLOOR: usize = USER_STACK_TOP - 0x1000_0000;

/// Resolution for one user #PF (classification is pure; apply mutates).
pub enum FaultAction {
    /// Absent page inside a VMA whose permissions cover the access:
    /// map one zeroed page (lazy anonymous fill).
    Fill { va: usize, prot: u32 },
    /// Data access below the stack VMA but at/above the guard floor:
    /// grow the stack VMA down to `new_start` (zero-filled, RW).
    GrowStack { new_start: usize },
    /// Unrecoverable: the caller kills the faulting process.
    Kill,
}

/// Index of the (single) stack VMA, if this aspace has one.
fn stack_vma_index(mm: &Mm) -> Option<usize> {
    (0..mm.vmas.count).find(|&i| mm.vmas.vmas[i].flags & vma::VMA_STACK != 0)
}

/// Classify one user #PF into a resolution action: pure lookup over the
/// VMA table — no mutation, no allocation, host-testable in principle.
/// `err` is the CPU-pushed #PF error code (CR2 = `va`).
pub fn classify(mm: &Mm, va: usize, err: u64) -> FaultAction {
    if va > USER_CANON_TOP {
        return FaultAction::Kill; // non-canonical or kernel half
    }
    if err & PF_ERR_U == 0 {
        return FaultAction::Kill; // kernel-mode access; M1R fixup paths own it
    }
    match mm.vmas.find(va) {
        Some(i) => {
            let v = mm.vmas.vmas[i];
            if err & PF_ERR_W != 0 && v.prot & vma::PROT_W == 0 {
                return FaultAction::Kill; // write to read-only
            }
            if err & PF_ERR_I != 0 && v.prot & vma::PROT_X == 0 {
                return FaultAction::Kill; // NX
            }
            if v.prot == vma::PROT_NONE {
                return FaultAction::Kill; // reserved, not mapped
            }
            if err & PF_ERR_P != 0 {
                return FaultAction::Kill; // present + permitted: stale/foreign PTE
            }
            FaultAction::Fill {
                va: pgtable::pgrounddown(va),
                prot: v.prot,
            }
        }
        None => match stack_vma_index(mm) {
            Some(i)
                if va < mm.vmas.vmas[i].start && va >= STACK_GUARD_FLOOR && err & PF_ERR_I == 0 =>
            {
                FaultAction::GrowStack {
                    new_start: pgtable::pgrounddown(va),
                }
            }
            _ => FaultAction::Kill,
        },
    }
}

/// Apply a classified action, mutating the address space. false means
/// unrecoverable (bad state or allocation failure): the caller kills the
/// faulting process.
pub fn apply(mm: &mut Mm, action: &FaultAction) -> bool {
    match action {
        FaultAction::Fill { va, prot } => {
            // Never overwrite an existing translation.
            // SAFETY: valid table; walk allocates nothing.
            let pte = unsafe { pgtable::walk(mm.pml4, *va, false) };
            if !pte.is_null() && (unsafe { *pte } & pgtable::PTE_P != 0) {
                return false;
            }
            mm.map_zero_page(*va, prot_to_pte(*prot))
        }
        FaultAction::GrowStack { new_start } => match stack_vma_index(mm) {
            None => false,
            Some(idx) => {
                let old = mm.vmas.vmas[idx];
                let _ = mm.vmas.remove_range(old.start, old.end);
                let newv = Vma {
                    start: *new_start,
                    end: old.end,
                    prot: old.prot,
                    flags: old.flags,
                };
                if !mm.vmas.insert(newv) {
                    return false;
                }
                // Eagerly zero-fill the growth window so the retried
                // faulting access succeeds.
                let mut a = *new_start;
                while a < old.start {
                    if !mm.map_zero_page(a, prot_to_pte(old.prot)) {
                        return false;
                    }
                    a += PGSIZE;
                }
                true
            }
        },
        FaultAction::Kill => false,
    }
}

/// Traps-side entry: classify one user #PF and apply its resolution.
/// false = kill the faulting process.
pub fn handle_user_fault(mm: &mut Mm, va: usize, err: u64) -> bool {
    let action = classify(mm, va, err);
    apply(mm, &action)
}
