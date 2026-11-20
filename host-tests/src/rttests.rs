//! m3r-s2 RT policy tests: level ordering, FIFO within a level, head
//! requeue for a preempted FIFO, RR tail rotation, and the strictly-
//! higher predicate the tick path consults.

use crate::rt::{self, RtLink, RtRunqueue};

/// Link-bearing stand-in for a Proc (rt.rs is link-agnostic).
struct Ent {
    link: RtLink,
    id: u32,
}

fn ent(id: u32) -> Ent {
    Ent {
        link: RtLink::new(),
        id,
    }
}

fn front_id(rq: &mut RtRunqueue) -> Option<u32> {
    let l = rt::front(rq);
    if l.is_null() {
        return None;
    }
    // SAFETY: front() is a live link of an Ent we own.
    let e = unsafe { &mut *(l as *mut Ent) };
    Some(e.id)
}

fn pop_front(rq: &mut RtRunqueue) -> Option<u32> {
    let l = rt::front(rq);
    if l.is_null() {
        return None;
    }
    let prio = rt::front_prio(rq).unwrap();
    // SAFETY: front link is in prio's list.
    unsafe { rt::unlink(rq, l, prio) };
    // SAFETY: live link of an owned Ent.
    Some(unsafe { (*(l as *mut Ent)).id })
}

#[test]
fn rt_level_order() {
    let mut rq = RtRunqueue::new();
    let mut a = ent(1);
    let mut b = ent(2);
    let mut c = ent(3);
    assert!(rt::front_prio(&rq).is_none());
    assert!(rq.is_empty());
    // SAFETY: owned links, not in any list.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 5);
        rt::push_tail(&mut rq, &mut b.link, 50);
        rt::push_tail(&mut rq, &mut c.link, 10);
    }
    assert_eq!(rt::front_prio(&rq), Some(50));
    assert_eq!(rq.nr_running(), 3);
    // Pop order is strictly by priority: 50, 10, 5.
    assert_eq!(pop_front(&mut rq), Some(2));
    assert_eq!(pop_front(&mut rq), Some(3));
    assert_eq!(pop_front(&mut rq), Some(1));
    assert!(rq.is_empty());
    assert_eq!(rt::front_prio(&rq), None);
}

#[test]
fn rt_fifo_within_level_and_head_requeue() {
    let mut rq = RtRunqueue::new();
    let mut a = ent(1);
    let mut b = ent(2);
    let mut c = ent(3);
    // SAFETY: owned links.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 10);
        rt::push_tail(&mut rq, &mut b.link, 10);
        assert_eq!(front_id(&mut rq), Some(1)); // FIFO arrival
        rt::push_head(&mut rq, &mut c.link, 10); // preempted FIFO returns
        assert_eq!(front_id(&mut rq), Some(3));
        rt::unlink(&mut rq, &mut c.link, 10);
        assert_eq!(front_id(&mut rq), Some(1));
        // RR rotation: front to the tail.
        rt::unlink(&mut rq, &mut a.link, 10);
        rt::push_tail(&mut rq, &mut a.link, 10);
        assert_eq!(front_id(&mut rq), Some(2));
        assert_eq!(pop_front(&mut rq), Some(2));
        assert_eq!(pop_front(&mut rq), Some(1));
    }
    assert!(rq.is_empty());
}

#[test]
fn rt_bounds_and_higher() {
    let mut rq = RtRunqueue::new();
    let mut lo = ent(1);
    let mut hi = ent(2);
    assert!(!rt::has_higher(&rq, 1));
    // SAFETY: owned links.
    unsafe {
        rt::push_tail(&mut rq, &mut lo.link, rt::RT_PRIO_MIN);
        assert!(!rt::has_higher(&rq, rt::RT_PRIO_MIN));
        assert!(rt::has_higher(&rq, rt::RT_PRIO_MIN - 1));
        rt::push_tail(&mut rq, &mut hi.link, rt::RT_PRIO_MAX);
        assert_eq!(rt::front_prio(&rq), Some(rt::RT_PRIO_MAX));
        assert!(rt::has_higher(&rq, 50));
    }
    assert_eq!(rt::RT_RR_QUANTUM_TICKS, 10); // 100 ms at 100 Hz
}

#[test]
fn rt_unlink_middle_tail_integrity() {
    let mut rq = RtRunqueue::new();
    let mut ents: Vec<Ent> = (0..5).map(|i| ent(i)).collect();
    // SAFETY: owned links, same level.
    unsafe {
        for e in ents.iter_mut() {
            rt::push_tail(&mut rq, &mut e.link, 7);
        }
        // Unlink the middle (2) and the tail (4); order must hold.
        rt::unlink(&mut rq, &mut ents[2].link, 7);
        rt::unlink(&mut rq, &mut ents[4].link, 7);
        assert_eq!(pop_front(&mut rq), Some(0));
        assert_eq!(pop_front(&mut rq), Some(1));
        assert_eq!(pop_front(&mut rq), Some(3));
        // Push after middle/tail removals still appends.
        rt::push_tail(&mut rq, &mut ents[2].link, 7);
        assert_eq!(pop_front(&mut rq), Some(2));
    }
    assert!(rq.is_empty());
}
