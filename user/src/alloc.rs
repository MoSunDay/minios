//! brk-only global allocator (plans/linux26/04 §3): grows the break,
//! never shrinks; deallocation is a no-op in M1R (recycling waits for the
//! M2 memory manager). Alignment above 16 is unsupported (returns null).

use crate::sys;
use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;

pub struct BrkAlloc {
    break_hint: Cell<u64>,
}

pub const BRK_ALLOC: BrkAlloc = BrkAlloc {
    break_hint: Cell::new(0),
};

impl BrkAlloc {
    fn current_break(&self) -> u64 {
        let cached = self.break_hint.get();
        if cached != 0 {
            return cached;
        }
        match sys::brk(0) {
            Ok(b) => {
                self.break_hint.set(b);
                b
            }
            Err(_) => 0,
        }
    }
}

unsafe impl GlobalAlloc for BrkAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align() as u64;
        if align > 16 {
            return core::ptr::null_mut();
        }
        let cur = self.current_break();
        if cur == 0 {
            return core::ptr::null_mut();
        }
        let aligned = (cur + (align - 1)) & !(align - 1);
        let want = aligned + layout.size() as u64;
        // Page-rounded growth keeps the kernel's page mapping simple.
        let want = (want + 0xfff) & !0xfff;
        match sys::brk(want) {
            Ok(b) if b == want => {
                self.break_hint.set(want);
                aligned as *mut u8
            }
            _ => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // M1R: brk never shrinks (04 §3: no recycling in the first cut).
    }
}
