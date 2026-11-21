//! vfs pure-core tests (m4r-s1): the path splitter, DAC matrix, symlink
//! budget, search gates, and the four-ops vtable dispatch — driving
//! the real kernel files (included by path; no test-only copy to
//! drift). dcache/icache state-machine tests live in dcachetests.rs;
//! the walk-with-locks and mount glue is s2 and tested at QEMU.

use crate::vfs::namei::{
    dac_allows, may_search, Cred, PathCursor, SymlinkBudget, MAY_EXEC, MAY_READ, MAY_WRITE,
};
use crate::vfs::ops::{check_permission, FileCommon, InodeCommon, InodeOps, SuperBlock};
use crate::vfs::{VfsErr, S_IFDIR, S_IFREG};

#[test]
fn path_split_basic() {
    let mut c = PathCursor::new(b"/a/bb/ccc").unwrap();
    assert_eq!(c.next().unwrap(), Some(&b"a"[..]));
    assert_eq!(c.next().unwrap(), Some(&b"bb"[..]));
    assert_eq!(c.next().unwrap(), Some(&b"ccc"[..]));
    assert_eq!(c.next().unwrap(), None);
    // Duplicate and trailing slashes collapse.
    let mut c = PathCursor::new(b"//x//y//").unwrap();
    assert!(c.saw_trailing_slash);
    assert_eq!(c.next().unwrap(), Some(&b"x"[..]));
    assert_eq!(c.next().unwrap(), Some(&b"y"[..]));
    assert_eq!(c.next().unwrap(), None);
    // Dot components pass through to the walker.
    let mut c = PathCursor::new(b"a/./..").unwrap();
    assert_eq!(c.next().unwrap(), Some(&b"a"[..]));
    assert_eq!(c.next().unwrap(), Some(&b"."[..]));
    assert_eq!(c.next().unwrap(), Some(&b".."[..]));
    assert_eq!(c.next().unwrap(), None);
    assert!(PathCursor::is_absolute(b"/x"));
    assert!(!PathCursor::is_absolute(b"x/"));
}

#[test]
fn path_split_errors() {
    assert_eq!(PathCursor::new(b"").unwrap_err(), VfsErr::NoEnt);
    let long = [b'x'; 256];
    let mut c = PathCursor::new(&long).unwrap();
    assert_eq!(c.next().unwrap_err(), VfsErr::NameTooLong);
    // "/" alone is a valid empty walk (root itself).
    let mut c = PathCursor::new(b"/").unwrap();
    assert_eq!(c.next().unwrap(), None);
}

#[test]
fn dac_owner_group_other_matrix() {
    let cred = Cred {
        uid: 1,
        gid: 1,
        euid: 1,
        egid: 1,
    };
    // Owner 0600: owner rw, group/other nothing.
    assert!(dac_allows(0o600, 1, 1, &cred, MAY_READ | MAY_WRITE));
    assert!(!dac_allows(0o600, 1, 1, &cred, MAY_EXEC));
    // Same file, other user, same group -> group bits (none).
    let other = Cred {
        uid: 2,
        gid: 1,
        euid: 2,
        egid: 1,
    };
    assert!(!dac_allows(0o600, 1, 1, &other, MAY_READ));
    // Group-readable 0o640: group read yes, write no.
    assert!(dac_allows(0o640, 1, 1, &other, MAY_READ));
    assert!(!dac_allows(0o640, 1, 1, &other, MAY_WRITE));
    // Other-user other-group falls to other bits 0o644 -> read.
    let stranger = Cred {
        uid: 3,
        gid: 3,
        euid: 3,
        egid: 3,
    };
    assert!(dac_allows(0o644, 1, 1, &stranger, MAY_READ));
    assert!(!dac_allows(0o644, 1, 1, &stranger, MAY_WRITE));
    // Owner bits win over group even when owner is more restrictive.
    let strict_owner = Cred {
        uid: 1,
        gid: 1,
        euid: 1,
        egid: 9,
    };
    assert!(!dac_allows(0o070, 1, 1, &strict_owner, MAY_WRITE));
    assert!(dac_allows(0o070, 1, 1, &other, MAY_WRITE));
    // egid membership checked as well as gid.
    let egid_member = Cred {
        uid: 5,
        gid: 5,
        euid: 5,
        egid: 1,
    };
    assert!(dac_allows(0o070, 1, 1, &egid_member, MAY_EXEC));
}

#[test]
fn dac_root_special_cases() {
    let root = Cred::ROOT;
    // Root reads/writes anything, even mode 0.
    assert!(dac_allows(0o000, 100, 100, &root, MAY_READ | MAY_WRITE));
    // Root exec needs at least one x bit.
    assert!(!dac_allows(0o666, 100, 100, &root, MAY_EXEC));
    assert!(!dac_allows(0o660, 100, 100, &root, MAY_EXEC));
    assert!(dac_allows(0o111, 100, 100, &root, MAY_EXEC));
}

#[test]
fn symlink_budget_eloop_at_40() {
    let mut b = SymlinkBudget::new();
    for _ in 0..40 {
        assert!(b.spend().is_ok());
    }
    assert_eq!(b.spend().unwrap_err(), VfsErr::SymLoop);
    assert_eq!(b.followed(), 40);
}

#[test]
fn may_search_dir_rules() {
    let cred = Cred {
        uid: 1,
        gid: 1,
        euid: 1,
        egid: 1,
    };
    let d = dir_ino(9);
    assert!(may_search(d, &cred).is_ok()); // 755 dir
    let mut locked = InodeCommon::empty();
    locked.i_mode = S_IFDIR | 0o600; // no x for others
    locked.i_uid = 7;
    assert_eq!(may_search(&locked, &cred).unwrap_err(), VfsErr::Access);
    locked.i_mode = S_IFREG | 0o755; // not a dir
    assert_eq!(may_search(&locked, &cred).unwrap_err(), VfsErr::NotDir);
}

fn dir_ino(ino: u32) -> &'static mut InodeCommon {
    let n = Box::leak(Box::new(InodeCommon::empty()));
    n.i_mode = S_IFDIR | 0o755;
    n.i_ino = ino;
    n.i_dev = 1;
    n
}

fn inode(mode: u16, ino: u32) -> *mut InodeCommon {
    let n = Box::leak(Box::new(InodeCommon::empty()));
    n.i_mode = mode;
    n.i_ino = ino;
    n.i_dev = 1;
    n
}

#[test]
fn ops_vtable_dispatch_and_generic_dac() {
    fn perm_override(_i: *mut InodeCommon, mask: u32, _c: &Cred) -> Result<(), VfsErr> {
        if mask & MAY_WRITE != 0 {
            Err(VfsErr::Access)
        } else {
            Ok(())
        }
    }
    let iops = InodeOps {
        lookup: |_d, _n, _l| Err(VfsErr::NoEnt),
        create: |_d, _n, _l, _m| Err(VfsErr::NotSup),
        link: |_i, _d, _n, _l| Err(VfsErr::NotSup),
        unlink: |_d, _n, _l| Err(VfsErr::NotSup),
        mkdir: |_d, _n, _l, _m| Err(VfsErr::NotSup),
        rmdir: |_d, _n, _l| Err(VfsErr::NotSup),
        rename: |_od, _on, _ol, _nd, _nn, _nl| Err(VfsErr::NotSup),
        readlink: |_i, _o, _c| Err(VfsErr::NotSup),
        symlink: |_d, _n, _l, _t, _tl| Err(VfsErr::NotSup),
        permission: Some(perm_override),
    };
    let n = unsafe { &mut *inode(S_IFREG | 0o666, 5) };
    n.i_op = &iops;
    let cred = Cred::ROOT;
    // Override answers even for root.
    assert_eq!(
        check_permission(n, MAY_WRITE, &cred).unwrap_err(),
        VfsErr::Access
    );
    assert!(check_permission(n, MAY_READ, &cred).is_ok());
    // No override: generic DAC decides (root reads anything).
    n.i_op = core::ptr::null();
    assert!(check_permission(n, MAY_READ, &cred).is_ok());
    assert!(!check_permission(n, MAY_EXEC, &cred).is_ok());
    // FileCommon shape compiles and initializes.
    let f = FileCommon {
        f_count: 1,
        f_mode: crate::vfs::ops::FMODE_READ,
        f_op: core::ptr::null(),
        f_pos: 0,
        f_inode: n as *mut InodeCommon,
        f_private: core::ptr::null_mut(),
    };
    assert_eq!(f.f_pos, 0);
    let _sb = SuperBlock {
        s_dev: 1,
        s_op: core::ptr::null(),
        s_root: core::ptr::null_mut(),
        s_private: core::ptr::null_mut(),
        s_flags: 0,
    };
}
