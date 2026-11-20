//! Workqueues (21 §3, UP): sleepable deferred work — the contrast with
//! softirqs (which must not sleep). A workqueue is a list of
//! WorkStructs drained by a dedicated kthread (the 2.6 `events` model,
//! single worker on UP); queue_work de-duplicates via the pending bit
//! and wakes the worker (lazily spawned on first use — boot/process
//! context only, a divergence recorded in ADR-0020; Linux also allows
//! IRQ context via a pending-spinning variant). flush_work sleeps on
//! a completion waitqueue the worker signals whenever the queue drains.

use crate::klib::sync::Spinlock;
use crate::proc::{kthread, sleep, waitqueue, Proc};

pub struct WorkStruct {
    next: *mut WorkStruct,
    func: fn(*mut WorkStruct),
    pending: bool,
}

impl WorkStruct {
    pub const fn new(func: fn(*mut WorkStruct)) -> Self {
        WorkStruct {
            next: core::ptr::null_mut(),
            func,
            pending: false,
        }
    }
}

pub struct WorkqueueStruct {
    lock: Spinlock<()>,
    /// FIFO of pending works.
    list: *mut WorkStruct,
    /// A work item is currently executing (drained but not idle).
    running: bool,
    /// Worker sleeps here when idle (exclusive: wake-one).
    more: waitqueue::WaitQueue,
    /// flush_work sleepers; woken when the queue drains.
    done: waitqueue::WaitQueue,
    worker: *mut Proc,
}

impl WorkqueueStruct {
    pub const fn new() -> Self {
        WorkqueueStruct {
            lock: Spinlock::new("workqueue", ()),
            list: core::ptr::null_mut(),
            running: false,
            more: waitqueue::WaitQueue::new(),
            done: waitqueue::WaitQueue::new(),
            worker: core::ptr::null_mut(),
        }
    }
}

/// The default (events) workqueue.
pub static mut EVENTS: WorkqueueStruct = WorkqueueStruct::new();

/// Queue `work` on `wq` (false when already pending — the 2.6
/// de-duplication contract).
///
/// # Safety
/// Boot or process context (the lazy worker spawn allocates); `work`
/// must stay valid until its function has run.
pub unsafe fn queue_work(wq: *mut WorkqueueStruct, work: *mut WorkStruct) -> bool {
    let spawn;
    {
        let _guard = (*wq).lock.lock();
        if (*work).pending {
            return false;
        }
        (*work).pending = true;
        (*work).next = core::ptr::null_mut();
        if (*wq).list.is_null() {
            (*wq).list = work;
        } else {
            let mut cur = (*wq).list;
            while !(*cur).next.is_null() {
                cur = (*cur).next;
            }
            (*cur).next = work;
        }
        spawn = (*wq).worker.is_null();
        if spawn {
            // SAFETY: quiet context, slot owned until kthread_run.
            let p = unsafe { kthread::kthread_create(worker_main, wq as usize, b"events") };
            if p.is_null() {
                return false;
            }
            // SAFETY: EMBRYO from kthread_create.
            unsafe { kthread::kthread_run(p) };
            (*wq).worker = p;
        }
    }
    // Wake after dropping the queue lock (the sleeper releases it
    // across its sleep; the waker never needs it).
    // SAFETY: static-free wake under the ptable lock.
    unsafe { sleep::wake_up(&mut (*wq).more, true) };
    true
}

/// `events` shorthand.
///
/// # Safety
/// Same contract as queue_work.
#[allow(dead_code)] // M5: writeback queues its works through here
pub unsafe fn schedule_work(work: *mut WorkStruct) -> bool {
    // SAFETY: EVENTS static owned by this module.
    unsafe { queue_work(core::ptr::addr_of_mut!(EVENTS), work) }
}

/// # Safety-free kthread body: arg is the queue pointer (kthread contract).
extern "C" fn worker_main(arg: usize) -> i32 {
    let wq = arg as *mut WorkqueueStruct;
    loop {
        // Drain: pop one work at a time, run it unlocked (it may sleep),
        // then re-check under the lock.
        // SAFETY: wq points at the queue that owns this worker; the
        // raw pointer is the kthread arg contract.
        unsafe {
            loop {
                let w;
                {
                    let _guard = (*wq).lock.lock();
                    w = (*wq).list;
                    if !w.is_null() {
                        (*wq).list = (*w).next;
                        (*wq).running = true;
                    }
                }
                if w.is_null() {
                    break;
                }
                ((*w).func)(w);
                let _guard = (*wq).lock.lock();
                (*w).pending = false;
                if (*wq).list.is_null() {
                    (*wq).running = false;
                    sleep::wake_up(&mut (*wq).done, false);
                }
            }
            if sleep::interrupted() {
                return 0;
            }
            {
                let _guard = (*wq).lock.lock();
                if (*wq).list.is_null() {
                    sleep::wq_sleep(&mut (*wq).more, &(*wq).lock, false, true);
                }
            }
        }
    }
}

/// Sleep until `wq` is empty and no work is executing (flush_scheduled_work
/// semantics). False when interrupted by a fatal kill.
///
/// # Safety
/// Process context (sleeps); the caller must not hold `wq`'s lock.
#[allow(dead_code)] // M5: writeback flush points
pub unsafe fn flush_work(wq: *mut WorkqueueStruct) -> bool {
    loop {
        // SAFETY: process context; wq outlives the sleep (caller-owned).
        unsafe {
            let _guard = (*wq).lock.lock();
            if (*wq).list.is_null() && !(*wq).running {
                return true;
            }
            if sleep::interrupted() {
                return false;
            }
            sleep::wq_sleep(&mut (*wq).done, &(*wq).lock, true, false);
        }
    }
}
