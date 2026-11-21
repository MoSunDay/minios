//! dentry.rs: the dentry object, cache stats, and rename outcome
//! (dcache data model, M4 s1). The tree/LRU mechanics live in
//! dcache.rs; this split keeps both under the file-line budget.

use super::ops::{InodeCommon, SuperBlock};

/// Linux NAME_MAX (255) + NUL slot.
pub const NAME_MAX: usize = 255;

/// Cached lookup miss.
pub const D_NEGATIVE: u32 = 0x1;
/// A mount sits on this dentry (s2 mount registry sets it; rename and
/// eviction must refuse it).
pub const D_MOUNTED: u32 = 0x2;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Dentry {
    pub(super) d_name: [u8; NAME_MAX + 1],
    /// Name length in bytes (d_name[len] == 0 kept for debug prints).
    pub(super) d_len: u16,
    pub(super) d_flags: u32,
    /// References held by path walks / open files / mounts. 0 == idle,
    /// which means "on the LRU".
    pub(super) d_ref: u32,
    pub(super) d_parent: *mut Dentry,
    /// Head of this entry's own child list (Linux keeps d_subdirs
    /// separate from the sibling links — so do we).
    pub(super) d_first_child: *mut Dentry,
    /// Sibling chain within the parent (hlist: pprev is
    /// &parent->d_first_child or &prev->d_next; null when detached).
    pub(super) d_next: *mut Dentry,
    pub(super) d_pprev: *mut *mut Dentry,
    /// LRU links (idle entries only; membership tracked by d_on_lru).
    pub(super) d_lru_next: *mut Dentry,
    pub(super) d_lru_prev: *mut Dentry,
    pub(super) d_on_lru: bool,
    /// Null iff D_NEGATIVE.
    pub(super) d_inode: *mut InodeCommon,
    pub(super) d_sb: *mut SuperBlock,
}

impl Dentry {
    pub const fn empty() -> Dentry {
        Dentry {
            d_name: [0; NAME_MAX + 1],
            d_len: 0,
            d_flags: 0,
            d_ref: 0,
            d_parent: core::ptr::null_mut(),
            d_first_child: core::ptr::null_mut(),
            d_next: core::ptr::null_mut(),
            d_pprev: core::ptr::null_mut(),
            d_lru_next: core::ptr::null_mut(),
            d_lru_prev: core::ptr::null_mut(),
            d_on_lru: false,
            d_inode: core::ptr::null_mut(),
            d_sb: core::ptr::null_mut(),
        }
    }

    #[inline]
    pub fn name(&self) -> &[u8] {
        &self.d_name[..self.d_len as usize]
    }

    #[inline]
    pub fn is_negative(&self) -> bool {
        self.d_flags & D_NEGATIVE != 0
    }

    #[inline]
    pub fn inode(&self) -> *mut InodeCommon {
        self.d_inode
    }

    #[inline]
    pub fn parent(&self) -> *mut Dentry {
        self.d_parent
    }

    #[inline]
    pub fn sb(&self) -> *mut SuperBlock {
        self.d_sb
    }

    #[inline]
    pub fn is_mounted(&self) -> bool {
        self.d_flags & D_MOUNTED != 0
    }

    #[inline]
    pub fn refcount(&self) -> u32 {
        self.d_ref
    }
}

/// Observable counters (rules/01: m4suite cases anchor on these
/// numbers, never on timing windows).
#[derive(Clone, Copy)]
pub struct DcacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub neg_hits: u64,
    pub inserts: u64,
    pub evicts: u64,
    pub renames: u64,
    pub neg_entries: u32,
    pub live: u32,
    pub lru_len: u32,
}

impl DcacheStats {
    /// Const zero (Dcache::new is a const fn).
    pub const ZERO: DcacheStats = DcacheStats {
        lookups: 0,
        hits: 0,
        neg_hits: 0,
        inserts: 0,
        evicts: 0,
        renames: 0,
        neg_entries: 0,
        live: 0,
        lru_len: 0,
    };
}

/// What a rename found at the target name. For `Replaced` the dentry
/// is detached and carries one reference for the caller (its dput
/// parks it on the LRU); `inode` is null when the occupant was a
/// negative entry, else the displaced inode the caller iput()s.
#[derive(Debug)]
pub enum RenameOutcome {
    Moved,
    Replaced {
        dentry: *mut Dentry,
        inode: *mut InodeCommon,
    },
    /// Same dir + name (rename("a","a")): POSIX no-op success.
    NoOp,
}
