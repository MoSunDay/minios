//! Anonymous-memory OOM policy (M2, plans/linux26 22 §5): anonymous
//! pages have no swap and are never reclaimable, so a user-page
//! allocation failure under pressure is relieved only by terminating
//! the largest anonymous consumer. Every trigger scans the process
//! table, records why candidates were rejected, counts the pages a
//! kill frees, and never drops memory silently or kills an unrelated
//! task (the faulting process is the memory demander by definition).

use super::frame::kalloc;
use super::pgtable;
use crate::klib::sync::Spinlock;
use crate::proc::proc::myproc;
use crate::proc::sched::yield_cpu;
use crate::proc::{Proc, ProcState, NPROC, PTABLE_LOCK};

/// Bounded relief retries: yields for the killed victim to exit (and be
/// reaped by its parent) before the caller gives up.
const RETRY_YIELDS: usize = 64;

/// OOM accounting (22 §5 M2 gate: scanned / rejected / freed / failed).
#[derive(Clone, Copy, PartialEq)]
pub struct OomStats {
    pub scanned: usize,  // processes examined by the collector
    pub rejected: usize, // ineligible: no mm / dying / zero anonymous pages
    pub freed: usize,    // pages freed (or doomed to free) by kills
    pub failed: usize,   // user-page allocations that hit an empty buddy
    pub kills: usize,    // processes marked killed by the OOM policy
}

/// One OOM candidate snapshot; selection is pure over these.
#[derive(Clone, Copy)]
pub struct Candidate {
    pub pid: i32,
    pub anon_pages: usize,
}

/// Candidate collection result: filled prefix length plus accounting.
struct Collected {
    count: usize,
    scanned: usize,
    rejected: usize,
    caller_pid: i32,
}

static OOM_LOCK: Spinlock<()> = Spinlock::new("oom", ());
static mut STATS: OomStats = OomStats {
    scanned: 0,
    rejected: 0,
    freed: 0,
    failed: 0,
    kills: 0,
};

fn stats_mut() -> &'static mut OomStats {
    // SAFETY: caller holds OOM_LOCK; single CPU.
    unsafe { &mut *core::ptr::addr_of_mut!(STATS) }
}

/// Point-in-time snapshot of the OOM counters.
pub fn stats() -> OomStats {
    let _guard = OOM_LOCK.lock();
    *stats_mut()
}

/// Pure candidate selection: the process with the largest anonymous
/// footprint wins; ties resolve to the lowest pid. None when the table
/// holds no terminable candidate (empty, all zero-footprint).
pub fn select_oom_victim(cands: &[Candidate]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, c) in cands.iter().enumerate() {
        if c.anon_pages == 0 {
            continue;
        }
        best = match best {
            None => Some(i),
            Some(b) => {
                let cur = &cands[b];
                let take = c.anon_pages > cur.anon_pages
                    || (c.anon_pages == cur.anon_pages && c.pid < cur.pid);
                if take {
                    Some(i)
                } else {
                    Some(b)
                }
            }
        };
    }
    best
}

/// Snapshot every live, killable user process as a candidate (footprint
/// walked under the ptable lock; on a single CPU an owner cannot mutate
/// its tables while we hold the CPU).
fn collect_candidates(out: &mut [Candidate]) -> Collected {
    let mut r = Collected {
        count: 0,
        scanned: 0,
        rejected: 0,
        caller_pid: -1,
    };
    let me = myproc();
    if !me.is_null() {
        r.caller_pid = unsafe { (*me).pid };
    }
    // SAFETY: ptable lock held for the whole walk.
    let _guard = PTABLE_LOCK.lock();
    let table = unsafe { crate::proc::ptable_mut() };
    for slot in table.iter() {
        let p = slot as *const Proc;
        r.scanned += 1;
        if unsafe { (*p).state } == ProcState::UNUSED
            || unsafe { (*p).state } == ProcState::EMBRYO
            || unsafe { (*p).state } == ProcState::ZOMBIE
            || unsafe { (*p).killed } != 0
        {
            r.rejected += 1;
            continue;
        }
        // SAFETY: ptable lock held; the owner is not running (1 CPU).
        let anon = match unsafe { (*p).mm.as_ref() } {
            Some(mm) => unsafe { pgtable::count_user_pages(mm.pml4) },
            None => {
                r.rejected += 1; // kernel thread / boot context: never a victim
                continue;
            }
        };
        if anon == 0 {
            r.rejected += 1;
            continue;
        }
        if r.count < out.len() {
            out[r.count] = Candidate {
                pid: unsafe { (*p).pid },
                anon_pages: anon,
            };
            r.count += 1;
        } else {
            r.rejected += 1; // snapshot overflow: defensively skip
        }
    }
    r
}

/// Allocation-failure trigger: count the failure, scan, select, and
/// terminate the victim. true means a process OTHER than the caller was
/// killed and memory may come back (the caller should yield and retry).
pub fn on_alloc_failure() -> bool {
    {
        let _guard = OOM_LOCK.lock();
        stats_mut().failed += 1;
    }
    let mut cands = [Candidate {
        pid: 0,
        anon_pages: 0,
    }; NPROC];
    let r = collect_candidates(&mut cands);
    {
        let _guard = OOM_LOCK.lock();
        stats_mut().scanned += r.scanned;
        stats_mut().rejected += r.rejected;
    }
    let vi = match select_oom_victim(&cands[..r.count]) {
        Some(i) => i,
        None => return false,
    };
    let (vpid, vpages) = (cands[vi].pid, cands[vi].anon_pages);
    // Mark killed under the ptable lock; wake it if it sleeps so the
    // kill takes effect at its next trap instead of never.
    // SAFETY: ptable lock held.
    unsafe {
        let _guard = PTABLE_LOCK.lock();
        let table = crate::proc::ptable_mut();
        for slot in table.iter_mut() {
            if slot.pid == vpid && slot.state != ProcState::ZOMBIE {
                slot.killed = 1;
                if slot.state == ProcState::SLEEPING {
                    crate::proc::sleep::wakeup1(slot.chan);
                }
                break;
            }
        }
    }
    {
        let _guard = OOM_LOCK.lock();
        stats_mut().kills += 1;
        stats_mut().freed += vpages;
    }
    vpid != r.caller_pid
}

/// Anonymous user-page allocation with the M2 relief valve: plain kalloc
/// on success; on failure run the OOM policy and, when another process
/// was killed for this allocation, yield-retry while its frames come
/// back. Null only when nothing can be freed.
pub fn alloc_user_page() -> *mut u8 {
    let p = kalloc();
    if !p.is_null() {
        return p;
    }
    if !on_alloc_failure() {
        return core::ptr::null_mut(); // self-killed, or no victim at all
    }
    for _ in 0..RETRY_YIELDS {
        yield_cpu();
        let q = kalloc();
        if !q.is_null() {
            return q;
        }
    }
    core::ptr::null_mut()
}
