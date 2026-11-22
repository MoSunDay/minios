//! namei wave-1 cases: root/dots resolution, the glue's DIRSIZ
//! component cap (the xv6 skipelem truncate), the negative-dentry
//! flip, and file-as-parent walks. Absolute `/`-level names with a
//! fresh `m4` prefix (each <= DIRSIZ); every case cleans up after
//! itself so later cases start from a stable tree, and closes with a
//! slot-ledger check against its own baseline.

use super::{drained_slots, with_inode};
use crate::fs::vfs::VfsErr;
use crate::fs::vfs_glue::{kern_create, kern_namei, kern_unlink, CreateKind};
use crate::fs::xv6fs::inode::iput;
use crate::fs::xv6fs::layout::ROOTINO;

/// "/", "/.", "/.." and ".."-beyond-the-root all resolve to the root
/// inode (the walker's root special case — the anchor every later
/// case's absolute walk leans on).
pub fn case_namei_root_paths() -> bool {
    for path in [b"/".as_slice(), b"/.", b"/..", b"/../.."] {
        if !with_inode(path, |ip| ip.inum == ROOTINO) {
            return false;
        }
    }
    true
}

/// The glue truncates components to DIRSIZ (xv6 skipelem): a 15-char
/// spelling and its 14-char prefix name the same dirent — both at
/// create and at lookup — while a 13-char prefix is a different name
/// (usertests fourteen, pinned at VfsErr level).
pub fn case_namei_dirsiz_trunc() -> bool {
    let long = b"/m4dst0123456789"; // 15-char component
    let short = b"/m4dst012345678"; // 14-char prefix spelling
    let base = drained_slots();
    let inum = match kern_create(long, CreateKind::File) {
        Ok(ip) => {
            // SAFETY: created reference; inum is stable.
            let inum = unsafe { (*ip).inum };
            // SAFETY: balancing put (the wrapper pins the slot).
            unsafe { iput(ip) };
            inum
        }
        Err(_) => return false,
    };
    let ok = with_inode(long, |ip| ip.inum == inum)
        && with_inode(short, |ip| ip.inum == inum)
        && matches!(kern_namei(b"/m4dst01234567"), Err(VfsErr::NoEnt));
    // Teardown always runs, hit or miss (either spelling removes the
    // same dirent; short-circuiting on `ok` would strand the file).
    let unlinked = kern_unlink(short).is_ok();
    ok && unlinked && drained_slots() == base
}

/// miss -> create -> hit -> unlink -> miss: the negative dentry
/// flips positive in place and back, and the slot ledger stays flat
/// across the whole cycle.
pub fn case_namei_negative_flip() -> bool {
    let path = b"/m4flip0";
    let base = drained_slots();
    if !matches!(kern_namei(path), Err(VfsErr::NoEnt)) {
        return false;
    }
    let inum = match kern_create(path, CreateKind::File) {
        Ok(ip) => {
            // SAFETY: created reference.
            let inum = unsafe { (*ip).inum };
            // SAFETY: balancing put.
            unsafe { iput(ip) };
            inum
        }
        Err(_) => return false,
    };
    let hit = with_inode(path, |ip| ip.inum == inum);
    let gone = kern_unlink(path).is_ok() && matches!(kern_namei(path), Err(VfsErr::NoEnt));
    hit && gone && drained_slots() == base
}

/// A file mid-path is ENOTDIR at VfsErr level for both lookup and
/// create (the syscall layer folds it to ENOENT for battery byte
/// stability; this suite pins the pre-fold error — the glue's mod
/// doc names these walk cases as the ENOTDIR surface).
pub fn case_namei_file_as_parent() -> bool {
    let file = b"/m4fap0";
    let base = drained_slots();
    let made = match kern_create(file, CreateKind::File) {
        Ok(ip) => {
            // SAFETY: balancing put for the created reference.
            unsafe { iput(ip) };
            true
        }
        Err(_) => return false,
    };
    let walk = matches!(kern_namei(b"/m4fap0/m4fap0x"), Err(VfsErr::NotDir));
    let mk = matches!(
        kern_create(b"/m4fap0/m4fap0x", CreateKind::File),
        Err(VfsErr::NotDir)
    );
    let gone = kern_unlink(file).is_ok();
    made && walk && mk && gone && drained_slots() == base
}
