//! User #PF classification and resolution (M2, plans/linux26/22 §4):
//! canonical + error-code + VMA-permission validation, stack VMA downward
//! growth bounded by the guard floor, lazy anonymous zero-fill; anything
//! else kills the faulting process.

use super::frame::{self, PGSIZE};
use super::mmap::prot_to_pte;
use super::pgtable;
use super::vm::{Mm, USER_STACK_TOP};
use super::vma::{self, Vma};
use crate::arch::x86_64::io;

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
    /// Write to a present read-only page whose VMA still grants write:
    /// the fork-time COW break — copy the frame or re-own it.
    Cow { va: usize },
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
        // Direct kernel-mode callers Kill; the traps layer routes real
        // supervisor #PFs through handle_kernel_user_fault, which
        // synthesizes the user bit for classification.
        return FaultAction::Kill;
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
                // Present + permitted leaves exactly one legal form: a
                // write to a page the VMA still allows but whose PTE
                // lost W at fork (COW). Anything else is stale/foreign.
                if err & PF_ERR_W != 0 && v.prot & vma::PROT_W != 0 {
                    // SAFETY: valid table; walk allocates nothing.
                    let pte = unsafe { pgtable::walk(mm.pml4, va, false) };
                    let cow = !pte.is_null() && unsafe { *pte } & pgtable::PTE_W == 0;
                    if cow {
                        return FaultAction::Cow {
                            va: pgtable::pgrounddown(va),
                        };
                    }
                }
                return FaultAction::Kill;
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
        FaultAction::Cow { va } => {
            // SAFETY: valid table; walk allocates nothing.
            let pte = unsafe { pgtable::walk(mm.pml4, *va, false) };
            if pte.is_null() {
                return false;
            }
            let entry = unsafe { *pte };
            if entry & pgtable::PTE_P == 0 {
                return false;
            }
            let pa = entry & pgtable::PTE_ADDR;
            let keep = entry & pgtable::PTE_NX;
            if frame::page_ref(pa as usize) == 1 {
                // Sole owner left (the other side exited or unmapped):
                // re-own in place, no copy.
                unsafe { *pte = pa | pgtable::PTE_P | pgtable::PTE_U | pgtable::PTE_W | keep };
                // SAFETY: this mm may be the active CR3.
                io::invlpg(*va as u64);
                return true;
            }
            let page = super::oom::alloc_user_page();
            if page.is_null() {
                return false;
            }
            // SAFETY: fresh destination; source frame stays mapped by
            // its remaining owner.
            unsafe {
                crate::klib::string::memmove(page, frame::p2v(pa as usize) as *mut u8, PGSIZE);
                *pte = frame::v2p(page as usize) as u64
                    | pgtable::PTE_P
                    | pgtable::PTE_U
                    | pgtable::PTE_W
                    | keep;
            }
            // SAFETY: this mm may be the active CR3.
            io::invlpg(*va as u64);
            // Drop this side's share of the old frame (the allocation
            // ticket stays with the remaining owner).
            frame::page_ref_dec(pa as usize);
            true
        }
        FaultAction::Kill => false,
    }
}

/// Traps-side entry: classify one user #PF and apply its resolution.
/// false = kill the faulting process.
pub fn handle_user_fault(mm: &mut Mm, va: usize, err: u64) -> bool {
    let action = classify(mm, va, err);
    apply(mm, &action)
}

/// Supervisor-mode #PF on a user address: uaccess copies (copyout into a
/// fork-COW page, copyin from a lazily absent page) fault with err.U=0
/// even though the W/P bits apply to CPL0 accesses identically. Resolve
/// them against the running process's mm — classifying with the user bit
/// synthesized, since every VMA/PTE rule the classifier checks depends
/// only on the access type (P/W/I), never on the privilege level. false
/// keeps the M1R fixup/oops semantics for genuine EFAULT untouched.
pub fn handle_kernel_user_fault(va: usize, err: u64) -> bool {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return false; // boot/kernel context: fixup owns it
    }
    // SAFETY: the running process owns mm; resolution only takes the
    // frame allocator lock (uaccess never runs under it).
    match unsafe { (*p).mm.as_mut() } {
        Some(mm) => handle_user_fault(mm, va, err | PF_ERR_U),
        None => false, // kernel thread: fixup owns it
    }
}
