//! rbtree structural audit + order/duplicate/erase cases for the CFS
//! ready tree (m3r-s1). The audit walker lives here (test crate) so the
//! kernel build carries no audit code.

use crate::rbtree::{Link, Tree};

/// Recursive shape check: parent links, red nodes have black children,
/// uniform black-height. Returns the subtree black-height.
fn walk_shape(n: *mut Link, parent: *mut Link, count: &mut usize) -> Result<u64, &'static str> {
    if n.is_null() {
        return Ok(1); // null-leaf black count
    }
    *count += 1;
    // SAFETY: `n` is a live link of the tree being audited.
    unsafe {
        let (np, nl, nr, nred) = (*n).audit();
        if np != parent {
            return Err("bad parent link");
        }
        if nred && !nl.is_null() && (*nl).audit().3 {
            return Err("red node with red left child");
        }
        if nred && !nr.is_null() && (*nr).audit().3 {
            return Err("red node with red right child");
        }
        let lbh = walk_shape(nl, n, count)?;
        let rbh = walk_shape(nr, n, count)?;
        if lbh != rbh {
            return Err("black-height mismatch");
        }
        Ok(lbh + u64::from(!nred))
    }
}

/// Full structural audit of a tree.
fn validate(t: &Tree) -> Result<(), &'static str> {
    let root = t.root();
    if root.is_null() {
        if t.len() != 0 || !t.first().is_null() {
            return Err("empty tree with nonzero count/leftmost");
        }
        return Ok(());
    }
    // SAFETY: root/links are live nodes of `t`.
    unsafe {
        let (rp, _, _, rr) = (*root).audit();
        if !rp.is_null() {
            return Err("root has a parent");
        }
        if rr {
            return Err("root is red");
        }
    }
    // In-order successor walk: non-decreasing keys, full population.
    let mut keys = Vec::new();
    let mut n = t.first();
    while !n.is_null() {
        // SAFETY: live link.
        keys.push(unsafe { (*n).key() });
        n = t.successor(n);
    }
    for w in keys.windows(2) {
        if w[1] < w[0] {
            return Err("BST order violated");
        }
    }
    if keys.len() != t.len() {
        return Err("count mismatch vs in-order walk");
    }
    // Cached leftmost must carry the minimum key.
    // SAFETY: live link.
    let first_key = unsafe { (*t.first()).key() };
    if first_key != keys[0] {
        return Err("leftmost cache stale");
    }
    let mut count = 0usize;
    walk_shape(root, core::ptr::null_mut(), &mut count)?;
    if count != t.len() {
        return Err("count mismatch vs shape walk");
    }
    Ok(())
}

/// Deterministic LCG shuffle of `0..n`.
fn shuffled(n: usize) -> Vec<u64> {
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut keys: Vec<u64> = (0..n as u64).collect();
    for i in (1..n).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = ((seed >> 33) % ((i + 1) as u64)) as usize;
        keys.swap(i, j);
    }
    keys
}

#[test]
fn rbtree_shuffled_insert_erase() {
    let mut links: Vec<Link> = (0..256).map(|_| Link::new()).collect();
    let mut tree = Tree::new();
    let keys = shuffled(256);
    for (i, k) in keys.iter().enumerate() {
        // SAFETY: link i is exclusively owned and not yet in any tree.
        unsafe { tree.insert(&mut links[i] as *mut Link, *k) };
        if i % 16 == 15 {
            validate(&tree).unwrap();
        }
    }
    assert_eq!(tree.len(), 256);
    validate(&tree).unwrap();

    // Erase every even key, keep auditing.
    for (i, k) in keys.iter().enumerate() {
        if k % 2 == 0 {
            // SAFETY: link i is in the tree.
            unsafe { tree.erase(&mut links[i] as *mut Link) };
            if i % 16 == 15 {
                validate(&tree).unwrap();
            }
        }
    }
    validate(&tree).unwrap();
    assert_eq!(tree.len(), 128);

    let mut odd = Vec::new();
    let mut n = tree.first();
    while !n.is_null() {
        // SAFETY: live link.
        odd.push(unsafe { (*n).key() });
        n = tree.successor(n);
    }
    assert!(odd.iter().all(|k| k % 2 == 1));
    assert_eq!(odd.len(), 128);
}

#[test]
fn rbtree_duplicate_keys_fifo() {
    let mut links: Vec<Link> = (0..8).map(|_| Link::new()).collect();
    let mut tree = Tree::new();
    for l in links.iter_mut() {
        // SAFETY: exclusively owned link, not yet in any tree.
        unsafe { tree.insert(l as *mut Link, 42) };
        // SAFETY: live leftmost link.
        assert_eq!(unsafe { (*tree.first()).key() }, 42);
    }
    validate(&tree).unwrap();
    // Pop order must be insertion order (FIFO among equal keys).
    for l in links.iter_mut() {
        assert_eq!(tree.first(), l as *mut Link);
        // SAFETY: the leftmost link is in the tree.
        unsafe { tree.erase(tree.first()) };
    }
    assert!(tree.is_empty());
    assert!(tree.first().is_null());
}

#[test]
fn rbtree_small_edges() {
    let mut a = Link::new();
    let mut b = Link::new();
    let mut c = Link::new();
    let mut t = Tree::new();
    // Single node root erase.
    // SAFETY: exclusively owned links.
    unsafe {
        t.insert(&mut a, 5);
        validate(&t).unwrap();
        t.erase(&mut a);
    }
    assert!(t.is_empty());
    // Root with two children; erase root (two-child case).
    // SAFETY: exclusively owned links.
    unsafe {
        t.insert(&mut b, 7);
        t.insert(&mut c, 3);
        t.insert(&mut a, 9);
        validate(&t).unwrap();
        t.erase(&mut b); // two children
        validate(&t).unwrap();
        assert_eq!((*t.first()).key(), 3);
        t.erase(&mut a);
        validate(&t).unwrap();
        assert_eq!((*t.first()).key(), 3);
        t.erase(&mut c);
    }
    assert!(t.is_empty());
    validate(&t).unwrap();
}
