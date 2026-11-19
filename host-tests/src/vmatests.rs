//! Host tests for the M2 anonymous VMA core (kernel/src/mm/vma.rs via
//! #[path]): sorted insert/merge, split, remove-range, re-protect and
//! top-down placement (plans/linux26/22 §4/§6.1 "VMA 分裂合并").

use super::vma::{
    self, Vma, VmaTable, NVMA, PROT_ALL, PROT_R, PROT_RW, PROT_RWX, VMA_ANON, VMA_HEAP,
};

const K: usize = 4096;

fn vma(start: usize, end: usize, prot: u32, flags: u32) -> Vma {
    Vma {
        start,
        end,
        prot,
        flags,
    }
}

fn table_of(vmas: &[Vma]) -> VmaTable {
    let mut t = VmaTable::empty();
    for v in vmas {
        assert!(t.insert(*v), "seed insert {v:?}");
    }
    t
}

#[test]
fn insert_sorts_and_merges_touching_same_attrs() {
    let mut t = VmaTable::empty();
    assert!(t.insert(vma(3 * K, 4 * K, PROT_RW, VMA_ANON)));
    assert!(t.insert(vma(K, 2 * K, PROT_RW, VMA_ANON)));
    assert!(t.insert(vma(2 * K, 3 * K, PROT_RW, VMA_ANON))); // bridges both
    assert_eq!(t.count, 1);
    assert_eq!(t.vmas[0], vma(K, 4 * K, PROT_RW, VMA_ANON));
}

#[test]
fn insert_keeps_distinct_neighbors_separate() {
    // Different prot or flags => no merge even when touching.
    let t = table_of(&[
        vma(K, 2 * K, PROT_R, VMA_ANON),
        vma(2 * K, 3 * K, PROT_RW, VMA_ANON),
        vma(3 * K, 4 * K, PROT_R, 0),
    ]);
    assert_eq!(t.count, 3);
    // Whole-table ordering is maintained.
    assert!(t.vmas[..t.count].windows(2).all(|w| w[0].end <= w[1].start));
}

#[test]
fn insert_rejects_overlap_bad_geometry_and_bits() {
    let mut t = table_of(&[vma(2 * K, 4 * K, PROT_RW, VMA_ANON)]);
    // Overlaps right edge, left edge, strictly inside, and encloses.
    assert!(!t.insert(vma(3 * K, 5 * K, PROT_RW, VMA_ANON)));
    assert!(!t.insert(vma(K, 3 * K, PROT_RW, VMA_ANON)));
    assert!(!t.insert(vma(2 * K + 100, 3 * K, PROT_RW, VMA_ANON)));
    assert!(!t.insert(vma(K, 5 * K, PROT_RW, VMA_ANON)));
    // Inverted/empty range.
    assert!(!t.insert(vma(9 * K, 8 * K, PROT_RW, VMA_ANON)));
    assert!(!t.insert(vma(8 * K, 8 * K, PROT_RW, VMA_ANON)));
    // Unaligned non-heap edges.
    assert!(!t.insert(vma(6 * K + 1, 8 * K, PROT_RW, VMA_ANON)));
    assert!(!t.insert(vma(6 * K, 8 * K + 1, PROT_RW, VMA_ANON)));
    // Unknown prot/flag bits.
    assert!(!t.insert(vma(6 * K, 8 * K, PROT_ALL | (1 << 3), VMA_ANON)));
    assert!(!t.insert(vma(6 * K, 8 * K, PROT_RW, 1 << 3)));
    assert_eq!(t.count, 1);
}

#[test]
fn heap_vma_allows_byte_precise_break() {
    let mut t = VmaTable::empty();
    // Exec plants the heap page-aligned; brk trims/grows byte-exactly.
    assert!(t.insert(vma(2 * K, 3 * K, PROT_RW, VMA_HEAP)));
    assert!(t.insert(vma(3 * K, 3 * K + 100, PROT_RW, VMA_HEAP))); // grow, merge
    assert_eq!(t.count, 1);
    assert_eq!(t.vmas[0].end, 3 * K + 100);
    // Shrink back through remove_range with a byte-exact edge.
    assert!(t.remove_range(2 * K + 50, 3 * K + 100));
    assert_eq!(t.count, 1);
    assert_eq!(t.vmas[0], vma(2 * K, 2 * K + 50, PROT_RW, VMA_HEAP));
}

#[test]
fn find_and_overlap_boundaries() {
    let t = table_of(&[
        vma(K, 2 * K, PROT_R, 0),
        vma(4 * K, 6 * K, PROT_RW, VMA_ANON),
    ]);
    assert_eq!(t.find(K), Some(0));
    assert_eq!(t.find(2 * K - 1), Some(0));
    assert_eq!(t.find(2 * K), None); // end is exclusive
    assert_eq!(t.find(3 * K), None);
    assert_eq!(t.find(5 * K), Some(1));
    assert_eq!(t.overlap(2 * K, 4 * K), None); // touching only
    assert_eq!(t.overlap(2 * K - 1, 4 * K), Some(0));
    assert_eq!(t.overlap(0, usize::MAX), Some(0));
    assert_eq!(t.overlap(7 * K, 6 * K), None); // inverted
}

#[test]
fn split_at_middle_edges_and_outside() {
    let mut t = table_of(&[vma(K, 4 * K, PROT_RW, VMA_ANON)]);
    assert!(t.split_at(2 * K)); // middle
    assert_eq!(t.count, 2);
    assert_eq!(t.vmas[0], vma(K, 2 * K, PROT_RW, VMA_ANON));
    assert_eq!(t.vmas[1], vma(2 * K, 4 * K, PROT_RW, VMA_ANON));
    assert!(t.split_at(K)); // left edge: no-op
    assert!(t.split_at(4 * K)); // right edge (== next gap): no-op
    assert!(t.split_at(8 * K)); // outside: no-op
    assert_eq!(t.count, 2);
    // Idempotent on an existing boundary.
    assert!(t.split_at(2 * K));
    assert_eq!(t.count, 2);
}

#[test]
fn split_fails_cleanly_when_full() {
    let mut t = VmaTable::empty();
    for i in 0..NVMA {
        // Odd prot pattern keeps neighbors unmergeable.
        let prot = if i % 2 == 0 { PROT_R } else { PROT_RW };
        assert!(t.insert(vma(i * K, (i + 1) * K, prot, VMA_ANON)));
    }
    assert_eq!(t.count, NVMA);
    assert!(!t.split_at(K / 2)); // cannot fit the second half
    assert_eq!(t.count, NVMA);
    assert!(!t.insert(vma(NVMA * K, (NVMA + 1) * K, PROT_R, VMA_ANON)));
    // remove_range on an inside range must now fail untouched.
    assert!(!t.remove_range(K + 100, 2 * K - 100));
    assert_eq!(t.count, NVMA);
}

#[test]
fn remove_range_whole_partial_and_multi() {
    let mut t = table_of(&[
        vma(K, 2 * K, PROT_R, 0),
        vma(3 * K, 5 * K, PROT_RW, VMA_ANON),
        vma(5 * K, 7 * K, PROT_RW, VMA_ANON), // same attrs: merged on insert
    ]);
    assert_eq!(t.count, 2); // 3K..7K merged
                            // Tail trim.
    assert!(t.remove_range(4 * K, 9 * K));
    assert_eq!(t.vmas[1], vma(3 * K, 4 * K, PROT_RW, VMA_ANON));
    // Head trim (the first VMA goes too; the heap piece moves to slot 0).
    assert!(t.remove_range(0, 3 * K + 100));
    assert_eq!(t.count, 1);
    assert_eq!(t.vmas[0], vma(3 * K + 100, 4 * K, PROT_RW, VMA_ANON));
    // Strictly-inside split.
    assert!(t.remove_range(3 * K + 200, 4 * K - 200));
    assert_eq!(t.count, 2);
    assert_eq!(t.vmas[0], vma(3 * K + 100, 3 * K + 200, PROT_RW, VMA_ANON));
    assert_eq!(t.vmas[1], vma(4 * K - 200, 4 * K, PROT_RW, VMA_ANON));
    // Empty/inverted ranges are no-ops.
    assert!(t.remove_range(9 * K, 8 * K));
    assert_eq!(t.count, 2);
    // Removing everything.
    assert!(t.remove_range(0, usize::MAX));
    assert_eq!(t.count, 0);
}

#[test]
fn set_prot_reprotects_and_remerges() {
    let mut t = table_of(&[vma(K, 8 * K, PROT_RW, VMA_ANON)]);
    // Carve a read-only middle: split+protect leaves three pieces.
    assert!(t.set_prot(3 * K, 5 * K, PROT_R));
    assert_eq!(t.count, 3);
    assert_eq!(t.vmas[1], vma(3 * K, 5 * K, PROT_R, VMA_ANON));
    // Return it to RW: neighbors re-merge into one VMA.
    assert!(t.set_prot(3 * K, 5 * K, PROT_RW));
    assert_eq!(t.count, 1);
    assert_eq!(t.vmas[0], vma(K, 8 * K, PROT_RW, VMA_ANON));
    // PROT_NONE reservation is representable.
    assert!(t.set_prot(2 * K, 3 * K, vma::PROT_NONE));
    assert_eq!(t.vmas[1].prot, 0);
    // Unknown bits rejected, range unchanged.
    assert!(!t.set_prot(2 * K, 3 * K, 1 << 4));
    assert_eq!(t.vmas[1].prot, 0);
}

#[test]
fn set_prot_requires_full_coverage() {
    let mut t = table_of(&[vma(K, 3 * K, PROT_RW, VMA_ANON)]);
    assert!(!t.set_prot(2 * K, 4 * K, PROT_R)); // pokes past the end
    assert!(!t.set_prot(0, 2 * K, PROT_R)); // before the start
    assert_eq!(t.count, 1); // untouched (no partial splits leak)
    assert_eq!(t.vmas[0], vma(K, 3 * K, PROT_RW, VMA_ANON));
    // A real hole (two VMAs with a gap): crossing it fails.
    let mut t2 = table_of(&[
        vma(K, 2 * K, PROT_RW, VMA_ANON),
        vma(3 * K, 4 * K, PROT_RW, VMA_ANON),
    ]);
    assert!(!t2.set_prot(K, 4 * K, PROT_R));
    // But the contiguous span of the second table alone works.
    assert!(t2.set_prot(3 * K, 4 * K, PROT_R));
    assert_eq!(t2.vmas[1].prot, PROT_R);
}

#[test]
fn find_area_topdown_picks_highest_gap() {
    let t = table_of(&[
        vma(4 * K, 6 * K, PROT_RW, VMA_ANON),
        vma(10 * K, 12 * K, PROT_RW, VMA_ANON),
    ]);
    // Highest placement: right below the ceiling.
    assert_eq!(t.find_area_topdown(2 * K, 0, 16 * K), Some(14 * K));
    // Squeezed below the top VMA when the ceiling drops to it.
    assert_eq!(t.find_area_topdown(2 * K, 0, 12 * K), Some(8 * K));
    // Ceiling clamps the gap below the top VMA.
    assert_eq!(t.find_area_topdown(2 * K, 0, 10 * K), Some(8 * K));
    // Floor raises the lowest gap out of reach.
    assert_eq!(t.find_area_topdown(2 * K, 3 * K, 10 * K), Some(8 * K));
    assert_eq!(t.find_area_topdown(2 * K, 7 * K, 8 * K), None);
    // Nothing fits an oversized request once the low gap is floored out.
    assert_eq!(t.find_area_topdown(4 * K, K, 8 * K), None);
    // Unaligned len/ceiling rejected.
    assert_eq!(t.find_area_topdown(2 * K + 1, 0, 16 * K), None);
    assert_eq!(t.find_area_topdown(2 * K, 0, 16 * K + 1), None);
}

#[test]
fn regression_brk_cycle_keeps_canonical_form() {
    // exec heap + grow/shrink cycles + an anon mmap above: the steady
    // state stays merged, sorted and hole-free where intended.
    let mut t = VmaTable::empty();
    assert!(t.insert(vma(2 * K, 3 * K, PROT_RWX, 0))); // text+bss
                                                       // Initial break sits at 3K with an empty heap (no VMA until sbrk).
    assert!(t.insert(vma(3 * K, 5 * K, PROT_RW, VMA_HEAP))); // sbrk grow
    assert!(t.insert(vma(8 * K, 9 * K, PROT_RW, VMA_ANON))); // mmap
    assert_eq!(t.count, 3);
    assert!(t.vmas[..t.count].windows(2).all(|w| w[0].end <= w[1].start));
    // munmap the mmap; a new top-down mapping lands highest again.
    assert!(t.remove_range(8 * K, 9 * K));
    assert_eq!(t.find_area_topdown(K, 6 * K, 16 * K), Some(15 * K));
}
