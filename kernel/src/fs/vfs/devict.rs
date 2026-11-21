//! devict.rs: dcache invalidation policy, pure core (M4 s1) —
//! rename (with occupant displacement), subtree shrink, and
//! leaf-ordered LRU prune. Split from dcache.rs for the file-line
//! budget; one Dcache, two cohesive halves.

use super::dcache::Dcache;
use super::dentry::{Dentry, RenameOutcome, NAME_MAX};
use super::{VfsErr, VfsResult};

impl Dcache {
    /// rename(2) dcache half. The caller runs the inode-level checks
    /// (ENOTEMPTY, type rules, cross-mount) first; this core reports
    /// what it displaced so the glue can finish the job.
    pub fn rename(
        &mut self,
        old_parent: *mut Dentry,
        old_name: &[u8],
        new_parent: *mut Dentry,
        new_name: &[u8],
    ) -> VfsResult<RenameOutcome> {
        if new_name.is_empty() || new_name.len() > NAME_MAX {
            return Err(VfsErr::NameTooLong);
        }
        let old = self.lookup(old_parent, old_name);
        if old.is_null() || unsafe { (*old).is_negative() } {
            return Err(VfsErr::NoEnt);
        }
        if unsafe { (*old).is_mounted() } {
            return Err(VfsErr::Busy);
        }
        if self.is_ancestor_or_self(old, new_parent) {
            return Err(VfsErr::Inval); // moving a dir under itself
        }
        let target = self.lookup(new_parent, new_name);
        let outcome = if target.is_null() {
            RenameOutcome::Moved
        } else if target == old {
            return Ok(RenameOutcome::NoOp);
        } else if unsafe { (*target).is_mounted() } {
            return Err(VfsErr::Busy); // cannot rename over a mountpoint
        } else {
            // Occupied: hand the caller one reference, detach, and
            // retire the occupant's idle descendants (referenced ones
            // survive detached, pinning nothing but themselves).
            self.dget(target);
            self.child_unlink(target);
            // SAFETY: target is live and detached.
            let inode = unsafe { (*target).d_inode };
            let _busy = self.shrink_subtree(target);
            RenameOutcome::Replaced {
                dentry: target,
                inode,
            }
        };
        // Move the source: relink under the new parent, new name.
        self.child_unlink(old);
        // SAFETY: old is ours to relink.
        unsafe {
            let e = &mut *old;
            e.d_name[..new_name.len()].copy_from_slice(new_name);
            e.d_name[new_name.len()] = 0;
            e.d_len = new_name.len() as u16;
        }
        self.child_link(new_parent, old);
        self.stats.renames += 1;
        Ok(outcome)
    }

    /// Free the idle descendants of `d` (shrink_dcache_parent core).
    /// Returns how many referenced descendants blocked. Children that
    /// remain attached because their own children are referenced also
    /// count as blocked.
    pub fn shrink_subtree(&mut self, d: *mut Dentry) -> u32 {
        let mut busy = 0u32;
        let mut cur = self.first_child(d);
        while !cur.is_null() {
            // SAFETY: children chain of d.
            let c = unsafe { &*cur };
            let next = c.d_next;
            busy += self.shrink_subtree(cur);
            // SAFETY: c is still live (referenced or freshly childless).
            let c2 = unsafe { &*cur };
            if c2.d_ref == 0 && c2.d_first_child.is_null() {
                self.retire_entry(cur);
            } else {
                busy += 1;
            }
            cur = next;
        }
        busy
    }

    /// Evict up to `n` idle entries, oldest first. Leaf-ordered: an
    /// idle parent is only taken after its children; mounted entries
    /// are never taken. Returns the number freed.
    pub fn prune_lru(&mut self, n: usize) -> usize {
        let mut freed = 0usize;
        let mut skips = 0u32;
        while freed < n {
            if skips > self.stats.lru_len {
                break; // every idle entry is pinned: nothing to take
            }
            let cand = self.lru_head;
            if cand.is_null() {
                break;
            }
            // SAFETY: LRU members are live pool entries.
            let take = unsafe {
                let c = &*cand;
                !c.is_mounted()
                    && c.d_ref == 0
                    && (c.d_first_child.is_null()
                        || self.shrink_subtree(cand) == 0 && (*cand).d_first_child.is_null())
            };
            if take {
                self.retire_entry(cand);
                freed += 1;
                skips = 0;
            } else {
                // Park it at the tail and look at the next-oldest.
                self.lru_unlink(cand);
                self.lru_push_tail(cand);
                skips += 1;
            }
        }
        freed
    }
}
