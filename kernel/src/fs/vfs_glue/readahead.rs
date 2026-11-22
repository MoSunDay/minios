//! readahead.rs (m4r-s4a): the sequential-detection policy for the
//! minimal M4 readahead window — 23 §2's "顺序窗口最小算法" with a
//! two-page window (双页窗起步); the full adaptive window algorithm
//! stays Parity per 23 §1. Pure state + decision functions only: the
//! fill machinery (locks, a_ops dispatch, frames) lives in pcache.rs.
//!
//! Determinism: one global hint {mapping slot, wrapper epoch, last
//! index}. A page is sequential when it extends the same mapping's
//! (epoch-guarded) run; readahead then rides the DEMAND MISS of a
//! sequential page (hits never trigger fills — a fully cached stream
//! must stay pure). The epoch guard kills the recycled-slot false
//! positive: a freed wrapper's slot address can be reborn under a
//! different file, but never with the same generation.

use crate::fs::vfs::pagecache::AddressSpace;

/// Pages filled ahead per sequential miss (双页窗).
pub const RA_WINDOW: u64 = 2;

/// The one-entry sequential hint (kernel static; host tests drive the
/// policy through the same functions).
pub struct RaHint {
    pub slot: *mut AddressSpace,
    pub epoch: u64,
    pub last: u64,
}

const NO_HINT: RaHint = RaHint {
    slot: core::ptr::null_mut(),
    epoch: 0,
    last: 0,
};

static mut RA_HINT: RaHint = NO_HINT;

/// Record that `index` of mapping `m` (wrapper epoch `epoch`) was just
/// read; returns whether the page extended a sequential run on the
/// SAME mapping generation (the trigger condition for the window).
pub fn note_page(m: *mut AddressSpace, epoch: u64, index: u64) -> bool {
    // SAFETY: single hint word; touched only from the read path.
    let h = unsafe { &core::ptr::addr_of!(RA_HINT).read() };
    let seq = h.slot == m && h.epoch == epoch && h.last.wrapping_add(1) == index;
    // SAFETY: same single word, owned by this read path.
    unsafe {
        core::ptr::addr_of_mut!(RA_HINT).write(RaHint {
            slot: m,
            epoch,
            last: index,
        });
    }
    seq
}

/// The fill window after a sequential miss at `index`: at most
/// RA_WINDOW pages, clamped at the file's last page (never past EOF).
pub fn window(index: u64, last_page: u64) -> core::ops::Range<u64> {
    let from = index.saturating_add(1);
    let to = core::cmp::min(index.saturating_add(RA_WINDOW), last_page);
    if to < from {
        return from..from;
    }
    from..to.saturating_add(1)
}

/// Reset the hint (suite hook: pin the policy state across cases).
pub fn reset() {
    // SAFETY: single hint word; boot/test context.
    unsafe {
        core::ptr::addr_of_mut!(RA_HINT).write(NO_HINT);
    }
}
