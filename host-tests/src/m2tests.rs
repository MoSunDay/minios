//! Host tests for the M2 portable cores: bootmem region normalization
//! and clipping, buddy conservation/merge/reserved exclusion, SLUB
//! reuse/canary/classification (kernel/src/mm real files via #[path]).

use super::{bootmem, buddy, slub};

// ---- bootmem ------------------------------------------------------------

#[test]
fn bootmem_normalize_sorts_merges_and_resolves_conflicts() {
    let mut t = bootmem::RegionTable::empty();
    // Unsorted input; two adjacent usable ranges and an overlapping
    // reserved range that must win over the union.
    assert!(t.push(0x10_0000, 0x10_0000, bootmem::Kind::Usable));
    assert!(t.push(0x0, 0x1000, bootmem::Kind::Usable));
    assert!(t.push(0x20_0000, 0x10_0000, bootmem::Kind::Usable));
    assert!(t.push(0x18_0000, 0x40_0000, bootmem::Kind::Reserved));
    bootmem::normalize(&mut t);
    assert_eq!(t.count, 2);
    assert_eq!(
        t.regions[0],
        bootmem::Region {
            base: 0,
            len: 0x1000,
            kind: bootmem::Kind::Usable
        }
    );
    // usable [1M,2M) + usable [2M,3M) merge to [1M,3M); reserved
    // [1.5M,5.5M) overlaps -> Reserved union [1M,5.5M).
    assert_eq!(t.regions[1].kind, bootmem::Kind::Reserved);
    assert_eq!(t.regions[1].base, 0x10_0000);
    assert_eq!(t.regions[1].len, 0x48_0000);
    assert_eq!(t.total_len(bootmem::Kind::Usable), 0x1000);
}

#[test]
fn bootmem_normalize_merges_touching_same_kind_only() {
    let mut t = bootmem::RegionTable::empty();
    // usable then reserved touching at 1 MiB: kinds differ, no merge.
    assert!(t.push(0, 0x10_0000, bootmem::Kind::Usable));
    assert!(t.push(0x10_0000, 0x10_0000, bootmem::Kind::Reserved));
    bootmem::normalize(&mut t);
    assert_eq!(t.count, 2);
    assert_eq!(t.regions[0].kind, bootmem::Kind::Usable);
    // Same kind touching merges.
    let mut t2 = bootmem::RegionTable::empty();
    assert!(t2.push(0, 0x10_0000, bootmem::Kind::Reserved));
    assert!(t2.push(0x10_0000, 0x10_0000, bootmem::Kind::Reserved));
    bootmem::normalize(&mut t2);
    assert_eq!(t2.count, 1);
    assert_eq!(t2.regions[0].len, 0x20_0000);
}

#[test]
fn bootmem_clip_usable_aligns_and_clips() {
    let mut t = bootmem::RegionTable::empty();
    assert!(t.push(0x1f_f800, 0x18_1000, bootmem::Kind::Usable)); // [~2M-2K, ~3.5M+2K)
    assert!(t.push(0x40_0000, 0x10_0000, bootmem::Kind::Reserved)); // dropped
    bootmem::normalize(&mut t);
    let c = bootmem::clip_usable(&t, 0x20_0000, 0x30_0000, 4096);
    assert_eq!(c.count, 1);
    assert_eq!(c.regions[0].base, 0x20_0000);
    assert_eq!(c.regions[0].len, 0x10_0000);
    // Floor above the range or top below it -> empty.
    assert_eq!(
        bootmem::clip_usable(&t, 0x50_0000, 0x60_0000, 4096).count,
        0
    );
    assert_eq!(bootmem::clip_usable(&t, 0, 0x1000, 4096).count, 0);
}

#[test]
fn bootmem_probed_top_and_capacity() {
    let mut t = bootmem::RegionTable::empty();
    assert!(t.push(0x10_0000, 0xef_00000, bootmem::Kind::Usable));
    assert!(t.push(0xffb_0000, 0x5_0000, bootmem::Kind::Reserved));
    assert_eq!(bootmem::probed_top(&t), 0x1000_0000);
    // Over-capacity pushes are dropped, not truncated.
    let mut full = bootmem::RegionTable::empty();
    for i in 0..bootmem::MAX_REGIONS {
        assert!(full.push(i as u64 * 0x1000, 0x1000, bootmem::Kind::Usable));
    }
    assert!(!full.push(0xffff_ffff_0000, 0x1000, bootmem::Kind::Usable));
    assert_eq!(full.count, bootmem::MAX_REGIONS);
}

// ---- buddy --------------------------------------------------------------

/// Two half-sized zones over a fully boot-allocated map, released like
/// the kernel boot seeding does.
fn zone_pair(m: &mut [buddy::Page]) -> [buddy::Zone; 2] {
    for p in m.iter_mut() {
        p.flags = 0;
        p.count = 1; // boot-allocated
    }
    let mut zones = [
        buddy::Zone::new("T", 0, (m.len() / 2) as u32),
        buddy::Zone::new("U", (m.len() / 2) as u32, m.len() as u32),
    ];
    for zi in 0..2 {
        buddy::release_zone(&mut zones[zi], m);
    }
    zones
}

#[test]
fn buddy_release_merges_to_maximal_blocks() {
    let mut m = [buddy::Page::EMPTY; 1024];
    let zones = zone_pair(&mut m);
    assert_eq!(zones[0].free_pages, 512);
    assert_eq!(zones[1].free_pages, 512);
    // 512 pages = one order-9 block per zone; order-10 impossible inside
    // a 512-page zone.
    assert_eq!(buddy::largest_free_order(&zones[0]), Some(9));
    assert_eq!(buddy::largest_free_order(&zones[1]), Some(9));
}

#[test]
fn buddy_conserve_interleaved_free_restores_everything() {
    let mut m = [buddy::Page::EMPTY; 1024];
    let mut zones = zone_pair(&mut m);
    // Drain zone U completely.
    let mut got = Vec::new();
    while let Some(pfn) = buddy::alloc_order(&mut zones, &mut m, 1, 0) {
        got.push(pfn);
    }
    assert_eq!(got.len(), 512);
    assert_eq!(zones[1].free_pages, 0);
    assert_eq!(buddy::largest_free_order(&zones[1]), None);
    // Interleaved (even-then-odd) free order still merges everything
    // back into one order-9 block.
    for &pfn in got.iter().step_by(2) {
        assert!(buddy::free_page(&mut zones, &mut m, pfn));
    }
    for &pfn in got.iter().skip(1).step_by(2) {
        assert!(buddy::free_page(&mut zones, &mut m, pfn));
    }
    assert_eq!(zones[1].free_pages, 512);
    assert_eq!(buddy::largest_free_order(&zones[1]), Some(9));
}

#[test]
fn buddy_reserved_pages_never_allocated() {
    let mut m = [buddy::Page::EMPTY; 512];
    for pfn in 16..32 {
        m[pfn] = buddy::Page::EMPTY; // PG_RESERVED, never boot-allocated
    }
    for (pfn, p) in m.iter_mut().enumerate() {
        if !(16..32).contains(&pfn) {
            p.flags = 0;
            p.count = 1;
        }
    }
    let mut zones = [buddy::Zone::new("T", 0, 512)];
    buddy::release_zone(&mut zones[0], &mut m);
    assert_eq!(zones[0].free_pages, 512 - 16);
    let mut seen = 0usize;
    while let Some(pfn) = buddy::alloc_order(&mut zones, &mut m, 0, 0) {
        assert!(!(16..32).contains(&(pfn as usize)));
        seen += 1;
    }
    assert_eq!(seen, 512 - 16);
    // Freeing a reserved pfn is rejected.
    assert!(!buddy::free_page(&mut zones, &mut m, 20));
}

#[test]
fn buddy_high_order_split_and_double_free() {
    let mut m = [buddy::Page::EMPTY; 1024];
    let mut zones = zone_pair(&mut m);
    let a = buddy::alloc_order(&mut zones, &mut m, 1, 4).unwrap();
    assert_eq!(a % 16, 0);
    assert_eq!(zones[1].free_pages, 512 - 16);
    // Split leaves free buddies: order-4 allocs carve from the top.
    let b = buddy::alloc_order(&mut zones, &mut m, 1, 4).unwrap();
    assert_eq!(b, a + 16);
    // Interior/double frees rejected.
    assert!(!buddy::free_page(&mut zones, &mut m, a + 1)); // interior
    assert!(buddy::free_page(&mut zones, &mut m, a));
    assert!(!buddy::free_page(&mut zones, &mut m, a)); // double
    assert!(buddy::free_page(&mut zones, &mut m, b));
    assert_eq!(buddy::largest_free_order(&zones[1]), Some(9));
    // Zones never merge across the boundary: zone T stays order-9.
    assert_eq!(buddy::largest_free_order(&zones[0]), Some(9));
}

// ---- slub ---------------------------------------------------------------

/// kfree_core truncates pointers to their 4096-byte page frame, so the
/// host arena must really be page-aligned (a Vec<u8> is not). The bytes
/// exist only for size/alignment; slub never reads them.
#[repr(align(4096))]
#[derive(Clone, Copy)]
struct SlabPage(#[allow(dead_code)] [u8; slub::PAGE_SIZE]);

fn slab_arena(n: usize) -> Vec<SlabPage> {
    vec![SlabPage([0u8; slub::PAGE_SIZE]); n]
}

#[test]
fn slub_class_for_boundaries() {
    assert_eq!(slub::class_for(0), None);
    assert_eq!(slub::class_for(1), Some(0));
    assert_eq!(slub::class_for(8), Some(0));
    assert_eq!(slub::class_for(9), Some(1));
    assert_eq!(slub::class_for(2048), Some(slub::NCLASS - 1));
    assert_eq!(slub::class_for(2049), None);
}

#[test]
fn slub_reuse_canary_and_double_free() {
    let mut arena = slab_arena(3);
    let base = arena.as_mut_ptr() as *mut u8;
    let mut next_page = 0usize;
    let mut page_src = || {
        let p = if next_page < 3 {
            next_page += 1;
            unsafe { base.add((next_page - 1) * slub::PAGE_SIZE) }
        } else {
            core::ptr::null_mut()
        };
        p
    };
    let mut s = slub::Slub::EMPTY;
    let p = slub::kmalloc_core(&mut s, 100, &mut page_src);
    assert!(!p.is_null());
    // Write and free; the body must then hold the poison canary.
    unsafe {
        for k in 0..100 {
            *p.add(k) = 0xa1;
        }
    }
    assert!(slub::kfree_core(&mut s, p));
    let body = unsafe { core::slice::from_raw_parts(p.add(4), 96) };
    assert!(body.iter().all(|&b| b == slub::POISON));
    // LIFO reuse: same size, same object.
    let q = slub::kmalloc_core(&mut s, 100, &mut page_src);
    assert_eq!(q, p);
    // Double free rejected.
    assert!(slub::kfree_core(&mut s, q));
    assert!(!slub::kfree_core(&mut s, q));
    // Misaligned interior pointer and slab-header offset rejected.
    assert!(!slub::kfree_core(&mut s, unsafe { q.add(4) }));
    assert!(!slub::kfree_core(&mut s, base));
}

#[test]
fn slub_canary_detects_uaf_write() {
    let mut arena = slab_arena(1);
    let base = arena.as_mut_ptr() as *mut u8;
    let mut armed = false;
    let mut page_src = || {
        if armed {
            core::ptr::null_mut()
        } else {
            armed = true;
            base
        }
    };
    let mut s = slub::Slub::EMPTY;
    let p = slub::kmalloc_core(&mut s, 64, &mut page_src);
    assert!(!p.is_null());
    assert!(slub::kfree_core(&mut s, p));
    // Use-after-free write between free and realloc: canary broken.
    unsafe { *p.add(40) = 0x41 };
    assert!(slub::kmalloc_core(&mut s, 64, &mut page_src).is_null());
    // No class / zero size.
    assert!(slub::kmalloc_core(&mut s, 0, &mut page_src).is_null());
    assert!(slub::kmalloc_core(&mut s, 3000, &mut page_src).is_null());
    // A page that was never slabbed is rejected.
    let mut arena2 = slab_arena(1);
    assert!(!slub::kfree_core(&mut s, arena2.as_mut_ptr() as *mut u8));
}

#[test]
fn slub_fills_one_page_and_grows() {
    let mut arena = slab_arena(2);
    let base = arena.as_mut_ptr() as *mut u8;
    let mut next = 0usize;
    let mut page_src = || {
        let p = if next < 2 {
            next += 1;
            unsafe { base.add((next - 1) * slub::PAGE_SIZE) }
        } else {
            core::ptr::null_mut()
        };
        p
    };
    let mut s = slub::Slub::EMPTY;
    let size = 96usize;
    let per = (slub::PAGE_SIZE - slub::DATA_OFF) / size;
    let mut keep = Vec::new();
    for i in 0..per {
        let p = slub::kmalloc_core(&mut s, size, &mut page_src);
        assert!(!p.is_null());
        unsafe { *p = i as u8 };
        keep.push(p);
    }
    // First slab is now full: the next alloc grows onto page 2.
    let extra = slub::kmalloc_core(&mut s, size, &mut page_src);
    assert!(!extra.is_null());
    assert!(extra < base || extra >= unsafe { base.add(slub::PAGE_SIZE) });
    // Every kept object still reads its own index (no cross-object
    // stomping), then all free cleanly.
    for (i, &p) in keep.iter().enumerate() {
        assert_eq!(unsafe { *p }, i as u8);
        assert!(slub::kfree_core(&mut s, p));
    }
    assert!(slub::kfree_core(&mut s, extra));
}
