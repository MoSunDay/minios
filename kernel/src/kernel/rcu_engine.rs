//! RCU grace engine, pure core (21 §5, M3 s7): the Linux TINY_RCU
//! three-list batch pipeline (nxt -> cur -> done) reshaped as zero-
//! dependency code so host tests drive the exact state machine the
//! kernel runs (same file, included by path — no test-only copy).
//! The kernel glue (reader count, callback lock, softirq plumbing,
//! sync services) lives in rcu.rs; quiescent-state semantics:
//! a batch parked in `cur` waits for one QS observed AFTER its grace
//! period opened, and QS reports are suppressed while any reader is
//! active (the kernel glue supplies that flag).

use core::ptr::null_mut;

/// Linux rcu_head: intrusive single link plus the callback to invoke
/// at the end of a grace period.
pub struct RcuHead {
    pub next: *mut RcuHead,
    pub func: fn(*mut RcuHead),
}

impl RcuHead {
    pub const fn new(func: fn(*mut RcuHead)) -> RcuHead {
        RcuHead {
            next: null_mut(),
            func,
        }
    }
}

/// Intrusive FIFO list of RcuHead nodes (tail-enqueue, head-drain).
#[derive(Clone, Copy)]
pub struct RcuList {
    head: *mut RcuHead,
    /// Null iff head is null (empty list).
    tail: *mut RcuHead,
}

impl RcuList {
    pub const fn new() -> RcuList {
        RcuList {
            head: null_mut(),
            tail: null_mut(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    /// Append one node at the tail (call_rcu enqueue).
    pub fn push(&mut self, h: *mut RcuHead) {
        // SAFETY: the caller hands exclusive ownership of `h` to the
        // list until invocation dequeues it.
        unsafe { (*h).next = null_mut() };
        if self.head.is_null() {
            self.head = h;
        } else {
            // SAFETY: tail is a queued node.
            unsafe { (*self.tail).next = h };
        }
        self.tail = h;
    }

    /// Detach the whole list (leaves this list empty).
    pub fn take(&mut self) -> RcuList {
        let l = *self;
        *self = RcuList::new();
        l
    }

    /// Splice `other` onto the tail (no-op when `other` is empty).
    pub fn append(&mut self, other: RcuList) {
        if other.is_empty() {
            return;
        }
        if self.head.is_null() {
            *self = other;
        } else {
            // SAFETY: both tails are queued nodes of their lists.
            unsafe { (*self.tail).next = other.head };
            self.tail = other.tail;
        }
    }
}

/// TINY-RCU batch state machine: callbacks flow nxt -> cur -> done.
/// `qs_pending` is cleared when a new GP opens, so a QS seen while no
/// GP is active never carries over (conservative TINY shape — the
/// kernel's global reader count would keep a carried QS safe too).
pub struct RcuEngine {
    nxt: RcuList,
    cur: RcuList,
    done: RcuList,
    qs_pending: bool,
    gp_completed: u64,
    cbs_invoked: u64,
}

impl RcuEngine {
    pub const fn new() -> RcuEngine {
        RcuEngine {
            nxt: RcuList::new(),
            cur: RcuList::new(),
            done: RcuList::new(),
            qs_pending: false,
            gp_completed: 0,
            cbs_invoked: 0,
        }
    }

    /// call_rcu enqueue: the callback waits for the next grace period.
    pub fn queue_cb(&mut self, h: *mut RcuHead) {
        self.nxt.push(h);
    }

    /// Quiescent-state report. `readers_active`: some context is inside
    /// an RCU read section — the QS is suppressed (deferred grace).
    /// Returns true when the active GP may now complete (the caller
    /// raises RCU_SOFTIRQ to drain it).
    pub fn note_qs(&mut self, readers_active: bool) -> bool {
        if readers_active || self.cur.is_empty() {
            return false;
        }
        self.qs_pending = true;
        true
    }

    /// Softirq step 1 (under the engine lock): open any pending GP,
    /// then complete the active GP if a QS was seen since it opened.
    pub fn advance(&mut self) {
        if self.cur.is_empty() && !self.nxt.is_empty() {
            self.cur = self.nxt.take();
            self.qs_pending = false;
        }
        if self.qs_pending && !self.cur.is_empty() {
            self.done.append(self.cur.take());
            self.qs_pending = false;
            self.gp_completed += 1;
        }
    }

    /// Softirq step 2: detach completed callbacks — invocation runs
    /// OUTSIDE the engine lock (funcs may take other locks).
    pub fn take_done(&mut self) -> RcuList {
        self.done.take()
    }

    /// Invoke a detached list head-to-tail; returns how many ran.
    pub fn invoke(list: RcuList) -> u64 {
        let mut n = 0u64;
        let mut cur = list.head;
        while !cur.is_null() {
            // SAFETY: dequeued nodes are owned by this invocation; the
            // func contract touches only its container, never the link.
            let (next, func) = unsafe { ((*cur).next, (*cur).func) };
            func(cur);
            cur = next;
            n += 1;
        }
        n
    }

    /// True when no batch sits anywhere in the pipeline.
    #[allow(dead_code)] // host rcu tests only
    pub fn is_idle(&self) -> bool {
        self.nxt.is_empty() && self.cur.is_empty() && self.done.is_empty()
    }

    /// True while a grace period is open (QS reports are relevant).
    #[allow(dead_code)] // host rcu tests
    pub fn gp_active(&self) -> bool {
        !self.cur.is_empty()
    }

    /// Work left for later grace periods (re-raise on the next QS).
    pub fn has_work(&self) -> bool {
        !self.nxt.is_empty() || !self.cur.is_empty()
    }

    /// (grace periods completed, callbacks invoked) — m3 probes.
    pub fn stats(&self) -> (u64, u64) {
        (self.gp_completed, self.cbs_invoked)
    }

    /// Account invoked callbacks (after the outside-lock run).
    pub fn add_invoked(&mut self, n: u64) {
        self.cbs_invoked += n;
    }
}
