//! Anonymous VMA bookkeeping core (Linux 2.6 mmap_region semantics, M2
//! subset): a sorted, fixed-capacity array of [start, end) regions with
//! prot/flags, supporting insert-with-merge, lookup, boundary split,
//! range removal and re-protect. Pure and portable (host-testable);
//! the owning address space (mm/vm.rs) drives the page tables.

/// mmap(2) protections (Linux x86-64 values).
#[allow(dead_code)] // s4 mm wiring consumes the VMA core
pub const PROT_NONE: u32 = 0;
#[allow(dead_code)] // s4 mm wiring
pub const PROT_R: u32 = 1;
#[allow(dead_code)] // s4 mm wiring
pub const PROT_W: u32 = 1 << 1;
#[allow(dead_code)] // s4 mm wiring
pub const PROT_X: u32 = 1 << 2;
#[allow(dead_code)] // s4 mm wiring
pub const PROT_ALL: u32 = PROT_R | PROT_W | PROT_X;
/// Common combinations (text/rodata/data).
#[allow(dead_code)] // s4 mm wiring
pub const PROT_RW: u32 = PROT_R | PROT_W;
#[allow(dead_code)] // s4 mm wiring
pub const PROT_RWX: u32 = PROT_R | PROT_W | PROT_X;

/// Anonymous mapping (mmap MAP_ANONYMOUS): zero-filled, no file backing
/// until M4.
#[allow(dead_code)] // s4 mm wiring
pub const VMA_ANON: u32 = 1 << 0;
/// Process stack: may grow down on faults below `start` (s5).
#[allow(dead_code)] // s4 mm wiring
pub const VMA_STACK: u32 = 1 << 1;
/// The brk heap: both ends may carry the byte-precise break and stay
/// unaligned; every other VMA is page-granular.
#[allow(dead_code)] // s4 mm wiring
pub const VMA_HEAP: u32 = 1 << 2;

const VMA_FLAGS_ALL: u32 = VMA_ANON | VMA_STACK | VMA_HEAP;

/// Fixed VMA capacity: NPROC is small and UP; a tree structure stays a
/// Parity concern (M2 design decision, plans/linux26/22 §4).
#[allow(dead_code)] // s4 mm wiring
pub const NVMA: usize = 32;

const PGSIZE: usize = 4096;

#[allow(dead_code)] // s4 mm wiring consumes the VMA core
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vma {
    /// Inclusive start.
    pub start: usize,
    /// Exclusive end.
    pub end: usize,
    /// PROT_* bit set.
    pub prot: u32,
    /// VMA_* bit set.
    pub flags: u32,
}

#[allow(dead_code)] // s4 mm wiring consumes the VMA core
impl Vma {
    #[inline]
    pub fn contains(&self, addr: usize) -> bool {
        self.start <= addr && addr < self.end
    }

    #[inline]
    fn same_attrs(&self, other: &Vma) -> bool {
        self.prot == other.prot && self.flags == other.flags
    }
}

const EMPTY: Vma = Vma {
    start: 0,
    end: 0,
    prot: 0,
    flags: 0,
};

/// Sorted-by-start VMA set. Invariants (maintained by every mutator):
/// vmas[0..count] ascending and non-overlapping; no two touching
/// neighbors share (prot, flags); count <= NVMA. Heap VMAs may be
/// byte-unaligned at either edge after splits (the brk is byte-exact).
#[allow(dead_code)] // s4 mm wiring consumes the VMA core
#[derive(Clone, Copy)]
pub struct VmaTable {
    pub vmas: [Vma; NVMA],
    pub count: usize,
}

#[allow(dead_code)] // s4 mm wiring consumes the VMA core
impl VmaTable {
    pub const fn empty() -> VmaTable {
        VmaTable {
            vmas: [EMPTY; NVMA],
            count: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Index of the first VMA whose end is past `addr` (insertion point).
    fn pos(&self, addr: usize) -> usize {
        let mut lo = 0usize;
        let mut hi = self.count;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.vmas[mid].end <= addr {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Index of the VMA covering `addr`, if any.
    pub fn find(&self, addr: usize) -> Option<usize> {
        let i = self.pos(addr);
        if i < self.count && self.vmas[i].contains(addr) {
            Some(i)
        } else {
            None
        }
    }

    /// Index of a VMA overlapping [start, end), if any.
    pub fn overlap(&self, start: usize, end: usize) -> Option<usize> {
        if end <= start {
            return None;
        }
        let i = self.pos(start);
        if i < self.count && self.vmas[i].start < end {
            Some(i)
        } else {
            None
        }
    }

    fn insert_idx(&mut self, i: usize, vma: Vma) {
        let mut j = self.count;
        while j > i {
            self.vmas[j] = self.vmas[j - 1];
            j -= 1;
        }
        self.vmas[i] = vma;
        self.count += 1;
    }

    fn drop_idx(&mut self, i: usize) {
        let mut j = i;
        while j + 1 < self.count {
            self.vmas[j] = self.vmas[j + 1];
            j += 1;
        }
        self.count -= 1;
        self.vmas[self.count] = EMPTY;
    }

    /// Insert `vma`, keeping the array sorted. Fails (false) on overlap,
    /// bad geometry, unknown bits, or a full table. Touching neighbors
    /// with identical (prot, flags) are merged.
    pub fn insert(&mut self, vma: Vma) -> bool {
        if vma.end <= vma.start {
            return false;
        }
        let heapish = vma.flags & VMA_HEAP != 0;
        if vma.start % PGSIZE != 0 && !heapish {
            return false;
        }
        if vma.end % PGSIZE != 0 && !heapish {
            return false;
        }
        if vma.prot & !PROT_ALL != 0 || vma.flags & !VMA_FLAGS_ALL != 0 {
            return false;
        }
        if self.count >= NVMA {
            return false;
        }
        let i = self.pos(vma.start);
        if i < self.count && self.vmas[i].start < vma.end {
            return false; // overlaps the right neighbor
        }
        if i > 0 && self.vmas[i - 1].end > vma.start {
            return false; // overlaps the left neighbor
        }
        self.insert_idx(i, vma);
        // Merge left first, then re-try the right side.
        let mut m = i;
        if m > 0 && self.vmas[m - 1].end == vma.start && self.vmas[m - 1].same_attrs(&vma) {
            self.vmas[m - 1].end = vma.end;
            self.drop_idx(m);
            m -= 1;
        }
        if m + 1 < self.count
            && self.vmas[m].end == self.vmas[m + 1].start
            && self.vmas[m].same_attrs(&self.vmas[m + 1])
        {
            self.vmas[m].end = self.vmas[m + 1].end;
            self.drop_idx(m + 1);
        }
        true
    }

    /// True when some VMA strictly straddles `addr` (a split would be
    /// needed to place a boundary there).
    fn straddles(&self, addr: usize) -> bool {
        match self.find(addr) {
            Some(i) => addr > self.vmas[i].start && addr < self.vmas[i].end,
            None => false,
        }
    }

    /// Place a boundary at `addr` (split the straddling VMA). The
    /// caller guarantees a free slot (see straddles); defensive false
    /// when it cannot fit.
    fn boundary(&mut self, addr: usize) -> bool {
        if let Some(i) = self.find(addr) {
            let vma = self.vmas[i];
            if addr > vma.start && addr < vma.end {
                if self.count >= NVMA {
                    return false;
                }
                self.vmas[i].end = addr;
                self.insert_idx(i + 1, Vma { start: addr, ..vma });
            }
        }
        true
    }

    /// Split the VMA covering `addr` at `addr` (no-op on edges/outside);
    /// false only when the split cannot fit (full table).
    pub fn split_at(&mut self, addr: usize) -> bool {
        self.boundary(addr)
    }

    /// Remove [start, end) from every overlapping VMA (munmap core):
    /// whole VMAs vanish, heads/tails are trimmed, a strictly-inside
    /// range splits its VMA in two. False (untouched) when a needed
    /// edge split does not fit.
    pub fn remove_range(&mut self, start: usize, end: usize) -> bool {
        if end <= start {
            return true;
        }
        // Transactional up front: the two edge splits must fit.
        let need = self.straddles(start) as usize + self.straddles(end) as usize;
        if self.count + need > NVMA {
            return false;
        }
        self.boundary(start);
        self.boundary(end);
        let mut i = 0usize;
        while i < self.count {
            let v = self.vmas[i];
            if v.start >= end {
                break;
            }
            if v.start >= start && v.end <= end {
                self.drop_idx(i);
            } else {
                i += 1;
            }
        }
        true
    }

    /// mprotect core: re-protect the fully covered range [start, end).
    /// Returns false (no change) when the range is not wholly inside
    /// mapped VMAs or the needed boundary splits do not fit.
    pub fn set_prot(&mut self, start: usize, end: usize, prot: u32) -> bool {
        if end <= start || prot & !PROT_ALL != 0 {
            return false;
        }
        // Read-only coverage pre-check: [start, end) must lie wholly
        // inside contiguous VMAs (mprotect on a hole is ENOMEM).
        let mut cover = start;
        let mut i = 0usize;
        let mut covered = false;
        while i < self.count {
            let v = self.vmas[i];
            if v.end <= start {
                i += 1;
                continue;
            }
            if v.start >= end {
                break;
            }
            if v.start > cover {
                return false;
            }
            cover = cover.max(v.end);
            if cover >= end {
                covered = true;
                break;
            }
            i += 1;
        }
        if !covered {
            return false;
        }
        // The two edge splits must fit before anything mutates.
        let need = self.straddles(start) as usize + self.straddles(end) as usize;
        if self.count + need > NVMA {
            return false;
        }
        self.boundary(start);
        self.boundary(end);
        let mut covered_end = start;
        let mut touched = false;
        let mut i = 0usize;
        while i < self.count {
            let v = self.vmas[i];
            if v.end <= start {
                i += 1;
                continue;
            }
            if v.start >= end {
                break;
            }
            if v.start != covered_end {
                return false; // hole inside the range
            }
            self.vmas[i].prot = prot;
            covered_end = v.end;
            touched = true;
            i += 1;
        }
        if !touched || covered_end < end {
            return false;
        }
        // Re-merge pieces that now match a touching neighbor.
        let mut i = 1usize;
        while i < self.count {
            if self.vmas[i - 1].end == self.vmas[i].start
                && self.vmas[i - 1].same_attrs(&self.vmas[i])
            {
                self.vmas[i - 1].end = self.vmas[i].end;
                self.drop_idx(i);
                continue;
            }
            i += 1;
        }
        true
    }

    /// Highest page-aligned placement for a mapping of `len` bytes in
    /// [floor, ceiling) that overlaps no VMA (top-down mmap_base
    /// semantics). None when no gap fits.
    pub fn find_area_topdown(&self, len: usize, floor: usize, ceiling: usize) -> Option<usize> {
        if len == 0 || len % PGSIZE != 0 || ceiling % PGSIZE != 0 {
            return None;
        }
        let mut gap_top = ceiling;
        let mut i = self.count;
        loop {
            let gap_bottom = if i == 0 { floor } else { self.vmas[i - 1].end };
            let bottom = gap_bottom.max(floor);
            let top = gap_top.min(ceiling);
            if let Some(addr) = top.checked_sub(len) {
                if top > bottom && addr >= bottom {
                    return Some(addr);
                }
            }
            if i == 0 {
                return None;
            }
            i -= 1;
            gap_top = self.vmas[i].start;
        }
    }
}
