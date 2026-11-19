//! M2 kernel-suite demand-paging cases (22 §4–§5): fault classification
//! (stack VMA growth with guard floor, lazy anonymous zero-fill), COW
//! fork page accounting, and the anonymous OOM policy. Every case is
//! frame-conservative: the buddy count starts and ends equal.

use super::m2map::pte_of;
use crate::mm::{fault, frame, oom, pgtable, vm, vma};
use frame::{free_frames, PGSIZE};

/// Stack VMA growth and guard (22 §4): a data fault below the stack VMA
/// but at/above the guard floor grows the VMA down with zeroed RW|NX
/// pages; below the floor, above the top, or an instruction fetch is
/// killed. Growth is repeatable and keeps the table sorted.
pub(super) fn case_fault_stack() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let top = vm::USER_STACK_TOP;
    let s0 = top - vm::USER_STACK_PAGES * PGSIZE;
    if !mm.vmas.insert(vma::Vma {
        start: s0,
        end: top,
        prot: vma::PROT_RW,
        flags: vma::VMA_STACK,
    }) {
        mm.free();
        return false;
    }
    // One page below the stack: growth with the faulting page zeroed.
    let act = fault::classify(&mm, s0 - PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W);
    let mut ok =
        matches!(act, fault::FaultAction::GrowStack { new_start } if new_start == s0 - PGSIZE);
    ok = ok
        && fault::apply(
            &mut mm,
            &fault::FaultAction::GrowStack {
                new_start: s0 - PGSIZE,
            },
        );
    ok = ok
        && mm.vmas.count == 1
        && mm.vmas.vmas[0].start == s0 - PGSIZE
        && mm.vmas.vmas[0].end == top
        && mm.vmas.vmas[0].flags & vma::VMA_STACK != 0;
    let e = pte_of(mm.pml4, s0 - PGSIZE);
    ok = ok && e & pgtable::PTE_P != 0 && e & pgtable::PTE_W != 0 && e & pgtable::PTE_NX != 0;
    if e & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
        ok = ok && w == 0;
    }
    // Growth is repeatable (deep recursion pattern) and eager below the
    // new start only.
    let act2 = fault::classify(&mm, s0 - 3 * PGSIZE, fault::PF_ERR_U);
    ok = ok
        && matches!(act2, fault::FaultAction::GrowStack { new_start } if new_start == s0 - 3 * PGSIZE);
    ok = ok
        && fault::apply(
            &mut mm,
            &fault::FaultAction::GrowStack {
                new_start: s0 - 3 * PGSIZE,
            },
        );
    ok = ok && mm.vmas.count == 1 && mm.vmas.vmas[0].start == s0 - 3 * PGSIZE;
    ok = ok && pte_of(mm.pml4, s0 - 2 * PGSIZE) & pgtable::PTE_P != 0;
    // Guard band: below the floor is killed, instruction fetch never grows.
    ok = ok
        && matches!(
            fault::classify(
                &mm,
                fault::STACK_GUARD_FLOOR - PGSIZE,
                fault::PF_ERR_U | fault::PF_ERR_W
            ),
            fault::FaultAction::Kill
        );
    ok = ok
        && matches!(
            fault::classify(&mm, s0 - 4 * PGSIZE, fault::PF_ERR_U | fault::PF_ERR_I),
            fault::FaultAction::Kill
        );
    // Above the stack top: no VMA, killed.
    ok = ok
        && matches!(
            fault::classify(&mm, top + PGSIZE, fault::PF_ERR_U),
            fault::FaultAction::Kill
        );
    mm.free();
    ok && free_frames() == base
}

/// Lazy anonymous zero-fill and kill classification (22 §4): pages of a
/// PROT_NONE reservation re-permissioned to RW fault in one page at a
/// time; write-to-readonly, NX fetch, non-canonical and kernel-source
/// faults are all killed.
pub(super) fn case_fault_lazy() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    // Reserve 2 pages, then make them RW: the PTEs stay absent (lazy).
    let c = mm.mmap_anon(0, 2 * PGSIZE, vma::PROT_NONE, false);
    if c == 0 || !mm.mprotect(c, 2 * PGSIZE, vma::PROT_RW) {
        mm.free();
        return false;
    }
    let mut ok = pte_of(mm.pml4, c + PGSIZE) & pgtable::PTE_P == 0;
    let act = fault::classify(&mm, c + PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W);
    ok = ok
        && matches!(act, fault::FaultAction::Fill { va, prot } if va == c + PGSIZE && prot == vma::PROT_RW);
    ok = ok && fault::apply(&mut mm, &act);
    let e = pte_of(mm.pml4, c + PGSIZE);
    ok = ok
        && e & pgtable::PTE_P != 0
        && e & pgtable::PTE_U != 0
        && e & pgtable::PTE_W != 0
        && e & pgtable::PTE_NX != 0;
    if e & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
        ok = ok && w == 0;
    }
    // Demand paging: the neighbour is still absent, and its fill drops
    // exactly one frame (the region's page tables are already warm).
    ok = ok && pte_of(mm.pml4, c) & pgtable::PTE_P == 0;
    let before = free_frames();
    let act0 = fault::classify(&mm, c, fault::PF_ERR_U);
    ok = ok && matches!(act0, fault::FaultAction::Fill { va, .. } if va == c);
    ok = ok && fault::apply(&mut mm, &act0);
    ok = ok && before - free_frames() == 1;
    // Second fault on the now-present page (stale P): killed.
    ok = ok
        && matches!(
            fault::classify(
                &mm,
                c + PGSIZE,
                fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P
            ),
            fault::FaultAction::Kill
        );
    // Re-protect page 2 read-only: a write fault is killed by VMA prot.
    ok = ok && mm.mprotect(c + PGSIZE, PGSIZE, vma::PROT_R);
    ok = ok
        && matches!(
            fault::classify(&mm, c + PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W),
            fault::FaultAction::Kill
        );
    // Instruction fetch on a non-exec VMA: killed.
    ok = ok
        && matches!(
            fault::classify(&mm, c, fault::PF_ERR_U | fault::PF_ERR_I),
            fault::FaultAction::Kill
        );
    // Non-canonical address and kernel-source fault: killed.
    ok = ok
        && matches!(
            fault::classify(&mm, 0x0000_8000_0000_0000, fault::PF_ERR_U),
            fault::FaultAction::Kill
        );
    ok = ok && matches!(fault::classify(&mm, c, 0), fault::FaultAction::Kill);
    // PROT_NONE access is killed regardless of error code bits.
    let d = mm.mmap_anon(0, PGSIZE, vma::PROT_NONE, false);
    ok = ok
        && d != 0
        && matches!(
            fault::classify(&mm, d, fault::PF_ERR_U | fault::PF_ERR_W),
            fault::FaultAction::Kill
        );
    mm.free();
    ok && free_frames() == base
}

/// COW fork page accounting (22 §4): fork shares every writable frame
/// read-only on both sides (refcount 2), a child write fault breaks into
/// a private copy (old frame back to 1, content preserved), and the sole
/// remaining owner re-owns in place without a copy. Teardown returns
/// every frame exactly once.
pub(super) fn case_cow_pagecount() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, PGSIZE, vma::PROT_RW, false);
    if a == 0 {
        mm.free();
        return false;
    }
    let e = pte_of(mm.pml4, a);
    let pa1 = e & pgtable::PTE_ADDR;
    // SAFETY: direct-map write into the mapped frame.
    unsafe {
        *(frame::p2v(pa1 as usize) as *mut u64) = 0x1234_5678_9abc_def0;
    }
    // fork: no data frame is copied — the child maps the same physical
    // page, write is broken on both sides.
    let before = free_frames();
    // SAFETY: scratch mm with present user mappings.
    let child = unsafe { pgtable::copy_user_half(mm.pml4) };
    // Tables only: the child needs pml4+pdpt+pd+pt for the region and
    // copies no data frame.
    let mut ok = !child.is_null() && before - free_frames() == 4;
    let ce = pte_of(child, a);
    ok = ok
        && ce & pgtable::PTE_P != 0
        && ce & pgtable::PTE_ADDR == pa1
        && ce & pgtable::PTE_W == 0
        && ce & pgtable::PTE_NX != 0;
    ok = ok && pte_of(mm.pml4, a) & pgtable::PTE_W == 0; // parent broken too
    ok = ok && frame::page_ref(pa1 as usize) == 2;
    // Child write fault: classify as COW, apply to a private copy.
    let child_mm = vm::Mm {
        pml4: child,
        vmas: mm.vmas,
        brk: mm.brk,
    };
    let mut child_mm = child_mm;
    let act = fault::classify(
        &child_mm,
        a,
        fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P,
    );
    ok = ok && matches!(act, fault::FaultAction::Cow { va } if va == a);
    ok = ok && fault::apply(&mut child_mm, &act);
    let ce2 = pte_of(child, a);
    let pa2 = ce2 & pgtable::PTE_ADDR;
    ok = ok
        && pa2 != pa1
        && ce2 & pgtable::PTE_W != 0
        && ce2 & pgtable::PTE_U != 0
        && ce2 & pgtable::PTE_NX != 0
        && frame::page_ref(pa1 as usize) == 1
        && frame::page_ref(pa2 as usize) == 1;
    // The copy carried the stamped marker.
    if ce2 & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v(pa2 as usize) as *const u64) };
        ok = ok && w == 0x1234_5678_9abc_def0;
    }
    // Parent write fault on the now-solely-owned frame: re-own in place,
    // no copy, same physical page.
    let act2 = fault::classify(&mm, a, fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P);
    ok = ok && matches!(act2, fault::FaultAction::Cow { va } if va == a);
    ok = ok && fault::apply(&mut mm, &act2);
    ok = ok
        && pte_of(mm.pml4, a) & pgtable::PTE_W != 0
        && pte_of(mm.pml4, a) & pgtable::PTE_ADDR == pa1;
    // Teardown: both spaces release every frame exactly once.
    // SAFETY: child tables no longer referenced.
    unsafe { pgtable::free_user_pml4(child) };
    mm.free();
    ok && free_frames() == base
}

/// Anonymous OOM policy (22 §5): pure candidate selection (largest
/// footprint, lowest pid on ties, zero pages never eligible), and the
/// real allocation-failure trigger in kernel-only context — the buddy
/// runs dry through an oversized anonymous mapping, the refusal is
/// counted, no user process exists to kill, the rollback loses no
/// frame, and the allocator serves again afterwards.
pub(super) fn case_oom_select_stats() -> bool {
    let cands = [
        oom::Candidate {
            pid: 5,
            anon_pages: 10,
        },
        oom::Candidate {
            pid: 7,
            anon_pages: 90,
        },
        oom::Candidate {
            pid: 6,
            anon_pages: 90,
        },
        oom::Candidate {
            pid: 8,
            anon_pages: 0,
        },
    ];
    let mut ok = oom::select_oom_victim(&cands) == Some(2); // tie -> lower pid
    ok = ok && oom::select_oom_victim(&cands[..1]) == Some(0);
    ok = ok && oom::select_oom_victim(&cands[3..4]).is_none(); // zero pages
    ok = ok && oom::select_oom_victim(&[]).is_none();
    // Real trigger: demand more than the whole buddy through one eager
    // anonymous mapping; mmap_anon must refuse and roll back cleanly.
    let base = free_frames();
    let s0 = oom::stats();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    // 1 GiB of RW anon cannot fit in PHYS_TOP (256 MiB).
    let refused = mm.mmap_anon(0, 1 << 30, vma::PROT_RW, false);
    let s1 = oom::stats();
    ok = ok && refused == 0;
    ok = ok && s1.failed > s0.failed; // the failure is counted
    ok = ok && s1.kills == s0.kills; // no user procs: nobody killed
    ok = ok && s1.scanned > s0.scanned; // the table was scanned
    mm.free();
    ok = ok && free_frames() == base; // rollback + teardown: no loss
                                      // The valve recovers: a modest mapping succeeds again (fresh Mm —
                                      // free() retired the old pml4).
    let mut mm2 = match vm::Mm::new() {
        Some(m) => m,
        None => return ok && false,
    };
    let c = mm2.mmap_anon(0, PGSIZE, vma::PROT_RW, false);
    ok = ok && c != 0;
    mm2.free();
    ok && free_frames() == base
}
