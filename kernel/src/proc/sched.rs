//! Scheduler (CFS, UP; plans/linux26/21 §2): the per-CPU idle thread
//! picks the leftmost vruntime entity off the ready tree and swtches
//! into it.
//!
//! Lock protocol (unchanged xv6 proc.c shape): the scheduler acquires
//! PTABLE_LOCK, marks the picked process RUNNING and swtches into it.
//! The process context then *owns* that acquire; it releases it either
//! through forkret (first entry) or by dropping its own guard after a
//! later swtch resumes it. sched() must run with interrupts off and the
//! ptable lock held.
//!
//! The CFS runqueue lives under the same PTABLE_LOCK (single UP lock;
//! no extra lock ordering): every enqueue/dequeue site holds it, the
//! running entity's slice is charged in process context (sched_raw /
//! yield / tick paths), and exec accounting runs at PIT tick
//! granularity (ADR-0016).

use core::arch::global_asm;
use core::ptr::addr_of_mut;

use super::proc::myproc;
use super::{cfs, fpu, preempt, rt, Context, Proc, ProcState, SchedPolicy, PTABLE_LOCK};
use crate::arch::x86_64::{cpu, io};
use crate::mm::pgtable;

global_asm!(include_str!("../arch/x86_64/swtch.S"), options(att_syntax));

unsafe extern "C" {
    fn _swtch(old_sp: *mut usize, new_sp: usize);
}

/// High-half stack for the scheduler thread: the boot stack's low identity
/// window is absent from user address spaces, so the scheduler leaves it
/// before the first switchuvm (see _sched_entry in swtch.S).
#[repr(C, align(16))]
pub struct SchedStack([u8; 16 * 1024]);

const _: () = assert!(core::mem::size_of::<SchedStack>() == 16 * 1024);

#[unsafe(no_mangle)]
#[allow(dead_code)]
pub static mut M1R_SCHED_STACK: SchedStack = SchedStack([0; 16 * 1024]);

unsafe extern "C" {
    /// Enter the scheduler loop on M1R_SCHED_STACK; never returns.
    fn _sched_entry() -> !;
}

#[unsafe(no_mangle)]
extern "C" fn scheduler_high_entry() -> ! {
    scheduler()
}

/// Enter the scheduler on its own high-half stack (never returns).
pub fn sched_entry() -> ! {
    // SAFETY: boot context, single CPU, interrupts off.
    unsafe { _sched_entry() };
}

/// Per-CPU state: the scheduler (idle) context and the running process.
pub struct Cpu {
    pub scheduler: Context,
    pub proc: *mut Proc,
}

pub static mut CPU0: Cpu = Cpu {
    scheduler: Context::ZERO,
    proc: core::ptr::null_mut(),
};

/// The one UP CFS runqueue (PTABLE_LOCK serializes every mutation).
static mut CFS_RQ: cfs::Runqueue = cfs::Runqueue::new();

/// The one UP RT runqueue (same lock; RT preempts CFS at pick time).
static mut RT_RQ: rt::RtRunqueue = rt::RtRunqueue::new();

/// # Safety
/// PTABLE_LOCK must be held by this context.
unsafe fn cfs_rq_mut() -> &'static mut cfs::Runqueue {
    unsafe { &mut *addr_of_mut!(CFS_RQ) }
}

/// # Safety
/// PTABLE_LOCK must be held by this context.
unsafe fn rt_rq_mut() -> &'static mut rt::RtRunqueue {
    unsafe { &mut *addr_of_mut!(RT_RQ) }
}

/// RT link -> containing proc (`container_of`).
fn rt_proc_of(link: *mut rt::RtLink) -> *mut Proc {
    let off = core::mem::offset_of!(Proc, rt_link);
    ((link as usize) - off) as *mut Proc
}

/// Class-aware enqueue (RUNNABLE ⟺ in its class queue).
///
/// # Safety
/// PTABLE_LOCK held; `p` is not queued in either class.
unsafe fn enqueue_runnable(p: *mut Proc) {
    // SAFETY: ptable lock held; p is ours.
    unsafe {
        if (*p).policy.is_rt() {
            rt::push_tail(rt_rq_mut(), &mut (*p).rt_link, (*p).rt_prio);
        } else {
            cfs::enqueue_entity(cfs_rq_mut(), &mut (*p).se);
        }
    }
}

/// Pick the next proc: highest RT first, else the CFS front.
///
/// # Safety
/// PTABLE_LOCK held; the returned proc is dequeued and RUNNING-able.
unsafe fn pick_next_proc() -> *mut Proc {
    // SAFETY: ptable lock held; RT list nodes are live.
    unsafe {
        let rtq = rt_rq_mut();
        if let Some(prio) = rt::front_prio(rtq) {
            let link = rt::front(rtq);
            rt::unlink(rtq, link, prio);
            let p = rt_proc_of(link);
            if (*p).policy == SchedPolicy::Rr {
                (*p).rt_ticks_left = rt::RT_RR_QUANTUM_TICKS;
            }
            return p;
        }
        let se = cfs::pick_next_entity(cfs_rq_mut());
        if se.is_null() {
            core::ptr::null_mut()
        } else {
            proc_of(se)
        }
    }
}

/// Monotonic clock from the time layer (TSC after calibration; jiffies
/// fallback early in boot). slice accounting precision tracks the
/// active clocksource.
fn now_ns() -> u64 {
    crate::kernel::time::clocksource::monotonic_ns()
}

/// Entity → containing proc (`container_of`).
fn proc_of(se: *mut cfs::SchedEntity) -> *mut Proc {
    let off = core::mem::offset_of!(Proc, se);
    ((se as usize) - off) as *mut Proc
}

/// Charge the outgoing current's exec slice into its entity.
///
/// # Safety
/// `p` is the current process on its own context.
unsafe fn charge_curr(p: *mut Proc) {
    // SAFETY: p is current; the runqueue is under our ptable lock.
    unsafe {
        let rq = cfs_rq_mut();
        cfs::update_curr(rq, &mut (*p).se, now_ns());
    }
}

/// Raw swtch wrapper (xv6: called with the ptable lock held and IF off).
///
/// # Safety
/// `old`/`new` are valid context slots; the caller owns the switch protocol.
pub unsafe fn swtch(old: *mut Context, new: *const Context) {
    unsafe { _swtch(addr_of_mut!((*old).sp), (*new).sp) };
}

/// Switch from the current process to the scheduler. Called with the
/// ptable lock held and interrupts disabled (yield/sleep/exit paths).
///
/// # Safety
/// See the module header; state must already be != RUNNING.
pub unsafe fn sched_raw() {
    let p = myproc();
    if p.is_null() {
        panic!("sched: no process");
    }
    let proc = &mut *p;
    if proc.state == ProcState::RUNNING {
        panic!("sched: running");
    }
    if io::intr_state() {
        panic!("sched: interrupts enabled");
    }
    preempt::assert_preemptible();
    charge_curr(p);
    // addr_of_mut! on a static-mut field is the lint-blessed pointer form.
    let sched_ctx = addr_of_mut!(CPU0.scheduler);
    // The process saves into its own context and resumes the scheduler's
    // (mirror of the scheduler-loop switch; old = current thread).
    unsafe { swtch(addr_of_mut!(proc.context), sched_ctx) };
}

/// Requeue the current process as RUNNABLE (optionally yield-boosted)
/// and switch to the scheduler.
///
/// # Safety
/// PTABLE_LOCK held by this context; `p` is the current process.
unsafe fn requeue_and_sched(p: *mut Proc, boost: bool) {
    // SAFETY: ptable lock held; p is current.
    unsafe {
        let rq = cfs_rq_mut();
        cfs::update_curr(rq, &mut (*p).se, now_ns());
        if boost && !(*p).policy.is_rt() {
            cfs::yield_entity(rq, &mut (*p).se);
        }
        (*p).state = ProcState::RUNNABLE;
        enqueue_runnable(p);
        sched_raw();
    }
}

/// Timer preemption from a user trap: give up the CPU for one round.
pub fn yield_cpu() {
    let p = myproc();
    if p.is_null() {
        return;
    }
    let guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is the current process.
    unsafe { requeue_and_sched(p, false) };
    drop(guard);
}

/// sched_yield(2): voluntary CFS yield — requeue behind the front.
pub fn sched_yield() {
    let p = myproc();
    if p.is_null() {
        return;
    }
    let guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is the current process.
    unsafe { requeue_and_sched(p, true) };
    drop(guard);
}

/// Timer tick on a user context: charge the slice, maybe preempt.
/// Kernel-context ticks never call this (preemption off, ADR-0016).
///
/// RT rules (21 §2): FIFO keeps the CPU unless a strictly higher RT
/// entity is runnable (preempted FIFO returns to its level's head); RR
/// rotates to the tail on quantum expiry; any runnable RT preempts a
/// CFS current unconditionally.
pub fn entity_tick() {
    let p = myproc();
    if p.is_null() {
        return;
    }
    let guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is the current process.
    unsafe {
        let rq = cfs_rq_mut();
        cfs::update_curr(rq, &mut (*p).se, now_ns());
        let rtq = rt_rq_mut();
        match (*p).policy {
            SchedPolicy::Fifo => {
                if rt::has_higher(rtq, (*p).rt_prio) {
                    rt::push_head(rtq, &mut (*p).rt_link, (*p).rt_prio);
                    (*p).state = ProcState::RUNNABLE;
                    sched_raw();
                }
            }
            SchedPolicy::Rr => {
                if rt::has_higher(rtq, (*p).rt_prio) {
                    rt::push_head(rtq, &mut (*p).rt_link, (*p).rt_prio);
                    (*p).state = ProcState::RUNNABLE;
                    sched_raw();
                } else if (*p).rt_ticks_left > 0 {
                    (*p).rt_ticks_left -= 1;
                } else {
                    // Quantum exhausted: rotate to the tail.
                    rt::push_tail(rtq, &mut (*p).rt_link, (*p).rt_prio);
                    (*p).state = ProcState::RUNNABLE;
                    sched_raw();
                }
            }
            SchedPolicy::Normal => {
                if !rtq.is_empty() || cfs::preempt_needed(rq, &(*p).se) {
                    requeue_and_sched(p, false);
                }
            }
        }
    }
    drop(guard);
}

/// Enqueue a fresh (never-run) entity as RUNNABLE (fork / userinit).
///
/// # Safety
/// PTABLE_LOCK held; `p` is EMBRYO and exclusively owned by the caller.
pub unsafe fn runnable_new(p: *mut Proc) {
    // SAFETY: ptable lock held; p is ours.
    unsafe {
        cfs::place_new_entity(cfs_rq_mut(), &mut (*p).se);
        (*p).state = ProcState::RUNNABLE;
        enqueue_runnable(p);
    }
}

/// Enqueue a woken sleeper as RUNNABLE (clamped up to min_vruntime).
///
/// # Safety
/// PTABLE_LOCK held; `p` is SLEEPING in this context.
pub unsafe fn runnable_wake(p: *mut Proc) {
    // SAFETY: ptable lock held; p is ours.
    unsafe {
        cfs::place_wake_entity(cfs_rq_mut(), &mut (*p).se);
        (*p).state = ProcState::RUNNABLE;
        enqueue_runnable(p);
    }
}

/// Change a proc's scheduling class/priority (sched_setscheduler /
/// sched_setparam backing). A RUNNABLE proc moves between its class
/// queues; a RUNNING or sleeping target just updates fields (the tick
/// path and later wake re-evaluate the class).
///
/// Caller validates `rt_prio` (0 for Normal, 1..=99 for RT).
///
/// # Safety
/// PTABLE_LOCK held by the caller.
pub unsafe fn set_sched_policy(p: *mut Proc, policy: SchedPolicy, rt_prio: i32) {
    // SAFETY: ptable lock held; p is a live ptable slot.
    unsafe {
        if (*p).state == ProcState::RUNNABLE {
            if (*p).policy.is_rt() {
                rt::unlink(rt_rq_mut(), &mut (*p).rt_link, (*p).rt_prio);
            } else {
                cfs::dequeue_entity(cfs_rq_mut(), &mut (*p).se);
            }
        }
        (*p).policy = policy;
        (*p).rt_prio = if policy.is_rt() { rt_prio } else { 0 };
        (*p).rt_ticks_left = 0;
        if (*p).state == ProcState::RUNNABLE {
            if !policy.is_rt() {
                // Entering CFS: clamp to the floor so a stale vruntime
                // cannot buy back the CPU (same shape as a wake).
                cfs::place_wake_entity(cfs_rq_mut(), &mut (*p).se);
            }
            enqueue_runnable(p);
        }
    }
}

/// switchuvm: install the process address space + kernel stack (IF off).
///
/// # Safety
/// Scheduler context: single CPU, interrupts off, ptable lock held.
pub unsafe fn switchuvm(p: *mut Proc) {
    let proc = &mut *p;
    let stack_top = super::proc::kstack_top_of(proc);
    cpu::set_kernel_stack(stack_top);
    if let Some(mm) = &proc.mm {
        io::set_cr3(mm.cr3_pa() as u64);
    } else {
        io::set_cr3(pgtable::boot_pml4_pa() as u64);
    }
}

/// switchkvm: back to the kernel-only address space (scheduler).
pub fn switchkvm() {
    io::set_cr3(pgtable::boot_pml4_pa() as u64);
    // SAFETY: scheduler context, interrupts off.
    unsafe {
        (*addr_of_mut!(CPU0)).proc = core::ptr::null_mut();
    }
}

/// First entry into a fresh process: swtch `ret`s here with the ptable
/// lock held by the scheduler. Release it (restoring IF), then return
/// through the seeded `_trapret_user` address into user mode.
#[unsafe(no_mangle)]
pub extern "C" fn forkret() {
    // SAFETY: the lock is held on this context (scheduler's acquire).
    unsafe { PTABLE_LOCK.raw_release() };
}

/// The scheduler loop: entered once from main and never returns.
pub fn scheduler() -> ! {
    loop {
        // Interrupts on while idle/looking, off while holding the lock.
        io::sti();
        let guard = PTABLE_LOCK.lock();
        let mut found = false;
        // SAFETY: ptable lock held for the whole pick/switch window.
        unsafe {
            let p = pick_next_proc();
            if !p.is_null() {
                found = true;
                (*p).state = ProcState::RUNNING;
                (*p).se.exec_start = now_ns();
                let cpu0 = addr_of_mut!(CPU0);
                (*cpu0).proc = p;
                switchuvm(p);
                fpu::restore(&(*p).fpu);
                swtch(&mut (*cpu0).scheduler, &(*p).context);
                fpu::save(&(*p).fpu);
                switchkvm();
            }
        }
        drop(guard);
        if !found {
            io::sti();
            io::hlt(); // wait for the timer to make someone runnable
        }
    }
}

/// Inline scheduler drive for boot-context suites (m3): runs runnable
/// procs on the caller's stack, borrowing CPU0.scheduler as the
/// swtch-back context, until `stop()` holds (a ptable-lock-free probe),
/// the step budget runs out, or nothing is runnable. Returns true iff
/// stop() held. Same pick/switch window as scheduler(), but it returns
/// instead of idling in sti/hlt — deterministic, IRQ-quiet stepping.
/// Must never run concurrently with scheduler().
pub fn sched_drive(stop: fn() -> bool, max_steps: usize) -> bool {
    let mut steps = 0usize;
    loop {
        if stop() {
            io::cli();
            return true;
        }
        io::sti();
        let guard = PTABLE_LOCK.lock();
        // SAFETY: ptable lock held for the whole pick/switch window
        // (same window as scheduler()).
        unsafe {
            let p = pick_next_proc();
            if p.is_null() {
                drop(guard);
                io::cli();
                return false;
            }
            (*p).state = ProcState::RUNNING;
            (*p).se.exec_start = now_ns();
            let cpu0 = addr_of_mut!(CPU0);
            (*cpu0).proc = p;
            switchuvm(p);
            fpu::restore(&(*p).fpu);
            swtch(&mut (*cpu0).scheduler, &(*p).context);
            fpu::save(&(*p).fpu);
            switchkvm();
        }
        drop(guard);
        steps += 1;
        if steps >= max_steps {
            io::cli();
            return false;
        }
    }
}
