//! CFS policy core (UP; plans/linux26/21 §2): pure scheduling math with
//! no locks and no hardware. sched.rs owns the runqueue instance and
//! performs every mutation under PTABLE_LOCK; host-tests drives this
//! module directly as the deterministic fairness model (21 §7: fixed
//! inputs and cadence, single CPU, no migrations).
//!
//! Deviations from Linux CFS (documented in ADR-0016):
//! - exec accounting granularity is the 100 Hz PIT tick (10 ms); TSC
//!   refinement arrives with the M3 time layer (stage s4);
//! - new/woken entities are clamped to `min_vruntime`: no sleeper bonus,
//!   no fork penalty;
//! - tick preemption when `curr.vruntime - front.vruntime` exceeds
//!   `max(SCHED_LATENCY_NS / nr_running, MIN_GRANULARITY_NS)`.

use super::rbtree::{Link, Tree};

/// Weight of a nice-0 task (Linux `NICE_0_LOAD`).
pub const NICE_0_LOAD: u64 = 1024;
/// Target scheduling period per entity at low runnable counts (Linux
/// `sysctl_sched_latency`).
pub const SCHED_LATENCY_NS: u64 = 20_000_000;
/// Minimum slice floor; the 100 Hz tick caps real precision at 10 ms.
pub const MIN_GRANULARITY_NS: u64 = 4_000_000;

/// Linux `sched_prio_to_weight`: nice −20..+19, index = nice + 20.
#[allow(dead_code)] // s2: scheduler-policy syscalls consume it
pub static NICE_TO_WEIGHT: [u64; 40] = [
    88761, 71755, 56483, 46273, 36291, /* -20..-16 */
    29154, 23254, 18705, 14949, 11916, /* -15..-11 */
    9548, 7620, 6100, 4904, 3906, /* -10..-6 */
    3121, 2501, 1991, 1586, 1277, /* -5..-1 */
    1024, 820, 655, 526, 423, /* 0..4 */
    335, 272, 215, 172, 137, /* 5..9 */
    110, 87, 70, 56, 45, /* 10..14 */
    36, 29, 23, 18, 15, /* 15..19 */
];

/// Weight for a nice level (panics outside −20..=19: kernel-internal).
#[allow(dead_code)] // s2: scheduler-policy syscalls consume it
pub fn nice_to_weight(nice: i32) -> u64 {
    debug_assert!((-20..=19).contains(&nice));
    NICE_TO_WEIGHT[(nice + 20) as usize]
}

/// CFS sched entity (21 §2): virtual runtime, nice-derived weight,
/// switch-in stamp, charged runtime. `link` embeds it in the ready tree;
/// the running entity is off-tree (picked out by the scheduler).
pub struct SchedEntity {
    pub link: Link,
    /// Weighted virtual runtime (ns); smaller runs sooner.
    pub vruntime: u64,
    pub load_weight: u64,
    /// Switch-in timestamp (ns); 0 while never picked.
    pub exec_start: u64,
    /// Total charged runtime (ns), weight-independent.
    pub sum_exec_ns: u64,
    pub on_rq: bool,
}

impl SchedEntity {
    pub const fn new() -> SchedEntity {
        SchedEntity {
            link: Link::new(),
            vruntime: 0,
            load_weight: NICE_0_LOAD,
            exec_start: 0,
            sum_exec_ns: 0,
            on_rq: false,
        }
    }
}

/// The UP ready queue: vruntime-ordered tree plus the monotonic floor.
pub struct Runqueue {
    tree: Tree,
    /// Monotonic virtual-time floor: new and woken entities are placed
    /// no earlier than this (Linux `cfs_rq.min_vruntime`).
    pub min_vruntime: u64,
    pub nr_running: u32,
}

impl Runqueue {
    pub const fn new() -> Runqueue {
        Runqueue {
            tree: Tree::new(),
            min_vruntime: 0,
            nr_running: 0,
        }
    }

    #[cfg(test)] // model consumers (host-tests crate)
    pub fn len(&self) -> usize {
        self.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    /// vruntime of the cached front (None when empty).
    pub fn front_vruntime(&self) -> Option<u64> {
        let l = self.tree.first();
        if l.is_null() {
            None
        } else {
            // SAFETY: the cached leftmost is a live link of this tree.
            Some(unsafe { (*l).key() })
        }
    }
}

/// Link → containing entity (`container_of`).
pub fn entity_of(link: *mut Link) -> *mut SchedEntity {
    let off = core::mem::offset_of!(SchedEntity, link);
    ((link as usize) - off) as *mut SchedEntity
}

/// Charge `now - exec_start` to the entity: sum_exec grows by wall
/// (tick) time, vruntime by the weight-scaled share.
///
/// Called only on the current entity (every sched_raw/yield/tick path);
/// `exec_start` is its pick timestamp, legitimately 0 before the first
/// PIT tick — so no zero sentinel is used here.
pub fn update_curr(rq: &mut Runqueue, se: &mut SchedEntity, now: u64) {
    if now <= se.exec_start {
        return;
    }
    let delta = now - se.exec_start;
    se.sum_exec_ns += delta;
    se.vruntime += delta * NICE_0_LOAD / se.load_weight;
    se.exec_start = now;
    if se.vruntime > rq.min_vruntime {
        rq.min_vruntime = se.vruntime;
    }
}

/// Fresh (never-run) entity: start fair at the queue's floor.
pub fn place_new_entity(rq: &Runqueue, se: &mut SchedEntity) {
    se.vruntime = rq.min_vruntime;
}

/// Woken sleeper: keep any earned position, but never run in the past
/// (clamp up to the floor; no sleeper bonus).
pub fn place_wake_entity(rq: &Runqueue, se: &mut SchedEntity) {
    if se.vruntime < rq.min_vruntime {
        se.vruntime = rq.min_vruntime;
    }
}

/// Put a ready entity on the tree (keyed by vruntime; FIFO on ties).
pub fn enqueue_entity(rq: &mut Runqueue, se: &mut SchedEntity) {
    debug_assert!(!se.on_rq, "cfs: double enqueue");
    // SAFETY: the caller owns se exclusively; it is in no tree.
    unsafe { rq.tree.insert(&mut se.link as *mut Link, se.vruntime) };
    se.on_rq = true;
    rq.nr_running += 1;
}

/// Take a ready entity off the tree.
pub fn dequeue_entity(rq: &mut Runqueue, se: &mut SchedEntity) {
    debug_assert!(se.on_rq, "cfs: dequeue of unqueued entity");
    // SAFETY: the link is in this tree.
    unsafe { rq.tree.erase(&mut se.link as *mut Link) };
    se.on_rq = false;
    rq.nr_running -= 1;
}

/// Pick + switch-in bookkeeping: the front entity leaves the tree
/// (Linux `set_next_entity`); min_vruntime advances to the new front.
pub fn pick_next_entity(rq: &mut Runqueue) -> *mut SchedEntity {
    let l = rq.tree.first();
    if l.is_null() {
        return core::ptr::null_mut();
    }
    let se = entity_of(l);
    // SAFETY: the leftmost link is in this tree; se is its entity.
    unsafe { dequeue_entity(rq, &mut *se) };
    if let Some(front) = rq.front_vruntime() {
        if front > rq.min_vruntime {
            rq.min_vruntime = front;
        }
    }
    se
}

/// Slice bound at the current population: the latency window split
/// across all runnable entities — the queued ones plus the running one
/// (kept off-tree in this UP shape; Linux counts curr in nr_running).
fn granularity(rq: &Runqueue) -> u64 {
    (SCHED_LATENCY_NS / (rq.nr_running + 1) as u64).max(MIN_GRANULARITY_NS)
}

/// Tick preemption test (consulted only from user-mode timer traps —
/// kernel preemption stays off, ADR-0016).
pub fn preempt_needed(rq: &Runqueue, curr: &SchedEntity) -> bool {
    let Some(front) = rq.front_vruntime() else {
        return false;
    };
    curr.vruntime.saturating_sub(front) > granularity(rq)
}

/// sched_yield: voluntarily requeue behind the current front.
pub fn yield_entity(rq: &Runqueue, se: &mut SchedEntity) {
    let target = rq.min_vruntime + granularity(rq);
    if se.vruntime < target {
        se.vruntime = target;
    }
}
