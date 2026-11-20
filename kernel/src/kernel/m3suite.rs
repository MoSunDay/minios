//! M3 kernel-mode suite: CFS policy (pick order, tick fairness) plus RT
//! FIFO/RR level semantics and the scheduler wiring invariants. Runs in
//! the boot context before the scheduler starts (policy-level battery;
//! process-level dynamics are the s4 guest-time counters). Kernel-only
//! (no root disk): boot cmdline `m3-suite` -> CASE lines -> merged
//! verdict, exit 0x10 -> QEMU 33 (0x11 on any failure).

use crate::arch::x86_64::io;
use crate::console;
use crate::proc::{cfs, rt, waitqueue, SchedPolicy};

/// Link-bearing stand-in for a Proc (both policies are link-agnostic).
struct FakeEnt {
    se: cfs::SchedEntity,
    link: rt::RtLink,
    id: u32,
}

impl FakeEnt {
    const fn new(id: u32) -> FakeEnt {
        FakeEnt {
            se: cfs::SchedEntity::new(),
            link: rt::RtLink::new(),
            id,
        }
    }
}

/// Container recovery from a member pointer — the same `offset_of!`
/// idiom sched.rs uses for `Proc`. Per-build offsets keep this correct
/// under any `#[repr(Rust)]` field reordering (a blind offset-0 cast
/// broke differently in debug vs release; see the s2 battery).
fn ent_of_se(se: *mut cfs::SchedEntity) -> *mut FakeEnt {
    ((se as usize) - core::mem::offset_of!(FakeEnt, se)) as *mut FakeEnt
}

/// `ent_of_se` for the RT link member.
fn ent_of_link(l: *mut rt::RtLink) -> *mut FakeEnt {
    ((l as usize) - core::mem::offset_of!(FakeEnt, link)) as *mut FakeEnt
}

/// m3.cfs.pick_order: vruntime order with FIFO tie-break.
fn case_cfs_pick_order() -> bool {
    let mut ents = [
        FakeEnt::new(0),
        FakeEnt::new(1),
        FakeEnt::new(2),
        FakeEnt::new(3),
    ];
    ents[0].se.vruntime = 30;
    ents[1].se.vruntime = 10;
    ents[2].se.vruntime = 20;
    ents[3].se.vruntime = 10;
    let mut rq = cfs::Runqueue::new();
    for e in ents.iter_mut() {
        cfs::enqueue_entity(&mut rq, &mut e.se);
    }
    // Pick order: vruntime 10 (insertion order 1 then 3), 20, 30.
    let want = [1u32, 3, 2, 0];
    for w in want {
        let se = cfs::pick_next_entity(&mut rq);
        let e = ent_of_se(se);
        // SAFETY: entity_of round-trips into our array elements.
        unsafe {
            if (*e).id != w {
                return false;
            }
        }
    }
    rq.is_empty()
}

/// m3.cfs.tick_fairness: two equal entities share the CPU within two
/// ticks (the xv6.ipc.preempt property at policy level).
fn case_cfs_tick_fairness() -> bool {
    const TICK_NS: u64 = 10_000_000;
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut rq = cfs::Runqueue::new();
    cfs::place_new_entity(&rq, &mut a.se);
    cfs::place_new_entity(&rq, &mut b.se);
    cfs::enqueue_entity(&mut rq, &mut b.se);
    let mut curr = &mut a.se as *mut cfs::SchedEntity;
    // SAFETY: both entities are ours; the loop only calls pure policy.
    unsafe {
        (*curr).exec_start = 0;
        let mut now = 0u64;
        for _ in 0..24 {
            now += TICK_NS;
            cfs::update_curr(&mut rq, &mut *curr, now);
            if cfs::preempt_needed(&rq, &*curr) {
                cfs::enqueue_entity(&mut rq, &mut *curr);
                curr = cfs::pick_next_entity(&mut rq);
                (*curr).exec_start = now;
            }
        }
    }
    let d = (a.se.sum_exec_ns as i64 - b.se.sum_exec_ns as i64).unsigned_abs();
    a.se.sum_exec_ns > 0 && b.se.sum_exec_ns > 0 && d <= 2 * TICK_NS
}

/// m3.rt.prio_order: highest non-empty level wins.
fn case_rt_prio_order() -> bool {
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut c = FakeEnt::new(2);
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links, not in any list.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 5);
        rt::push_tail(&mut rq, &mut b.link, 50);
        rt::push_tail(&mut rq, &mut c.link, 10);
    }
    let mut order = [0u32; 3];
    for slot in order.iter_mut() {
        // SAFETY: front/unlink on owned links.
        unsafe {
            let l = rt::front(&mut rq);
            if l.is_null() {
                return false;
            }
            let prio = rt::front_prio(&rq).unwrap();
            rt::unlink(&mut rq, l, prio);
            *slot = (*ent_of_link(l)).id;
        }
    }
    rq.is_empty() && order == [1, 2, 0]
}

/// m3.rt.fifo_head: FIFO within a level; a preempted FIFO requeues at
/// the head (keeps its turn).
fn case_rt_fifo_head() -> bool {
    let mut a = FakeEnt::new(0);
    let mut b = FakeEnt::new(1);
    let mut c = FakeEnt::new(2);
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links.
    unsafe {
        rt::push_tail(&mut rq, &mut a.link, 10);
        rt::push_tail(&mut rq, &mut b.link, 10);
        if rt::front_prio(&rq) != Some(10) {
            return false;
        }
        let front = rt::front(&mut rq);
        if (*ent_of_link(front)).id != 0 {
            return false; // FIFO arrival order
        }
        rt::push_head(&mut rq, &mut c.link, 10); // preempted FIFO returns
        let front = rt::front(&mut rq);
        (*ent_of_link(front)).id == 2
    }
}

/// m3.rt.rr_rotate: quantum rotation moves the front to the tail —
/// strict round-robin within a level.
fn case_rt_rr_rotate() -> bool {
    let mut ents = [FakeEnt::new(0), FakeEnt::new(1), FakeEnt::new(2)];
    let mut rq = rt::RtRunqueue::new();
    // SAFETY: owned links, same level.
    unsafe {
        for e in ents.iter_mut() {
            rt::push_tail(&mut rq, &mut e.link, 7);
        }
        // Three rotations (sched.rs RR-expiry path shape): front -> tail.
        for _ in 0..3 {
            let l = rt::front(&mut rq);
            rt::unlink(&mut rq, l, 7);
            rt::push_tail(&mut rq, l, 7);
        }
        // Full cycles later the order is back to 0, 1, 2.
        let mut seen = [0u32; 3];
        for slot in seen.iter_mut() {
            let l = rt::front(&mut rq);
            rt::unlink(&mut rq, l, 7);
            *slot = (*ent_of_link(l)).id;
        }
        seen == [0, 1, 2] && rq.is_empty()
    }
}

/// Link-bearing stand-in for a waiter (recovered via `offset_of!`,
/// not blind offset-0 casts — see the s2 battery note above).
struct FakeWqEnt {
    link: waitqueue::WaitLink,
    id: u32,
}

impl FakeWqEnt {
    const fn new(id: u32) -> FakeWqEnt {
        FakeWqEnt {
            link: waitqueue::WaitLink::new(),
            id,
        }
    }
}

fn ent_of_wqlink(l: *mut waitqueue::WaitLink) -> *mut FakeWqEnt {
    ((l as usize) - core::mem::offset_of!(FakeWqEnt, link)) as *mut FakeWqEnt
}

/// m3.wq.wake_policy: Linux wake discipline — non-exclusive entries
/// queue at the head, exclusive at the tail; wake-one takes every
/// non-exclusive waiter plus the first exclusive one; wake-all drains.
fn case_wq_wake_policy() -> bool {
    let mut a = FakeWqEnt::new(0);
    let mut b = FakeWqEnt::new(1);
    let mut c = FakeWqEnt::new(2);
    let mut d = FakeWqEnt::new(3);
    let mut q = waitqueue::WaitQueue::new();
    // SAFETY: owned links, not queued anywhere.
    unsafe {
        waitqueue::enqueue_link(&mut a.link, &mut q, true, false); // [a]
        waitqueue::enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        waitqueue::enqueue_link(&mut c.link, &mut q, true, true); // [b a c]
        waitqueue::enqueue_link(&mut d.link, &mut q, true, true); // [b a c d]
    }
    let mut set = [core::ptr::null_mut(); waitqueue::MAX_WAITERS];
    let n = waitqueue::select_wake(&q, true, &mut set);
    let mut ok = n == 3; // b a (non-exclusive) + c (first exclusive); d stays
    let want = [1u32, 0, 2];
    for (i, l) in set.iter().take(n).enumerate() {
        // SAFETY: links from our own ents.
        unsafe {
            ok = ok && (*ent_of_wqlink(*l)).id == want[i];
        }
    }
    let n_all = waitqueue::select_wake(&q, false, &mut set);
    ok = ok && n_all == 4;
    // SAFETY: owned links.
    unsafe {
        waitqueue::unlink(&mut a.link);
        waitqueue::unlink(&mut b.link);
        waitqueue::unlink(&mut c.link);
        waitqueue::unlink(&mut d.link);
    }
    ok && q.is_empty()
}

/// m3.wq.unlink_middle: unlinking a middle waiter keeps the list walk
/// exact, and a second unlink of the same link is a no-op.
fn case_wq_unlink_middle() -> bool {
    let mut a = FakeWqEnt::new(0);
    let mut b = FakeWqEnt::new(1);
    let mut c = FakeWqEnt::new(2);
    let mut q = waitqueue::WaitQueue::new();
    // SAFETY: owned links.
    unsafe {
        waitqueue::enqueue_link(&mut a.link, &mut q, true, false); // [a]
        waitqueue::enqueue_link(&mut b.link, &mut q, true, false); // [b a]
        waitqueue::enqueue_link(&mut c.link, &mut q, true, false); // [c b a]
        waitqueue::unlink(&mut b.link); // [c a]
        waitqueue::unlink(&mut b.link); // idempotent
    }
    let mut set = [core::ptr::null_mut(); waitqueue::MAX_WAITERS];
    let n = waitqueue::select_wake(&q, false, &mut set);
    let mut ok = n == 2;
    let want = [2u32, 0];
    for (i, l) in set.iter().take(n).enumerate() {
        // SAFETY: links from our own ents.
        unsafe {
            ok = ok && (*ent_of_wqlink(*l)).id == want[i];
        }
    }
    // SAFETY: owned links.
    unsafe {
        waitqueue::unlink(&mut a.link);
        waitqueue::unlink(&mut c.link);
    }
    ok && q.is_empty()
}

/// m3.sched.policy_wiring: Linux-visible numbering and bounds the
/// syscalls and pick order rely on.
fn case_sched_policy_wiring() -> bool {
    let ok_policy = SchedPolicy::from_i64(0) == Some(SchedPolicy::Normal)
        && SchedPolicy::from_i64(1) == Some(SchedPolicy::Fifo)
        && SchedPolicy::from_i64(2) == Some(SchedPolicy::Rr)
        && SchedPolicy::from_i64(3).is_none()
        && SchedPolicy::from_i64(-1).is_none();
    let ok_bounds = rt::RT_PRIO_MIN == 1
        && rt::RT_PRIO_MAX == 99
        && rt::RT_RR_QUANTUM_TICKS == 10
        && (rt::RT_PRIO_MIN..=rt::RT_PRIO_MAX).contains(&50);
    // The RT runqueue empties exactly with its bitmap.
    let rq = rt::RtRunqueue::new();
    let ok_empty = rq.is_empty();
    let ok_gran = cfs::SCHED_LATENCY_NS == 20_000_000
        && cfs::MIN_GRANULARITY_NS == 4_000_000
        && cfs::NICE_0_LOAD == 1024;
    ok_policy && ok_bounds && ok_empty && ok_gran
}

pub fn run() -> ! {
    ksymtab_placeholder();
    let checks: [(&str, fn() -> bool); 8] = [
        ("m3.cfs.pick_order", case_cfs_pick_order),
        ("m3.cfs.tick_fairness", case_cfs_tick_fairness),
        ("m3.rt.prio_order", case_rt_prio_order),
        ("m3.rt.fifo_head", case_rt_fifo_head),
        ("m3.rt.rr_rotate", case_rt_rr_rotate),
        ("m3.sched.policy_wiring", case_sched_policy_wiring),
        ("m3.wq.wake_policy", case_wq_wake_policy),
        ("m3.wq.unlink_middle", case_wq_unlink_middle),
    ];
    let mut total = 0usize;
    let mut passed = 0usize;
    for (case_id, check) in checks {
        total += 1;
        let ok = check();
        console::write_str("CASE ");
        console::write_str(case_id);
        console::write_str(if ok { " PASS\n" } else { " FAIL\n" });
        if ok {
            passed += 1;
        }
    }
    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m3 run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m3 run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x10 } else { 0x11 });
    loop {
        io::hlt();
    }
}

/// m2suite names its symbol prefix via ksymtab; m3 cases are policy-only
/// (no injected faults), so just arm the suite name for the log.
fn ksymtab_placeholder() {
    super::ksymtab::set_active_suite("m3");
}

fn write_decimal(mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &buf[i..] {
        crate::arch::x86_64::console::write_byte(d);
    }
}
