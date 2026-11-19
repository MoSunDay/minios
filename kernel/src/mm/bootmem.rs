//! Boot memory map: the loader's (Multiboot mmap) region list normalized
//! into usable/reserved tables. The pure core (sort/merge/clip) is
//! host-tested; the kernel glue parses the physical MBI once at boot.

pub const MAX_REGIONS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Usable,
    Reserved,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    pub base: u64,
    pub len: u64,
    pub kind: Kind,
}

/// Fixed-capacity sorted region table (no allocator at this boot stage).
#[derive(Clone, Copy)]
pub struct RegionTable {
    pub regions: [Region; MAX_REGIONS],
    pub count: usize,
}

impl RegionTable {
    pub const fn empty() -> Self {
        RegionTable {
            regions: [Region {
                base: 0,
                len: 0,
                kind: Kind::Usable,
            }; MAX_REGIONS],
            count: 0,
        }
    }

    pub fn push(&mut self, base: u64, len: u64, kind: Kind) -> bool {
        if self.count >= MAX_REGIONS || len == 0 {
            return false;
        }
        self.regions[self.count] = Region { base, len, kind };
        self.count += 1;
        true
    }

    pub fn iter(&self) -> core::slice::Iter<'_, Region> {
        self.regions[..self.count].iter()
    }

    /// Total length of regions of one kind.
    pub fn total_len(&self, kind: Kind) -> u64 {
        self.iter().filter(|r| r.kind == kind).map(|r| r.len).sum()
    }
}

/// Sort by base and coalesce: overlapping or touching same-kind ranges
/// merge; where usable and reserved overlap, reserved wins over the whole
/// union (conservative: never hand reserved bytes to the page allocator).
pub fn normalize(t: &mut RegionTable) {
    let n = t.count;
    for i in 1..n {
        let r = t.regions[i];
        let mut j = i;
        while j > 0 && t.regions[j - 1].base > r.base {
            t.regions[j] = t.regions[j - 1];
            j -= 1;
        }
        t.regions[j] = r;
    }
    let mut out = 0usize;
    for i in 0..n {
        let r = t.regions[i];
        if out == 0 {
            t.regions[out] = r;
            out += 1;
            continue;
        }
        let cur = t.regions[out - 1];
        let cur_end = cur.base + cur.len;
        let overlaps = r.base < cur_end;
        let touches_same = r.base == cur_end && cur.kind == r.kind;
        if overlaps || touches_same {
            let end = cur_end.max(r.base + r.len);
            t.regions[out - 1] = Region {
                base: cur.base,
                len: end - cur.base,
                kind: if cur.kind == r.kind {
                    cur.kind
                } else {
                    Kind::Reserved
                },
            };
        } else {
            t.regions[out] = r;
            out += 1;
        }
    }
    t.count = out;
}

/// Keep only Usable regions, clip to [floor, top), page-align both edges,
/// drop empties. Input must already be normalized (sorted, merged).
pub fn clip_usable(src: &RegionTable, floor: u64, top: u64, page: u64) -> RegionTable {
    let mut out = RegionTable::empty();
    for r in src.iter() {
        if r.kind != Kind::Usable {
            continue;
        }
        let b = r.base.max(floor);
        let e = (r.base + r.len).min(top);
        if e <= b {
            continue;
        }
        let b = b.div_ceil(page) * page;
        let e = (e / page) * page;
        if e > b {
            out.push(b, e - b, Kind::Usable);
        }
    }
    out
}

/// Highest probed physical address (any kind): what the direct map and
/// the mem_map have to cover.
pub fn probed_top(t: &RegionTable) -> u64 {
    t.iter().map(|r| r.base + r.len).max().unwrap_or(0)
}

// ---- kernel glue (physical MBI -> tables) ------------------------------
// Kernel-only: the host-test build includes this file by #[path] and has
// no crate::multiboot, so the glue is gated to the none target.

#[cfg(target_os = "none")]
use crate::multiboot;

#[cfg(target_os = "none")]
static mut RAW: RegionTable = RegionTable::empty();
#[cfg(target_os = "none")]
static mut USABLE: RegionTable = RegionTable::empty();
#[cfg(target_os = "none")]
static mut PROBED_TOP: usize = 0;

#[cfg(target_os = "none")]
fn raw_mut() -> &'static mut RegionTable {
    // SAFETY: boot context, single CPU; raw-pointer access avoids
    // taking a reference to the static directly.
    unsafe { &mut *core::ptr::addr_of_mut!(RAW) }
}

#[cfg(target_os = "none")]
fn usable_mut() -> &'static mut RegionTable {
    // SAFETY: see raw_mut.
    unsafe { &mut *core::ptr::addr_of_mut!(USABLE) }
}

/// Parse the loader map once: RAW keeps every probed region (usable and
/// reserved, normalized), USABLE is the allocator seed clipped to
/// [floor_pa, cap_pa) page-aligned, PROBED_TOP bounds the direct map.
#[cfg(target_os = "none")]
pub fn init(mbi_phys: usize, floor_pa: usize, cap_pa: usize) {
    multiboot::for_each_entry(mbi_phys, |base, len, mtype| {
        raw_mut().push(
            base,
            len,
            if mtype == multiboot::MMAP_AVAILABLE {
                Kind::Usable
            } else {
                Kind::Reserved
            },
        );
    });
    normalize(raw_mut());
    *usable_mut() = clip_usable(raw_mut(), floor_pa as u64, cap_pa as u64, 4096);
    // SAFETY: boot-context write.
    unsafe {
        PROBED_TOP = (probed_top(raw_mut()) as usize).min(cap_pa);
    }
}

/// The allocator seed: usable, kernel-end-clipped, page-aligned regions.
#[cfg(target_os = "none")]
pub fn usable() -> &'static RegionTable {
    // SAFETY: written once at boot before any reader.
    unsafe { &*core::ptr::addr_of!(USABLE) }
}

/// Every probed region (any kind), normalized.
#[cfg(target_os = "none")]
pub fn raw() -> &'static RegionTable {
    // SAFETY: written once at boot before any reader.
    unsafe { &*core::ptr::addr_of!(RAW) }
}

/// Mapped/managed physical top (probed top capped at the architecture cap).
#[cfg(target_os = "none")]
pub fn probed_top_pa() -> usize {
    // SAFETY: written once at boot before any reader.
    unsafe { PROBED_TOP }
}

/// True when [pa, pa+len) lies wholly inside one usable region.
#[cfg(target_os = "none")]
pub fn in_usable(pa: usize, len: usize) -> bool {
    let a = pa as u64;
    let l = len as u64;
    usable()
        .iter()
        .any(|r| r.kind == Kind::Usable && a >= r.base && a - r.base + l <= r.len)
}
