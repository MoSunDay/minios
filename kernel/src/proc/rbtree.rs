//! Intrusive red-black tree keyed by `u64` with a cached leftmost node
//! (the Linux kernel rbtree shape): CFS ready entities ordered by
//! vruntime, leftmost == next to run.
//!
//! Pure data structure, no locks (the caller serializes access — sched.rs
//! holds PTABLE_LOCK around every mutation). Order/duplicate/erase cases
//! plus a structural audit live in host-tests (rbtreetests.rs).

/// One embeddable tree link. A link lives in at most one tree; erased
/// links are cleared so a stray re-insert is detectable in debug builds.
#[repr(C)]
pub struct Link {
    parent: *mut Link,
    left: *mut Link,
    right: *mut Link,
    key: u64,
    red: bool,
}

impl Link {
    pub const fn new() -> Link {
        Link {
            parent: core::ptr::null_mut(),
            left: core::ptr::null_mut(),
            right: core::ptr::null_mut(),
            key: 0,
            red: false,
        }
    }

    pub fn key(&self) -> u64 {
        self.key
    }

    /// Test-crate audit accessors (compiled out of the kernel build).
    #[cfg(test)]
    pub fn audit(&self) -> (*mut Link, *mut Link, *mut Link, bool) {
        (self.parent, self.left, self.right, self.red)
    }
}

/// The tree: root, cached leftmost (next-to-run), and population count.
pub struct Tree {
    root: *mut Link,
    leftmost: *mut Link,
    count: usize,
}

impl Tree {
    pub const fn new() -> Tree {
        Tree {
            root: core::ptr::null_mut(),
            leftmost: core::ptr::null_mut(),
            count: 0,
        }
    }

    #[cfg(test)] // audit/model consumers (host-tests crate)
    pub fn len(&self) -> usize {
        self.count
    }

    #[cfg(test)] // audit/model consumers (host-tests crate)
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Cached leftmost (minimum key; earliest arrival on ties).
    pub fn first(&self) -> *mut Link {
        self.leftmost
    }

    /// Test-crate audit accessor (compiled out of the kernel build).
    #[cfg(test)]
    pub fn root(&self) -> *mut Link {
        self.root
    }

    /// In-order successor (null at the end of the walk).
    pub fn successor(&self, n: *mut Link) -> *mut Link {
        // SAFETY: `n` is a live link in this tree (caller contract).
        unsafe {
            if !(*n).right.is_null() {
                return Tree::min((*n).right);
            }
            let mut node = n;
            let mut parent = (*node).parent;
            while !parent.is_null() && node == (*parent).right {
                node = parent;
                parent = (*node).parent;
            }
            parent
        }
    }

    /// Insert `link` under `key`. Equal keys order after existing ones
    /// (FIFO among ties); the cached leftmost only moves on a strictly
    /// smaller key.
    ///
    /// # Safety
    /// `link` is exclusively owned by the caller and not in any tree.
    pub unsafe fn insert(&mut self, link: *mut Link, key: u64) {
        // SAFETY: caller owns the link; tree nodes are live links.
        unsafe {
            let mut y: *mut Link = core::ptr::null_mut();
            let mut x: *mut Link = self.root;
            while !x.is_null() {
                y = x;
                x = if key < (*x).key {
                    (*x).left
                } else {
                    (*x).right
                };
            }
            (*link).parent = y;
            (*link).left = core::ptr::null_mut();
            (*link).right = core::ptr::null_mut();
            (*link).key = key;
            (*link).red = true;
            if y.is_null() {
                self.root = link;
            } else if key < (*y).key {
                (*y).left = link;
            } else {
                (*y).right = link;
            }
            if self.leftmost.is_null() || key < (*self.leftmost).key {
                self.leftmost = link;
            }
            self.count += 1;
            self.insert_fixup(link);
            (*self.root).red = false;
        }
    }

    /// Unlink `link` from the tree (CLRS RB-DELETE with parent pointers).
    ///
    /// # Safety
    /// `link` is a live link in this tree and is exclusively owned after.
    pub unsafe fn erase(&mut self, link: *mut Link) {
        // SAFETY: caller contract; internal invariants hold (see tests).
        unsafe {
            let z = link;
            if z == self.leftmost {
                self.leftmost = self.successor(z);
            }
            let y = if (*z).left.is_null() || (*z).right.is_null() {
                z
            } else {
                Tree::min((*z).right)
            };
            let x = if !(*y).left.is_null() {
                (*y).left
            } else {
                (*y).right
            };
            let y_old_parent = (*y).parent;
            let y_was_z_right = y == (*z).right;

            if !x.is_null() {
                (*x).parent = (*y).parent;
            }
            if (*y).parent.is_null() {
                self.root = x;
            } else if y == (*(*y).parent).left {
                (*(*y).parent).left = x;
            } else {
                (*(*y).parent).right = x;
            }

            let y_was_red = (*y).red;
            if y != z {
                // Graft y (the successor) into z's position, then give it
                // z's color; z's subtrees move under y.
                (*y).parent = (*z).parent;
                if (*z).parent.is_null() {
                    self.root = y;
                } else if z == (*(*z).parent).left {
                    (*(*z).parent).left = y;
                } else {
                    (*(*z).parent).right = y;
                }
                (*y).left = (*z).left;
                if !(*y).left.is_null() {
                    (*(*y).left).parent = y;
                }
                (*y).right = (*z).right;
                if !(*y).right.is_null() {
                    (*(*y).right).parent = y;
                }
                (*y).red = (*z).red;
            }

            // Where would a null `x` hang? (CLRS tracks this via x.p.)
            let x_parent = if !x.is_null() {
                (*x).parent
            } else if y != z && y_was_z_right && y_old_parent == z {
                y // x conceptually sits at y.right after the graft
            } else {
                y_old_parent
            };

            self.count -= 1;
            (*z).parent = core::ptr::null_mut();
            (*z).left = core::ptr::null_mut();
            (*z).right = core::ptr::null_mut();
            (*z).red = false;

            if !y_was_red {
                self.erase_fixup(x, x_parent);
            }
        }
    }

    fn min(mut n: *mut Link) -> *mut Link {
        // SAFETY: `n` is a live link.
        unsafe {
            while !(*n).left.is_null() {
                n = (*n).left;
            }
            n
        }
    }

    /// # Safety
    /// `z` is a live link just inserted red with its parent links set.
    unsafe fn insert_fixup(&mut self, mut z: *mut Link) {
        // SAFETY: CLRS RB-INSERT-FIXUP with parent pointers.
        unsafe {
            while !(*z).parent.is_null() && (*(*z).parent).red {
                let mut p = (*z).parent;
                let g = (*p).parent; // p is red => p is not the root
                if p == (*g).left {
                    let u = (*g).right;
                    if !u.is_null() && (*u).red {
                        (*p).red = false;
                        (*u).red = false;
                        (*g).red = true;
                        z = g;
                    } else {
                        if z == (*p).right {
                            z = p;
                            self.rotate_left(z);
                            p = (*z).parent;
                        }
                        (*p).red = false;
                        (*(*p).parent).red = true;
                        self.rotate_right((*p).parent);
                    }
                } else {
                    let u = (*g).left;
                    if !u.is_null() && (*u).red {
                        (*p).red = false;
                        (*u).red = false;
                        (*g).red = true;
                        z = g;
                    } else {
                        if z == (*p).left {
                            z = p;
                            self.rotate_right(z);
                            p = (*z).parent;
                        }
                        (*p).red = false;
                        (*(*p).parent).red = true;
                        self.rotate_left((*p).parent);
                    }
                }
            }
        }
    }

    /// # Safety
    /// `x` is a (possibly null) link hanging at `xp`; black-height needs
    /// repair (CLRS RB-DELETE-FIXUP).
    unsafe fn erase_fixup(&mut self, x: *mut Link, xp: *mut Link) {
        let mut x = x;
        let mut xp = xp;
        // SAFETY: CLRS RB-DELETE-FIXUP; `w` always exists because a black
        // non-root node has a sibling of black-height >= 1.
        unsafe {
            while x != self.root && (x.is_null() || !(*x).red) {
                if x == (*xp).left {
                    let mut w = (*xp).right;
                    assert!(!w.is_null(), "rbtree: missing sibling");
                    if (*w).red {
                        (*w).red = false;
                        (*xp).red = true;
                        self.rotate_left(xp);
                        w = (*xp).right;
                    }
                    let wl = (*w).left;
                    let wr = (*w).right;
                    if (wl.is_null() || !(*wl).red) && (wr.is_null() || !(*wr).red) {
                        (*w).red = true;
                        x = xp;
                        xp = (*x).parent;
                    } else {
                        if wr.is_null() || !(*wr).red {
                            (*wl).red = false;
                            (*w).red = true;
                            self.rotate_right(w);
                            w = (*xp).right;
                        }
                        (*w).red = (*xp).red;
                        (*xp).red = false;
                        (*(*w).right).red = false;
                        self.rotate_left(xp);
                        x = self.root;
                    }
                } else {
                    let mut w = (*xp).left;
                    assert!(!w.is_null(), "rbtree: missing sibling");
                    if (*w).red {
                        (*w).red = false;
                        (*xp).red = true;
                        self.rotate_right(xp);
                        w = (*xp).left;
                    }
                    let wl = (*w).left;
                    let wr = (*w).right;
                    if (wl.is_null() || !(*wl).red) && (wr.is_null() || !(*wr).red) {
                        (*w).red = true;
                        x = xp;
                        xp = (*x).parent;
                    } else {
                        if wl.is_null() || !(*wl).red {
                            (*wr).red = false;
                            (*w).red = true;
                            self.rotate_left(w);
                            w = (*xp).left;
                        }
                        (*w).red = (*xp).red;
                        (*xp).red = false;
                        (*(*w).left).red = false;
                        self.rotate_right(xp);
                        x = self.root;
                    }
                }
            }
            if !x.is_null() {
                (*x).red = false;
            }
        }
    }

    /// # Safety
    /// `x.right` is non-null.
    unsafe fn rotate_left(&mut self, x: *mut Link) {
        // SAFETY: caller contract; CLRS LEFT-ROTATE.
        unsafe {
            let y = (*x).right;
            (*x).right = (*y).left;
            if !(*y).left.is_null() {
                (*(*y).left).parent = x;
            }
            (*y).parent = (*x).parent;
            if (*x).parent.is_null() {
                self.root = y;
            } else if x == (*(*x).parent).left {
                (*(*x).parent).left = y;
            } else {
                (*(*x).parent).right = y;
            }
            (*y).left = x;
            (*x).parent = y;
        }
    }

    /// # Safety
    /// `x.left` is non-null.
    unsafe fn rotate_right(&mut self, x: *mut Link) {
        // SAFETY: caller contract; CLRS RIGHT-ROTATE.
        unsafe {
            let y = (*x).left;
            (*x).left = (*y).right;
            if !(*y).right.is_null() {
                (*(*y).right).parent = x;
            }
            (*y).parent = (*x).parent;
            if (*x).parent.is_null() {
                self.root = y;
            } else if x == (*(*x).parent).right {
                (*(*x).parent).right = y;
            } else {
                (*(*x).parent).left = y;
            }
            (*y).right = x;
            (*x).parent = y;
        }
    }
}
