//! fs mutation wave-1 cases: create EEXIST (with the failed-create
//! refcount balance — the s2b leak regression pin), directory dots,
//! link's two-names-one-inode semantics, ENOTEMPTY (with the
//! dirlookup-ref balance — the s2b NotEmpty fix pin), and
//! inode-table pressure beyond NINODE. Absolute `/`-level `m4`
//! names; every case cleans up and closes with a slot-ledger check
//! against its own baseline.

use super::{drained_slots, with_inode};
use crate::fs::vfs::VfsErr;
use crate::fs::vfs_glue::{kern_create, kern_link, kern_namei, kern_unlink, CreateKind};
use crate::fs::xv6fs::inode::iput;
use crate::fs::xv6fs::layout::{ROOTINO, T_FILE};

/// The exclusive-create rule: dir-over-dir and file-over-dir both
/// fail EEXIST, the failed creates pin no table slots, and teardown
/// returns the ledger to base.
pub fn case_fs_create_eexist() -> bool {
    let dir = b"/m4ce0";
    let base = drained_slots();
    let ip = match kern_create(dir, CreateKind::Dir) {
        Ok(ip) => ip,
        Err(_) => return false,
    };
    // SAFETY: balancing put for the created reference (the wrapper
    // keeps exactly one slot pinned below baseline).
    unsafe { iput(ip) };
    let held = drained_slots();
    let e1 = matches!(kern_create(dir, CreateKind::Dir), Err(VfsErr::Exist));
    let e2 = matches!(kern_create(dir, CreateKind::File), Err(VfsErr::Exist));
    // The s2b leak pin: the EEXIST dirlookups must not pin slots.
    let balanced = drained_slots() == held;
    let gone = kern_unlink(dir).is_ok();
    held + 1 == base && e1 && e2 && balanced && gone && drained_slots() == base
}

/// "." and ".." resolve through the dentry tree — self and parent
/// (the root) — matching the on-disk dot dirents mkdir wrote.
pub fn case_fs_dir_dots() -> bool {
    let dir = b"/m4dd0";
    let base = drained_slots();
    let dinum = match kern_create(dir, CreateKind::Dir) {
        Ok(ip) => {
            // SAFETY: created reference; inum is stable.
            let inum = unsafe { (*ip).inum };
            // SAFETY: balancing put.
            unsafe { iput(ip) };
            inum
        }
        Err(_) => return false,
    };
    let dot = with_inode(b"/m4dd0/.", |ip| ip.inum == dinum);
    let dotdot = with_inode(b"/m4dd0/..", |ip| ip.inum == ROOTINO);
    let gone = kern_unlink(dir).is_ok();
    dot && dotdot && gone && drained_slots() == base
}

/// link(2): two names, one inode; removing either name keeps the
/// other resolvable; only the last unlink retires the inode, and the
/// retired slot returns only after the zombie drain.
pub fn case_fs_link_two_names() -> bool {
    let a = b"/m4lk0a";
    let b = b"/m4lk0b";
    let base = drained_slots();
    let inum = match kern_create(a, CreateKind::File) {
        Ok(ip) => {
            // SAFETY: created reference.
            let inum = unsafe { (*ip).inum };
            // SAFETY: balancing put.
            unsafe { iput(ip) };
            inum
        }
        Err(_) => return false,
    };
    let linked = kern_link(a, b).is_ok();
    let same = with_inode(a, |ip| ip.inum == inum) && with_inode(b, |ip| ip.inum == inum);
    // Kill one name: the survivor still resolves.
    let half = kern_unlink(a).is_ok() && with_inode(b, |ip| ip.inum == inum);
    // Kill the last name: miss, and the slots settle after the drain.
    let last = kern_unlink(b).is_ok() && matches!(kern_namei(b), Err(VfsErr::NoEnt));
    linked && same && half && last && drained_slots() == base
}

/// unlink of a non-empty directory is ENOTEMPTY and pins no slots
/// (the adapter's dirlookup ref goes back on the error path — the
/// s2b NotEmpty fix pin).
pub fn case_fs_unlink_notempty() -> bool {
    let dir = b"/m4ne0";
    let kid = b"/m4ne0/m4ne0k";
    let base = drained_slots();
    let made = match kern_create(dir, CreateKind::Dir) {
        Ok(ip) => {
            // SAFETY: balancing put for the created reference.
            unsafe { iput(ip) };
            true
        }
        Err(_) => return false,
    };
    let kid_made = match kern_create(kid, CreateKind::File) {
        Ok(ip) => {
            // SAFETY: balancing put for the created reference.
            unsafe { iput(ip) };
            true
        }
        Err(_) => false,
    };
    // Refused with ENOTEMPTY; dir + kid wrappers pin exactly two
    // slots and the failed unlink must not add a third.
    let refused = matches!(kern_unlink(dir), Err(VfsErr::NotEmpty));
    let balanced = drained_slots() + 2 == base;
    // Teardown: kid first, then the (now empty) dir.
    let gone = kern_unlink(kid).is_ok() && kern_unlink(dir).is_ok();
    made && kid_made && refused && balanced && gone && drained_slots() == base
}

/// "/m4prNN" for NN in 0..80 (7 bytes, well under DIRSIZ). The two
/// decimal digits cap N at 100 — callers beyond that need a wider
/// scheme (asserted so growth fails loud, not silently aliased).
fn press_name(i: usize) -> [u8; 7] {
    assert!(i < 100, "press_name: two-digit budget");
    let mut p = *b"/m4pr00";
    p[5] = b'0' + (i / 10) as u8;
    p[6] = b'0' + (i % 10) as u8;
    p
}

/// 80 files > NINODE(50): creation must recycle inode-table slots by
/// pruning idle dentries (relieve_inode_pressure) — the createdelete
/// regression class — and full teardown returns the ledger to base.
pub fn case_fs_table_pressure() -> bool {
    const N: usize = 80;
    let base = drained_slots();
    let mut made = true;
    for i in 0..N {
        match kern_create(&press_name(i), CreateKind::File) {
            Ok(ip) => {
                // SAFETY: balancing put for the created reference.
                unsafe { iput(ip) };
            }
            Err(_) => {
                made = false;
                break;
            }
        }
    }
    // All 80 resolve as files.
    let mut resolved = made;
    if resolved {
        for i in 0..N {
            if !with_inode(&press_name(i), |ip| ip.type_ == T_FILE) {
                resolved = false;
                break;
            }
        }
    }
    // Full teardown; the ledger must return to base.
    let mut torn = true;
    for i in 0..N {
        if kern_unlink(&press_name(i)).is_err() {
            torn = false;
        }
    }
    made && resolved && torn && drained_slots() == base
}
