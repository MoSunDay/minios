//! pagecache (m4r-s3a, 23 §2): the page-cache pure core — per-inode
//! address spaces, an index-ordered page tree, and the global
//! inactive-LRU with clean-page reclaim.
//!
//! Structure mirroring: one `AddressSpace` per file inode (the
//! `i_data` embedded in InodeCommon, `i_mapping` points at it — the
//! Linux shape), pages keyed by index inside it, one engine
//! (`PageCache`) owning the descriptor pool, the idle LRU and the
//! counters. The core is allocation-free: the caller supplies the
//! descriptor pool at init and the page frames at insert; frames go
//! back through the `free_data` owner callback (kernel: kfree; host
//! tests: None/leak), so the same code runs host-tested and in the
//! kernel (the dcache discipline).
//!
//! Index structure: an index-ordered intrusive list per mapping — the
//! M4 tailoring of the xarray (xv6fs caps files at MAXFILE=140
//! 512-byte blocks ≈ 18 pages, so O(n) scans are honest; a real radix
//! arrives only if file scale grows — recorded as a divergence in
//! ADR-0024, the dcache linear-scan precedent).
//!
//! Reclaim (23 §2): only idle (ref == 0) AND clean pages leave the
//! cache; dirty pages survive until writeback (s5) — dropping data in
//! the name of reclaim is forbidden. `insert` self-reclaims from the
//! LRU when the descriptor pool runs dry (pool pressure is the
//! evict-clean driver).

pub mod lru;
pub mod page;
pub mod tree;

pub use page::{PageCacheStats, PageDesc};
pub use tree::PageCache;

use super::ops::AddressSpaceOps;

/// The per-inode page cache (Linux `struct address_space`): the page
/// tree root plus the ops vtable and the owning-inode back pointer.
/// Embedded in InodeCommon (`i_data`); `i_mapping` points here.
#[repr(C)]
pub struct AddressSpace {
    /// The fs address-space vtable (readpage now; write family s5).
    pub a_ops: *const AddressSpaceOps,
    /// Owning inode (opaque back pointer; the fs recovers its object).
    pub host: *mut u8,
    /// Index-ordered page list (see the module doc for the tailoring).
    pub root: *mut PageDesc,
    /// Resident pages in this mapping.
    pub nrpages: u32,
}

impl AddressSpace {
    pub const EMPTY: AddressSpace = AddressSpace {
        a_ops: core::ptr::null(),
        host: core::ptr::null_mut(),
        root: core::ptr::null_mut(),
        nrpages: 0,
    };
}
