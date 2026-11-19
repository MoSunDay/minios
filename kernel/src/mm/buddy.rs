//! Buddy page allocator core (Linux zone/buddy semantics, orders 0..=10).
//! Pure and portable: the caller owns the mem_map array and the zone
//! table; free-block linkage lives in the mem_map, never in the pages.
//! The kernel wiring (static mem_map, zones, lock) lives in mm::frame.

pub const MAX_ORDER: usize = 10;
/// Sentinel pfn for "no link".
pub const NOPFN: u32 = u32::MAX;
/// This page is inside a free buddy block (not allocated out).
pub const PG_FREE: u8 = 1 << 0;
/// Never hand this page to the allocator (boot/kernel/reserved/firmware).
pub const PG_RESERVED: u8 = 1 << 1;
/// order value carried by interior pages of a free block.
pub const ORDER_INTERIOR: u8 = 0xff;

#[derive(Clone, Copy)]
pub struct Page {
    /// Free-list links (pfn) while the page heads a free block.
    pub next: u32,
    pub prev: u32,
    /// Free block order when heading one; 0 when allocated; 0xff inside
    /// a free block.
    pub order: u8,
    pub flags: u8,
    /// Allocation refcount: 1 while allocated (order-0 units only in M2).
    pub count: u16,
}

impl Page {
    pub const EMPTY: Page = Page {
        next: NOPFN,
        prev: NOPFN,
        order: 0,
        flags: PG_RESERVED,
        count: 0,
    };
}

pub struct Zone {
    pub name: &'static str,
    pub base_pfn: u32,
    pub top_pfn: u32,
    /// Free-list head pfn per order.
    pub free_head: [u32; MAX_ORDER + 1],
    /// Free pages counted in order-0 units.
    pub free_pages: usize,
}

impl Zone {
    pub const fn new(name: &'static str, base_pfn: u32, top_pfn: u32) -> Zone {
        Zone {
            name,
            base_pfn,
            top_pfn,
            free_head: [NOPFN; MAX_ORDER + 1],
            free_pages: 0,
        }
    }
}

fn list_push(z: &mut Zone, m: &mut [Page], pfn: u32, order: u8) {
    let head = z.free_head[order as usize];
    m[pfn as usize].order = order;
    m[pfn as usize].prev = NOPFN;
    m[pfn as usize].next = head;
    if head != NOPFN {
        m[head as usize].prev = pfn;
    }
    z.free_head[order as usize] = pfn;
    z.free_pages += 1usize << order;
}

fn list_del(z: &mut Zone, m: &mut [Page], pfn: u32, order: u8) {
    let (prev, next) = {
        let pg = &mut m[pfn as usize];
        (pg.prev, pg.next)
    };
    if prev != NOPFN {
        m[prev as usize].next = next;
    } else {
        z.free_head[order as usize] = next;
    }
    if next != NOPFN {
        m[next as usize].prev = prev;
    }
    m[pfn as usize].next = NOPFN;
    m[pfn as usize].prev = NOPFN;
    z.free_pages -= 1usize << order;
}

/// Mark one whole free block [pfn, pfn+2^order) free in the mem_map.
fn mark_block(m: &mut [Page], pfn: u32, order: u8) {
    for k in 0..(1usize << order) {
        let pg = &mut m[pfn as usize + k];
        pg.flags |= PG_FREE;
        pg.count = 0;
        pg.order = if k == 0 { order } else { ORDER_INTERIOR };
    }
}

/// Free one validated page into zone `z`, buddy-merging upward while the
/// symmetric partner heads a free block of the same order.
fn free_merged(z: &mut Zone, m: &mut [Page], pfn: u32) {
    let mut pfn = pfn;
    // Order-0 singles merge from 0; block heads carry their alloc order.
    let mut order: u8 = m[pfn as usize].order;
    while (order as usize) < MAX_ORDER {
        let buddy = pfn ^ (1 << order);
        if buddy < z.base_pfn || buddy >= z.top_pfn {
            break;
        }
        let b = m[buddy as usize];
        if b.flags & PG_FREE == 0 || b.order != order || b.count != 0 {
            break;
        }
        list_del(z, m, buddy, order);
        pfn = pfn.min(buddy);
        order += 1;
    }
    mark_block(m, pfn, order);
    list_push(z, m, pfn, order);
}

/// Allocate one 2^order-page block from zone `zone_idx`, splitting higher
/// orders down; returns the head pfn. The returned block's pages are
/// marked allocated (count 1 on the head, PG_FREE clear throughout).
pub fn alloc_order(zones: &mut [Zone], m: &mut [Page], zone_idx: usize, order: u8) -> Option<u32> {
    let o = order as usize;
    if o > MAX_ORDER {
        return None;
    }
    let z = &mut zones[zone_idx];
    let mut j = o;
    while j <= MAX_ORDER && z.free_head[j] == NOPFN {
        j += 1;
    }
    if j > MAX_ORDER {
        return None;
    }
    let pfn = z.free_head[j];
    let j = j as u8;
    list_del(z, m, pfn, j);
    // Split j down to o: at each step the high half goes back as a free
    // block, the low half keeps splitting.
    let mut cur = j as usize;
    while cur > o {
        cur -= 1;
        let half = pfn + (1u32 << cur);
        mark_block(m, half, cur as u8);
        list_push(z, m, half, cur as u8);
    }
    for k in 0..(1usize << o) {
        let pg = &mut m[pfn as usize + k];
        pg.flags &= !PG_FREE;
        pg.order = 0;
        pg.count = 0;
    }
    // Head keeps count 1 (the free() ticket) and its block order so
    // free_page can release the whole block, not just the first page.
    m[pfn as usize].count = 1;
    m[pfn as usize].order = o as u8;
    Some(pfn)
}

/// Free one page (order-0 units; the head must have been handed out by
/// alloc_order and not double-freed). Returns false — with the state
/// untouched — on any validation failure.
pub fn free_page(zones: &mut [Zone], m: &mut [Page], pfn: u32) -> bool {
    let Some(zi) = zone_of(zones, pfn) else {
        return false;
    };
    let pg = m[pfn as usize];
    if pg.flags & PG_RESERVED != 0 || pg.flags & PG_FREE != 0 || pg.count != 1 {
        return false;
    }
    free_merged(&mut zones[zi], m, pfn);
    true
}

/// Boot seeding: every page in the zone still marked "allocated by boot"
/// (count == 1, not reserved) is freed in ascending pfn order, so the
/// incremental merge rebuilds maximal-order blocks (free_all_bootmem).
pub fn release_zone(z: &mut Zone, m: &mut [Page]) {
    for pfn in z.base_pfn..z.top_pfn {
        let pg = m[pfn as usize];
        if pg.flags & PG_RESERVED != 0 || pg.flags & PG_FREE != 0 || pg.count != 1 {
            continue;
        }
        free_merged(z, m, pfn);
    }
}

/// Largest order with a free block in this zone (merge proof).
pub fn largest_free_order(z: &Zone) -> Option<u8> {
    (0..=MAX_ORDER)
        .rev()
        .find(|&o| z.free_head[o] != NOPFN)
        .map(|o| o as u8)
}

/// Zone index owning this pfn.
pub fn zone_of(zones: &[Zone], pfn: u32) -> Option<usize> {
    zones
        .iter()
        .position(|z| pfn >= z.base_pfn && pfn < z.top_pfn)
}
