//! SLUB-style kmalloc core over whole pages (size classes, free-object
//! poison canary, per-slab in-use bitmap). Pure and portable: the page
//! source is a closure, so host tests drive it from a plain buffer.
//! Kernel wiring (lock + buddy-backed pages) lives in mm::frame.
//!
//! M2 semantics: slab pages are carved into objects of one class, never
//! returned to the buddy allocator (no shrink); free objects are filled
//! with POISON and threaded through a freelist whose head words carry a
//! FREED_MAGIC guard, so use-after-free writes and double frees are
//! detectable instead of silent.

/// Class sizes in bytes (smallest fit wins; larger requests go to the
/// page allocator directly).
pub const CLASS_SIZES: [u16; 12] = [8, 16, 32, 64, 96, 128, 192, 256, 384, 512, 1024, 2048];
pub const NCLASS: usize = CLASS_SIZES.len();

pub const PAGE_SIZE: usize = 4096;
pub const SLAB_MAGIC: u64 = 0x51ab_51ab_51ab_51ab;
pub const POISON: u8 = 0x6b;
pub const FREED_MAGIC: u16 = 0xf7ee;
const NONE_OFF: u16 = 0xffff;

/// Per-slab header at offset 0 of each slab page (88 bytes, 8-aligned).
#[repr(C)]
pub struct SlabHead {
    pub magic: u64,
    /// Next slab page of the same class (0 = list end).
    pub next_page: u64,
    pub size: u16,
    /// Offset (within the page) of the first free object.
    pub free_off: u16,
    pub inuse: u16,
    pub _pad: u16,
    /// In-use bitmap (one bit per object; bit i set = allocated).
    pub bitmap: [u8; 64],
}

/// DATA_OFF: objects start here (8-aligned).
pub const DATA_OFF: usize = 88;

pub struct Slub {
    /// Slab page list heads per class (direct-map/kernel VA, 0 = empty).
    pub slab_pages: [u64; NCLASS],
}

impl Slub {
    pub const EMPTY: Slub = Slub {
        slab_pages: [0; NCLASS],
    };
}

pub fn class_for(size: usize) -> Option<usize> {
    if size == 0 {
        return None;
    }
    CLASS_SIZES.iter().position(|&c| c as usize >= size)
}

fn nobj(size: u16) -> u16 {
    ((PAGE_SIZE - DATA_OFF) / size as usize) as u16
}

fn head_of(page: u64) -> &'static mut SlabHead {
    // SAFETY: page is a slab page owned by the allocator; the header is
    // the first 88 bytes and objects start above it.
    unsafe { &mut *(page as usize as *mut SlabHead) }
}

fn obj_ptr(page: u64, off: u16) -> *mut u8 {
    (page as usize + off as usize) as *mut u8
}

/// Wire one fresh zeroed page into class `ci` as a new slab: every object
/// starts free (poisoned, linked).
pub fn add_slab_page(s: &mut Slub, ci: usize, page: *mut u8) {
    let h = head_of(page as usize as u64);
    h.magic = SLAB_MAGIC;
    h.next_page = s.slab_pages[ci];
    h.size = CLASS_SIZES[ci];
    h.inuse = 0;
    h.bitmap = [0; 64];
    let size = h.size;
    let n = nobj(size);
    // Thread the freelist last-to-first so the first object pops first;
    // every object is born in the "freed" state (link + magic + poison).
    let mut next = NONE_OFF;
    let mut i = n;
    while i > 0 {
        i -= 1;
        let off = (DATA_OFF + i as usize * size as usize) as u16;
        let p = obj_ptr(page as usize as u64, off);
        // SAFETY: object lies inside the slab page we own.
        unsafe {
            core::ptr::write_volatile(p as *mut u16, next);
            core::ptr::write_volatile(p.add(2) as *mut u16, FREED_MAGIC);
            fill_poison(p.add(4), size as usize - 4);
        }
        next = off;
    }
    h.free_off = next;
    s.slab_pages[ci] = page as usize as u64;
}

/// Poison fill helper (memset without kernel imports).
fn fill_poison(dst: *mut u8, len: usize) {
    // SAFETY: caller-contracted writable range.
    unsafe {
        for k in 0..len {
            core::ptr::write_volatile(dst.add(k), POISON);
        }
    }
}

/// kmalloc core: smallest fitting class, first slab with a free object,
/// growing through the `page` closure when every slab is full. Returns
/// null on no-fit/OOM/corruption-detect.
pub fn kmalloc_core(s: &mut Slub, size: usize, page: &mut dyn FnMut() -> *mut u8) -> *mut u8 {
    let Some(ci) = class_for(size) else {
        return core::ptr::null_mut();
    };
    if s.slab_pages[ci] == 0 {
        let p = page();
        if p.is_null() {
            return core::ptr::null_mut();
        }
        add_slab_page(s, ci, p);
    }
    // Walk the class slab list for one with a free object.
    let mut slab = s.slab_pages[ci];
    loop {
        let h = head_of(slab);
        if h.free_off != NONE_OFF {
            break;
        }
        if h.next_page == 0 {
            let p = page();
            if p.is_null() {
                return core::ptr::null_mut();
            }
            add_slab_page(s, ci, p);
            slab = s.slab_pages[ci];
            break;
        }
        slab = h.next_page;
    }
    let h = head_of(slab);
    let off = h.free_off;
    let p = obj_ptr(slab, off);
    let size = h.size;
    // Canary: while free, bytes [2..4) hold FREED_MAGIC and [4..size)
    // hold POISON. Anything else is a use-after-free write.
    // SAFETY: object lies inside the slab page.
    unsafe {
        let magic = core::ptr::read_volatile(p.add(2) as *const u16);
        if magic != FREED_MAGIC {
            return core::ptr::null_mut();
        }
        for k in 4..size as usize {
            if core::ptr::read_volatile(p.add(k)) != POISON {
                return core::ptr::null_mut();
            }
        }
        let next = core::ptr::read_volatile(p as *const u16);
        h.free_off = next;
    }
    let idx = (off as usize - DATA_OFF) / size as usize;
    h.bitmap[idx / 8] |= 1 << (idx % 8);
    h.inuse += 1;
    p
}

/// kfree core: return one object to its slab. False on not-a-slab
/// pointer, misaligned interior pointer, or double free.
pub fn kfree_core(s: &mut Slub, ptr: *mut u8) -> bool {
    let page = (ptr as usize) & !(PAGE_SIZE - 1);
    let h = head_of(page as u64);
    // SAFETY: header fields read before any validation stay in-bounds of
    // the page; the magic check gates the rest.
    unsafe {
        if core::ptr::read_volatile(&h.magic) != SLAB_MAGIC {
            return false;
        }
    }
    let size = h.size as usize;
    let off = ptr as usize - page;
    if off < DATA_OFF || (off - DATA_OFF) % size != 0 {
        return false;
    }
    let idx = (off - DATA_OFF) / size;
    if idx >= nobj(h.size) as usize {
        return false;
    }
    if h.bitmap[idx / 8] & (1 << (idx % 8)) == 0 {
        return false; // double free
    }
    let p = ptr;
    // SAFETY: object lies inside its slab page.
    unsafe {
        core::ptr::write_volatile(p as *mut u16, h.free_off);
        core::ptr::write_volatile(p.add(2) as *mut u16, FREED_MAGIC);
        fill_poison(p.add(4), size - 4);
    }
    h.free_off = (off as u16).min(NONE_OFF);
    h.bitmap[idx / 8] &= !(1 << (idx % 8));
    h.inuse -= 1;
    let _ = s;
    true
}
