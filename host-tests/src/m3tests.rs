//! m3r-s1 CFS policy tests: the deterministic fairness gate of
//! plans/linux26/21 §7 — same-weight spread < 5%, nice ratios, fixed
//! 10 ms tick cadence, single CPU, no migrations. `Sim` mirrors the
//! sched.rs glue (pick = leftmost off-tree, tick = charge + preempt
//! test) so the model checks the real policy code.

use crate::cfs::{self, SchedEntity};

/// PIT tick period at 100 Hz (the s1 accounting cadence).
const TICK_NS: u64 = 10_000_000;

/// Deterministic UP model of scheduler()/entity_tick() from sched.rs.
struct Sim {
    ents: Vec<SchedEntity>,
    rq: cfs::Runqueue,
    curr: usize,
    now: u64,
}

impl Sim {
    fn new(weights: &[u64]) -> Sim {
        let mut s = Sim {
            ents: weights
                .iter()
                .map(|w| {
                    let mut se = SchedEntity::new();
                    se.load_weight = *w;
                    se
                })
                .collect(),
            rq: cfs::Runqueue::new(),
            curr: usize::MAX,
            now: 0,
        };
        for i in 0..s.ents.len() {
            // Fork path: place_new_entity + enqueue (ptable lock held).
            cfs::place_new_entity(&s.rq, &mut s.ents[i]);
            cfs::enqueue_entity(&mut s.rq, &mut s.ents[i]);
        }
        s.pick();
        s
    }

    /// scheduler(): leftmost leaves the tree, becomes current.
    fn pick(&mut self) {
        let se = cfs::pick_next_entity(&mut self.rq);
        assert!(!se.is_null());
        self.curr = Self::index(&self.ents, se);
        self.ents[self.curr].exec_start = self.now;
    }

    fn index(ents: &[SchedEntity], se: *mut SchedEntity) -> usize {
        let base = ents.as_ptr() as usize;
        ((se as usize) - base) / core::mem::size_of::<SchedEntity>()
    }

    /// entity_tick(): charge the slice, maybe requeue + repick.
    fn tick(&mut self) {
        let c = self.curr;
        self.now += TICK_NS;
        cfs::update_curr(&mut self.rq, &mut self.ents[c], self.now);
        if cfs::preempt_needed(&self.rq, &self.ents[c]) {
            // yield_cpu path: RUNNABLE + enqueue + sched.
            cfs::enqueue_entity(&mut self.rq, &mut self.ents[c]);
            self.pick();
        }
    }

    fn sum_exec(&self, i: usize) -> u64 {
        self.ents[i].sum_exec_ns
    }
}

#[test]
fn nice_weight_table() {
    assert_eq!(cfs::NICE_TO_WEIGHT.len(), 40);
    assert_eq!(cfs::nice_to_weight(0), 1024);
    assert_eq!(cfs::nice_to_weight(-20), 88761);
    assert_eq!(cfs::nice_to_weight(19), 15);
    for n in -20..19 {
        assert!(cfs::nice_to_weight(n) > cfs::nice_to_weight(n + 1));
    }
}

#[test]
fn vruntime_scaled_by_weight() {
    let mut rq = cfs::Runqueue::new();
    let mut heavy = SchedEntity::new();
    heavy.load_weight = cfs::nice_to_weight(-5); // 3121
    let mut light = SchedEntity::new();
    light.load_weight = cfs::nice_to_weight(5); // 335
    heavy.exec_start = 1_000_000;
    light.exec_start = 1_000_000;
    cfs::update_curr(&mut rq, &mut heavy, 11_000_000);
    cfs::update_curr(&mut rq, &mut light, 11_000_000);
    // 10 ms charged: vruntime = 10ms * 1024 / weight.
    assert_eq!(heavy.vruntime, 10_000_000 * 1024 / 3121);
    assert_eq!(light.vruntime, 10_000_000 * 1024 / 335);
    // sum_exec is weight-independent; min_vruntime follows the leader.
    assert_eq!(heavy.sum_exec_ns, 10_000_000);
    assert_eq!(rq.min_vruntime, light.vruntime);
}

#[test]
fn preempt_threshold() {
    let mut rq = cfs::Runqueue::new();
    let mut curr = SchedEntity::new();
    let mut front = SchedEntity::new();
    // Empty queue: never preempt.
    assert!(!cfs::preempt_needed(&rq, &curr));
    // Two runnable (front queued): granularity = max(20ms/2, 4ms) = 10ms.
    cfs::enqueue_entity(&mut rq, &mut front);
    curr.vruntime = 10_000_000;
    assert_eq!(front.vruntime, 0);
    assert!(!cfs::preempt_needed(&rq, &curr)); // == threshold: keep running
    curr.vruntime = 10_000_001;
    assert!(cfs::preempt_needed(&rq, &curr));
    cfs::dequeue_entity(&mut rq, &mut front);
}

#[test]
fn wake_clamp_and_yield() {
    let mut rq = cfs::Runqueue::new();
    let mut runner = SchedEntity::new();
    let mut sleeper = SchedEntity::new();
    // Runner advances min_vruntime by 50 ms.
    runner.exec_start = 1;
    cfs::update_curr(&mut rq, &mut runner, 50_000_001);
    // A task that slept from t=0 re-enters at the floor (no bonus).
    cfs::place_wake_entity(&rq, &mut sleeper);
    assert_eq!(sleeper.vruntime, rq.min_vruntime);
    // Fresh entities also start at the floor.
    let mut fresh = SchedEntity::new();
    cfs::place_new_entity(&rq, &mut fresh);
    assert_eq!(fresh.vruntime, rq.min_vruntime);
    // sched_yield pushes behind the front by the granularity.
    cfs::enqueue_entity(&mut rq, &mut sleeper);
    let mut yielding = SchedEntity::new();
    yielding.vruntime = rq.min_vruntime;
    cfs::yield_entity(&rq, &mut yielding);
    assert_eq!(yielding.vruntime, rq.min_vruntime + 10_000_000);
}

#[test]
fn model_same_weight_fairness() {
    // 21 §7 host gate: 4 equal-weight entities, 4000 ticks (40 s guest).
    let mut s = Sim::new(&[1024; 4]);
    for _ in 0..4_000 {
        s.tick();
    }
    let sums: Vec<u64> = (0..4).map(|i| s.sum_exec(i)).collect();
    let total: u64 = sums.iter().sum();
    let mean = total / 4;
    let spread = (*sums.iter().max().unwrap() - *sums.iter().min().unwrap()) as f64 / mean as f64;
    assert!(spread < 0.05, "same-weight spread {} >= 5%", spread);
    // Everyone ran and the model made progress.
    assert!(sums.iter().all(|s| *s > 0));
    assert_eq!(s.rq.len(), 3); // one entity always current (off-tree)
}

#[test]
fn model_nice_ratio() {
    // 21 §7: nice 0 vs nice 5 → runtime ratio ≈ 1024:335 (±5%).
    let mut s = Sim::new(&[1024, cfs::nice_to_weight(5)]);
    for _ in 0..4_000 {
        s.tick();
    }
    let a = s.sum_exec(0) as f64;
    let b = s.sum_exec(1) as f64;
    let want = 1024.0 / 335.0;
    let got = a / b;
    assert!(
        (got - want).abs() / want < 0.05,
        "nice ratio {} vs want {}",
        got,
        want
    );
}

#[test]
fn model_two_equal_alternate() {
    // Sanity: two equal entities must not starve either side (the
    // xv6.ipc.preempt property the QEMU battery pins).
    let mut s = Sim::new(&[1024, 1024]);
    for _ in 0..200 {
        s.tick();
    }
    let (a, b) = (s.sum_exec(0), s.sum_exec(1));
    let d = (a as i64 - b as i64).unsigned_abs();
    assert!(d <= 2 * TICK_NS, "imbalance {} ns", d);
    assert!(a > 0 && b > 0);
}
