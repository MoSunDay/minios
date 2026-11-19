//! M2 kernel-suite mapping cases (22 §4): exec-style VMA layout with a
//! byte-exact brk cycle, eager anonymous mmap placement/unmap frame
//! accounting, and mprotect PTE repermissioning. Frame-conservative:
//! every case starts and ends with equal buddy counts. The leaf-PTE
//! reader here is shared with the fault/COW/OOM cases (m2mem.rs).

use crate::console;
use crate::mm::{frame, mmap, pgtable, vm, vma};
use frame::{free_frames, PGSIZE};

/// Read a leaf PTE (0 when absent).
pub(super) fn pte_of(pml4: pgtable::PtPtr, va: usize) -> u64 {
    // SAFETY: boot-context scratch mm; walk allocates nothing.
    let pte = unsafe { pgtable::walk(pml4, va, false) };
    if pte.is_null() {
        return 0;
    }
    // SAFETY: valid table page.
    unsafe { *pte }
}

/// exec-style VMA layout + brk cycle: segment VMAs carry their ELF
/// permissions into the PTEs (text W-off, data NX), the heap VMA tracks
/// a byte-exact break through grow/shrink/regrow, and the address space
/// frees without leaking frames.
pub(super) fn case_vma_layout() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let text = 0x1_0000usize;
    let data = 0x1_1000usize;
    // Two segments: RX text, RW data (exec's insert order).
    let ok = mm.alloc(text, data, mmap::prot_to_pte(vma::PROT_R | vma::PROT_X)) == data
        && mm.vmas.insert(vma::Vma {
            start: text,
            end: data,
            prot: vma::PROT_R | vma::PROT_X,
            flags: 0,
        })
        && mm.alloc(data, data + PGSIZE, mmap::prot_to_pte(vma::PROT_RW)) == data + PGSIZE
        && mm.vmas.insert(vma::Vma {
            start: data,
            end: data + PGSIZE,
            prot: vma::PROT_RW,
            flags: 0,
        });
    if !ok {
        mm.free();
        return false;
    }
    let mut ok = true;
    // Text page: present+user, NOT writable, NOT NX (executable).
    let tpte = pte_of(mm.pml4, text);
    ok = ok
        && tpte & pgtable::PTE_P != 0
        && tpte & pgtable::PTE_U != 0
        && tpte & pgtable::PTE_W == 0
        && tpte & pgtable::PTE_NX == 0;
    // Data page: writable and NX.
    let dpte = pte_of(mm.pml4, data);
    ok = ok
        && dpte & pgtable::PTE_P != 0
        && dpte & pgtable::PTE_W != 0
        && dpte & pgtable::PTE_NX != 0;
    // Segments with different attrs never merge.
    ok = ok && mm.vmas.count == 2;
    // brk: page-aligned start, unaligned grow, exact shrink, regrow.
    mm.brk = data + PGSIZE;
    ok = ok && mm.set_brk(data + PGSIZE + 0x123) == data + PGSIZE + 0x123;
    ok = ok && mm.brk == data + PGSIZE + 0x123;
    ok = ok && mm.set_brk(data + PGSIZE) == data + PGSIZE;
    // Heap VMA vanishes on full shrink; regrow re-creates it byte-exact.
    ok = ok && mm.set_brk(data + 2 * PGSIZE + 0x45) == data + 2 * PGSIZE + 0x45;
    ok = ok && mm.vmas.count == 3;
    let heap = mm.vmas.vmas[2];
    ok = ok
        && heap.flags & vma::VMA_HEAP != 0
        && heap.start == data + PGSIZE
        && heap.end == data + 2 * PGSIZE + 0x45;
    // Sorted, non-overlapping table.
    ok = ok
        && mm.vmas.vmas[..mm.vmas.count]
            .windows(2)
            .all(|w| w[0].end <= w[1].start);
    let _fb = free_frames();
    if _fb != base {
        console::write_hex(_fb);
        console::write_str(" vs ");
        console::write_hex(base);
        console::write_str("\n");
    }
    mm.free();
    let _fa = free_frames();
    ok && _fa == base
}

/// Eager anonymous mmap: top-down disjoint placement, zero-filled RW|NX
/// pages, PROT_NONE reserves without mapping, munmap frees frames, and
/// non-anonymous ranges refuse to unmap.
pub(super) fn case_mmap_eager() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, 3 * PGSIZE, vma::PROT_RW, false);
    let b = mm.mmap_anon(0, PGSIZE, vma::PROT_RW, false);
    let mut ok = a != 0 && b != 0 && b + PGSIZE <= a && a >= mmap::MMAP_FLOOR;
    // Second mapping packs directly below the first.
    ok = ok && b + PGSIZE == a;
    // Pages present, RW, NX, and zeroed.
    for off in 0..3 * PGSIZE {
        let e = pte_of(mm.pml4, a + off);
        ok = ok && e & pgtable::PTE_P != 0 && e & pgtable::PTE_W != 0 && e & pgtable::PTE_NX != 0;
        if e & pgtable::PTE_P != 0 {
            // SAFETY: direct-map word inside the mapped frame.
            let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
            ok = ok && w == 0;
        }
    }
    // Table anatomy under top-down placement: 1 pml4 + 1 pdpt + 1 pd +
    // 1 pt (the four pages share one 2 MiB span) plus 4 data frames.
    let frames_mapped = base - free_frames();
    ok = ok && frames_mapped == 8;
    // PROT_NONE reserves address space only.
    let c = mm.mmap_anon(0, PGSIZE, vma::PROT_NONE, false);
    ok = ok && c != 0 && c + PGSIZE == b && pte_of(mm.pml4, c) & pgtable::PTE_P == 0;
    ok = ok && free_frames() == base - 8;
    // munmap frees exactly the mapping's frames; a PROT_NONE range
    // unmaps fine (no frames to free).
    ok = ok && mm.munmap(a, 3 * PGSIZE);
    ok = ok && pte_of(mm.pml4, a) & pgtable::PTE_P == 0;
    // b stays mapped: 4 table pages + b's data page.
    ok = ok && free_frames() == base - 5;
    ok = ok && mm.munmap(c, PGSIZE);
    // Non-anonymous range (nothing there) refuses.
    ok = ok && !mm.munmap(text_floor(), PGSIZE);
    mm.free();
    ok && free_frames() == base
}

/// A canonical low address that is definitely not anonymous mmap memory.
fn text_floor() -> usize {
    0x2_0000
}

/// mprotect permission bits: RW -> R strips PTE_W; PROT_NONE strips
/// PTE_U; restoring RW keeps the data; holes are rejected and the VMA
/// table re-merges to one piece.
pub(super) fn case_mprotect_perm() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, 2 * PGSIZE, vma::PROT_RW, false);
    if a == 0 {
        mm.free();
        return false;
    }
    // Stamp a marker through the direct map.
    let e = pte_of(mm.pml4, a + PGSIZE);
    // SAFETY: direct-map write into the mapped frame.
    unsafe {
        *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *mut u64) = 0xfeed_beef_cafe_1234;
    }
    let mut ok = true;
    // Read-only over page 2 only: table splits, PTE loses W.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_R);
    let e2 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok
        && e2 & pgtable::PTE_P != 0
        && e2 & pgtable::PTE_U != 0
        && e2 & pgtable::PTE_W == 0
        && e2 & pgtable::PTE_NX != 0;
    ok = ok && mm.vmas.count == 2;
    // Exec permission clears NX.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_R | vma::PROT_X);
    ok = ok && pte_of(mm.pml4, a + PGSIZE) & pgtable::PTE_NX == 0;
    // PROT_NONE keeps the page resident but user-inaccessible.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_NONE);
    let e3 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok && e3 & pgtable::PTE_P != 0 && e3 & pgtable::PTE_U == 0;
    ok = ok && free_frames() == base - 6; // 4 table pages + 2 data, none dropped
                                          // Back to RW: data survives, table re-merges.
    ok = ok && mm.mprotect(a, 2 * PGSIZE, vma::PROT_RW);
    ok = ok && mm.vmas.count == 1;
    let e4 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok && e4 & pgtable::PTE_W != 0 && e4 & pgtable::PTE_U != 0;
    // SAFETY: direct-map read of the same frame.
    let v = unsafe { *(frame::p2v((e4 & pgtable::PTE_ADDR) as usize) as *const u64) };
    ok = ok && v == 0xfeed_beef_cafe_1234;
    // Holes and bad prot are refused untouched.
    ok = ok && !mm.mprotect(a + 4 * PGSIZE, PGSIZE, vma::PROT_R);
    ok = ok && !mm.mprotect(a, PGSIZE, vma::PROT_R | 0x8);
    ok = ok && mm.vmas.count == 1;
    mm.free();
    ok && free_frames() == base
}
