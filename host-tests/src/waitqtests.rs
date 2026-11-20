//! Wait-queue policy tests (21 §3): wake discipline, unlink exactness,
//! flag bookkeeping — the pure list layer under sched/pipe users.

use crate::waitqueue::{enqueue_link, select_wake, unlink, WaitLink, WaitQueue};

use crate::waitqueue::MAX_WAITERS;

struct Ent {
    link: WaitLink,
    id: u32,
}

impl Ent {
    fn new(id: u32) -> Ent {
        Ent {
            link: WaitLink::new(),
            id,
        }
    }
}

fn ent_of(l: *mut WaitLink) -> *mut Ent {
    ((l as usize) - core::mem::offset_of!(Ent, link)) as *mut Ent
}

fn ids_of(set: &[*mut WaitLink; MAX_WAITERS], n: usize) -> Vec<u32> {
    let mut v = Vec::new();
    for l in set.iter().take(n) {
        // SAFETY: links from our own ents.
        v.push(unsafe { (*ent_of(*l)).id });
    }
    v
}

#[test]
fn wake_one_takes_all_nonexclusive_plus_first_exclusive() {
    let mut a = Ent::new(0);
    let mut b = Ent::new(1);
    let mut c = Ent::new(2);
    let mut d = Ent::new(3);
    let mut q = WaitQueue::new();
    // SAFETY: owned links, not queued.
    unsafe {
        enqueue_link(&mut a.link, &mut q, true, false); // [a]
        enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        enqueue_link(&mut c.link, &mut q, true, true); // [b a c]
        enqueue_link(&mut d.link, &mut q, true, true); // [b a c d]
    }
    let mut set = [core::ptr::null_mut(); MAX_WAITERS];
    let n = select_wake(&q, true, &mut set);
    assert_eq!(n, 3);
    assert_eq!(ids_of(&set, n), vec![1, 0, 2]); // d (exclusive) stays
}

#[test]
fn wake_all_drains_every_entry() {
    let mut a = Ent::new(0);
    let mut b = Ent::new(1);
    let mut c = Ent::new(2);
    let mut q = WaitQueue::new();
    // SAFETY: owned links.
    unsafe {
        enqueue_link(&mut a.link, &mut q, true, false);
        enqueue_link(&mut b.link, &mut q, false, true); // uninterruptible
        enqueue_link(&mut c.link, &mut q, true, true);
    }
    let mut set = [core::ptr::null_mut(); MAX_WAITERS];
    let n = select_wake(&q, false, &mut set);
    assert_eq!(n, 3);
    assert_eq!(ids_of(&set, n), vec![0, 1, 2]);
}

#[test]
fn unlink_middle_keeps_walk_exact_and_is_idempotent() {
    let mut a = Ent::new(0);
    let mut b = Ent::new(1);
    let mut c = Ent::new(2);
    let mut q = WaitQueue::new();
    // SAFETY: owned links.
    unsafe {
        enqueue_link(&mut a.link, &mut q, true, false); // [a]
        enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        enqueue_link(&mut c.link, &mut q, true, false); // [c b a]
        unlink(&mut b.link); // [c a]
        unlink(&mut b.link); // no-op
    }
    let mut set = [core::ptr::null_mut(); MAX_WAITERS];
    let n = select_wake(&q, false, &mut set);
    assert_eq!(n, 2);
    assert_eq!(ids_of(&set, n), vec![2, 0]);
    // SAFETY: owned links.
    unsafe {
        unlink(&mut a.link);
        unlink(&mut c.link);
    }
    assert!(q.is_empty());
}

#[test]
fn unlink_head_and_tail_fix_neighbors() {
    let mut a = Ent::new(0);
    let mut b = Ent::new(1);
    let mut c = Ent::new(2);
    let mut q = WaitQueue::new();
    // SAFETY: owned links.
    unsafe {
        enqueue_link(&mut a.link, &mut q, true, false); // [a]
        enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        enqueue_link(&mut c.link, &mut q, true, false); // [c b a]
        unlink(&mut c.link); // head out -> [b a]
        unlink(&mut a.link); // tail out -> [b]
    }
    let mut set = [core::ptr::null_mut(); MAX_WAITERS];
    let n = select_wake(&q, false, &mut set);
    assert_eq!(n, 1);
    assert_eq!(ids_of(&set, n), vec![1]);
}

#[test]
fn flags_roundtrip_on_link() {
    let mut a = Ent::new(0);
    let mut q = WaitQueue::new();
    // SAFETY: owned link.
    unsafe {
        enqueue_link(&mut a.link, &mut q, false, true);
    }
    assert!(!a.link.interruptible()); // uninterruptible wait
                                      // SAFETY: owned link.
    unsafe {
        unlink(&mut a.link);
    }
    // Unlink clears the discipline for the next wait.
    assert!(!a.link.interruptible());
}

#[test]
fn distinct_queues_have_distinct_chans() {
    let q1 = WaitQueue::new();
    let q2 = WaitQueue::new();
    assert_ne!(q1.chan(), q2.chan());
    assert!(q1.is_empty() && q2.is_empty());
}
