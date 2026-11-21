//! namei.rs: pure path-resolution logic (M4 s1, 23 §1).
//!
//! The s2 glue walks with locks around this core; everything here is
//! allocation-free and side-effect-free:
//! * `PathCursor` — component splitter over a byte slice (multiple
//!   slashes collapse, "."/".." pass to the walker, NAME_MAX enforced,
//!   trailing slash reported for directory-intent checks);
//! * `dac_allows` — the classic Unix owner/group/other permission-bit
//!   check including the root special cases (23 §1: single-user
//!   minimal credentials, full algorithms are Parity M12);
//! * `SymlinkBudget` — the ELOOP accounting (40 follows, Linux value).

use super::dentry::NAME_MAX;
use super::ops::InodeCommon;
use super::{imode_has_type, VfsErr, VfsResult, S_IFDIR};

/// Linux MAXSYMLINKS: follows beyond this are ELOOP.
pub const MAXSYMLINKS: u32 = 40;

/// MAY_* masks (Linux namei.h values).
pub const MAY_READ: u32 = 0x1;
pub const MAY_WRITE: u32 = 0x2;
pub const MAY_EXEC: u32 = 0x4;

/// Minimal single-user credentials (M4; full cred/LSM is M12 and must
/// not leak back into Core).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cred {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
}

impl Cred {
    /// The M4 default: uid 0 root, single group.
    pub const ROOT: Cred = Cred {
        uid: 0,
        gid: 0,
        euid: 0,
        egid: 0,
    };

    #[inline]
    pub fn is_root(&self) -> bool {
        self.euid == 0
    }
}

/// Pure component splitter. `next()` yields the next path component
/// ("." and ".." included — the walker resolves them) or None at the
/// end of the path; a component over NAME_MAX is `NameTooLong`.
#[derive(Debug)]
pub struct PathCursor<'a> {
    rest: &'a [u8],
    pub saw_trailing_slash: bool,
}

impl<'a> PathCursor<'a> {
    pub fn new(path: &'a [u8]) -> VfsResult<PathCursor<'a>> {
        if path.is_empty() {
            return Err(VfsErr::NoEnt); // POSIX ENOENT for ""
        }
        let trailing = path.len() > 1 && path[path.len() - 1] == b'/';
        Ok(PathCursor {
            rest: path,
            saw_trailing_slash: trailing,
        })
    }

    /// True when the path starts with '/' (absolute walk).
    pub fn is_absolute(path: &[u8]) -> bool {
        !path.is_empty() && path[0] == b'/'
    }

    /// Advance to the next component, skipping duplicate slashes.
    pub fn next(&mut self) -> Result<Option<&'a [u8]>, VfsErr> {
        let mut i = 0;
        while i < self.rest.len() && self.rest[i] == b'/' {
            i += 1;
        }
        self.rest = &self.rest[i..];
        if self.rest.is_empty() {
            return Ok(None);
        }
        let mut j = 0;
        while j < self.rest.len() && self.rest[j] != b'/' {
            j += 1;
        }
        let comp = &self.rest[..j];
        self.rest = &self.rest[j..];
        if comp.len() > NAME_MAX {
            return Err(VfsErr::NameTooLong);
        }
        Ok(Some(comp))
    }
}

/// The classic mode-bit DAC (namei.c inode_permission core): owner
/// bits when euid matches, else group bits when egid matches, else
/// other bits. Root: read/write always granted, exec needs at least
/// one x bit set on the inode (Linux generic_permission).
pub fn dac_allows(mode: u16, fuid: u32, fgid: u32, cred: &Cred, want: u32) -> bool {
    if cred.is_root() {
        if want & MAY_EXEC == 0 {
            return true;
        }
        // Root needs SOME x bit for exec.
        return mode & 0o111 != 0;
    }
    let shift = if cred.euid == fuid {
        6 // owner bits
    } else if cred.egid == fgid || cred.gid == fgid {
        3 // group bits
    } else {
        0 // other bits
    };
    let bits = ((mode >> shift) & 0o7) as u32;
    let want_bits = ((want & MAY_READ).min(1) << 2)
        | ((want & MAY_WRITE).min(1) << 1)
        | (want & MAY_EXEC).min(1);
    bits & want_bits == want_bits
}

/// Directory-search permission: must be a directory and searchable
/// (x bit) by cred. The walk's per-component gate.
pub fn may_search(dir: &InodeCommon, cred: &Cred) -> Result<(), VfsErr> {
    if !imode_has_type(dir.i_mode, S_IFDIR) {
        return Err(VfsErr::NotDir);
    }
    if dac_allows(dir.i_mode, dir.i_uid, dir.i_gid, cred, MAY_EXEC) {
        Ok(())
    } else {
        Err(VfsErr::Access)
    }
}

/// ELOOP accounting: each symlink follow consumes one unit; the 41st
/// is SymLoop (Linux: 40 follows allowed, the budget check happens
/// before the follow).
#[derive(Clone, Copy, Debug)]
pub struct SymlinkBudget {
    followed: u32,
}

impl SymlinkBudget {
    pub const fn new() -> SymlinkBudget {
        SymlinkBudget { followed: 0 }
    }

    /// Reserve one follow; Err once the budget is spent.
    pub fn spend(&mut self) -> Result<(), VfsErr> {
        if self.followed >= MAXSYMLINKS {
            return Err(VfsErr::SymLoop);
        }
        self.followed += 1;
        Ok(())
    }

    pub fn followed(&self) -> u32 {
        self.followed
    }
}
