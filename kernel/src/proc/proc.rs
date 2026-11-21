//! Process lifecycle (xv6 proc.c): allocation with seeded first-entry
//! frames, exit/wait reaping, kill, brk growth.

use core::ptr::{addr_of, addr_of_mut, null_mut};
use core::sync::atomic::{AtomicI32, Ordering};

use super::sched;
use super::{exec, fpu, Proc, ProcState, KSTACKS, KSTACKSIZE, NPROC, PTABLE, PTABLE_LOCK};
use crate::kernel::traps::{Trapframe, T_SYSCALL};

unsafe extern "C" {
    fn _trapret_user();
}

static NEXT_PID: AtomicI32 = AtomicI32::new(1);

/// The reaper process (init): orphans are handed to it on exit so no child
/// keeps a parent link into a ptable slot that freeproc is about to reuse.
/// Null before `userinit` succeeds (kernel-only suites never exit).
static mut INITPROC: *mut Proc = null_mut();

/// The init/reaper process, or null before userinit.
pub fn initproc() -> *mut Proc {
    // SAFETY: reading a raw-pointer static; no reference is formed.
    unsafe { core::ptr::read(core::ptr::addr_of!(INITPROC)) }
}

fn set_initproc(p: *mut Proc) {
    // SAFETY: boot context (userinit), ptable quiescent.
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(INITPROC), p) };
}

/// The process running on this CPU (null in scheduler context).
pub fn myproc() -> *mut Proc {
    // SAFETY: reading a pointer field; no reference to the static is made.
    unsafe { (*addr_of_mut!(sched::CPU0)).proc }
}

/// Top of this process's kernel stack (KSTACKS slot).
pub fn kstack_top_of(p: *const Proc) -> usize {
    let base = addr_of!(KSTACKS) as usize;
    let ptable_base = addr_of!(PTABLE) as usize;
    let idx = (p as usize - ptable_base) / core::mem::size_of::<Proc>();
    debug_assert!(idx < NPROC);
    base + idx * KSTACKSIZE + KSTACKSIZE
}

/// The seeded trapframe slot at the top of the kernel stack.
pub fn kstack_tf(p: *const Proc) -> *mut Trapframe {
    (kstack_top_of(p) - core::mem::size_of::<Trapframe>()) as *mut Trapframe
}

/// Allocate a slot, seed the first-entry swtch image + trapframe.
///
/// # Safety
/// Called at boot or with the ptable lock free; returns EMBRYO or null.
pub unsafe fn allocproc() -> *mut Proc {
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    let table = unsafe { super::ptable_mut() };
    for i in 0..NPROC {
        let p = &mut table[i] as *mut Proc;
        if (*p).state != ProcState::UNUSED {
            continue;
        }
        (*p).state = ProcState::EMBRYO;
        (*p).pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        (*p).parent = null_mut();
        (*p).killed = 0;
        (*p).xstate = 0;
        (*p).chan = 0;
        (*p).cwd = null_mut();
        for fd in 0..(*p).ofile.len() {
            (*p).ofile[fd] = null_mut();
        }
        (*p).name = [0; 16];
        (*p).mm = None;
        fpu::init_area(&mut (*p).fpu);

        // Seed the swtch image (7 slots) plus the first trapframe at the
        // kstack top: [rbp..r15=0][forkret][_trapret_user][Trapframe].
        let tf = kstack_tf(p);
        let ctx_sp = tf as usize - 64;
        // SAFETY: freshly owned kernel stack bytes.
        unsafe {
            core::ptr::write_bytes(ctx_sp as *mut u8, 0, 64 + core::mem::size_of::<Trapframe>())
        };
        unsafe {
            *(ctx_sp as *mut usize).add(6) = sched::forkret as *const () as usize;
            *(ctx_sp as *mut usize).add(7) = _trapret_user as *const () as usize;
        }
        (*p).context.sp = ctx_sp;
        (*p).tf = tf;
        return p;
    }
    core::ptr::null_mut()
}

/// Release a proc slot back to UNUSED (ptable lock held by caller).
///
/// # Safety
/// ptable lock held; `p` will never be scheduled again.
pub unsafe fn freeproc(p: *mut Proc) {
    if let Some(mm) = (*p).mm.as_mut() {
        mm.free();
    }
    (*p).mm = None;
    (*p).name = [0; 16];
    (*p).killed = 0;
    (*p).xstate = 0;
    (*p).chan = 0;
    (*p).parent = null_mut();
    (*p).tf = null_mut();
    // Files/cwd were closed by exit(); clear the slots for reuse.
    for fd in 0..(*p).ofile.len() {
        (*p).ofile[fd] = null_mut();
    }
    (*p).cwd = null_mut();
    (*p).pid = 0;
    // Fresh CFS identity and Normal policy for the reused slot (nice 0,
    // empty history; fork re-inherits policy/priority below).
    (*p).se = super::cfs::SchedEntity::new();
    (*p).policy = super::SchedPolicy::Normal;
    (*p).rt_prio = 0;
    (*p).rt_link = super::rt::RtLink::new();
    (*p).rt_ticks_left = 0;
    // The wait link must be unqueued: exit paths dequeue, but a killed
    // slot's link is guaranteed fresh regardless.
    (*p).wait_link = super::waitqueue::WaitLink::new();
    // Same for the alarm: exit() dequeued the timer, and slots reach
    // freeproc only through exit — the reset is belt-and-braces.
    (*p).alarm = crate::kernel::time::wheel::LrTimer::new();
    (*p).alarm_fired = 0;
    (*p).rcu_read_nesting = 0;
    // Signal state: fresh slot = all-default dispositions, empty sets
    // (fork re-inherits below; pending never crosses fork/exec).
    (*p).sig_pending = 0;
    (*p).sig_blocked = 0;
    (*p).sig_actions = [crate::kernel::sigset::SigAction::DFL; crate::kernel::sigset::NSIG];
    (*p).state = ProcState::UNUSED;
}

/// Current process exits with `status`; never returns.
pub fn exit(status: i32) -> ! {
    // SAFETY: process context on its own kernel stack.
    unsafe {
        let p = myproc();
        // Close all open files and drop the cwd (xv6 exit, before the
        // ptable lock: fileclose/iput may sleep and write the fs).
        for fd in 0..(*p).ofile.len() {
            if !(*p).ofile[fd].is_null() {
                crate::fs::file_table::fileclose((*p).ofile[fd]);
                (*p).ofile[fd] = core::ptr::null_mut();
            }
        }
        if !(*p).cwd.is_null() {
            crate::fs::xv6fs::log::begin_op();
            crate::fs::xv6fs::inode::iput((*p).cwd);
            crate::fs::xv6fs::log::end_op();
            (*p).cwd = core::ptr::null_mut();
        }
        let _guard = PTABLE_LOCK.lock();
        // Reparent orphans (xv6 exit): children must not keep a parent
        // link into the slot this exit frees. Without init (kernel-only
        // boot) they become unreapable zombies, never dangling links.
        {
            // SAFETY: ptable lock held.
            let table = super::ptable_mut();
            for i in 0..NPROC {
                let child = &mut table[i] as *mut Proc;
                if child != p && (*child).parent == p {
                    (*child).parent = initproc();
                }
            }
        }
        // SIGCHLD (21 §7): generate for the living parent before the
        // waiters wake — a handler-bearing parent sleeping in an
        // interruptible wait takes its sigreturn there; wait4 parents
        // reap through the wake below regardless of disposition.
        // SAFETY: ptable lock held (the sig_send_locked contract).
        let par = (*p).parent;
        if !par.is_null() {
            crate::kernel::signal::sig_send_locked(par, crate::kernel::sigset::SIGCHLD);
        }
        // Wake reaping parents (all waiters; ptable held).
        // SAFETY: ptable lock held; static access without a reference.
        let wq = core::ptr::addr_of_mut!(super::PTABLE_WAITQ);
        super::sleep::wake_up_locked(&mut *wq, false);
        (*p).state = ProcState::ZOMBIE;
        (*p).xstate = status;
        // Suite mode: pid 1 is the user driver; its exit is the verdict.
        if crate::kernel::suite::user_phase_armed() && p == initproc() {
            crate::kernel::suite::finish_driver(status);
        }
        sched::sched_raw();
    }
    unreachable!("zombie rescheduled");
}

/// xv6 wait(): reap one zombie child, -1 when childless/killed.
#[allow(dead_code)] // S2 sys_wait4/sys_kill/sys_brk
pub fn wait() -> i32 {
    wait_status(-1).0
}

/// wait4-style reap: pid > 0 targets that child, pid <= 0 takes any.
/// Returns (pid, exit status, exec ns) or (-1, 0, 0) when
/// childless/killed; the third element is the reaped child's
/// scheduler-charged CPU ns (rusage ru_utime source; charged at
/// tick/switch granularity). The lock is acquired once; sleep()
/// transfers it across the swtch.
pub fn wait_status(want_pid: i32) -> (i32, i32, u64) {
    let _guard = PTABLE_LOCK.lock();
    let me = myproc();
    loop {
        let mut have_kids = false;
        // SAFETY: ptable lock held (possibly inherited through sleep).
        unsafe {
            let table = super::ptable_mut();
            for i in 0..NPROC {
                let p = &mut table[i] as *mut Proc;
                if (*p).parent != me || p == me {
                    continue;
                }
                if want_pid > 0 && (*p).pid != want_pid {
                    continue;
                }
                have_kids = true;
                if (*p).state == ProcState::ZOMBIE {
                    let pid = (*p).pid;
                    let xstate = (*p).xstate;
                    let exec_ns = (*p).se.sum_exec_ns;
                    freeproc(p);
                    drop(_guard);
                    return (pid, xstate, exec_ns);
                }
            }
            if !have_kids || (*me).killed != 0 {
                drop(_guard);
                return (-1, 0, 0);
            }
            // An unblocked signal interrupts the reap wait without a
            // zombie to take: answer the -2 marker so sys_wait4 rewinds
            // (ERESTARTSYS) and the delivery point runs the handler,
            // then the call re-enters (21 §3 + s6).
            if crate::kernel::signal::pending_unblocked(me) {
                drop(_guard);
                return (-2, 0, 0);
            }
            // Sleep on the reap wait queue (interruptible: ERESTARTSYS
            // rewinds wait4, whose entry path sees killed and answers
            // -1 — the xv6-visible contract); the scheduler takes the
            // lock.
            // SAFETY: ptable lock held (ours); wait_link unqueued.
            super::sleep::wq_sleep(
                core::ptr::addr_of_mut!(super::PTABLE_WAITQ),
                &PTABLE_LOCK,
                true,
                false,
            );
        }
    }
}

/// fork(): duplicate the current process — address space (Mm::copy, no COW
/// until M2), trapframe (child rax=0), FPU area, parent link — and mark the
/// child RUNNABLE. Returns the child pid or -1 (ptable/full memory).
pub fn fork() -> i32 {
    let me = myproc();
    // SAFETY: process context on its own kernel stack; ptable lock free.
    let np = unsafe { allocproc() };
    if np.is_null() {
        return -1;
    }
    // SAFETY: the EMBRYO slot is exclusively ours until RUNNABLE is set.
    unsafe {
        (*np).mm = (*me).mm.as_ref().and_then(|mm| mm.copy());
        if (*me).mm.is_some() && (*np).mm.is_none() {
            let _guard = PTABLE_LOCK.lock();
            freeproc(np);
            return -1;
        }
        if (*me).tf.is_null() || (*np).tf.is_null() {
            // No user trapframe to duplicate (kernel-thread fork is
            // unsupported); fail cleanly instead of scheduling a zeroed
            // frame that would iretq to rip 0.
            let _guard = PTABLE_LOCK.lock();
            freeproc(np);
            return -1;
        }
        core::ptr::copy_nonoverlapping((*me).tf, (*np).tf, 1);
        // Child returns 0 from the (never-taken-in-parent) fork call.
        (*(*np).tf).rax = 0;
        (*np).parent = me;
        core::ptr::copy_nonoverlapping(
            core::ptr::addr_of!((*me).fpu) as *const u8,
            core::ptr::addr_of_mut!((*np).fpu) as *mut u8,
            core::mem::size_of::<super::FpuArea>(),
        );
        core::ptr::copy_nonoverlapping(
            (*me).name.as_ptr(),
            (*np).name.as_mut_ptr(),
            (*me).name.len(),
        );
        // Inherit open files (filedup) and the cwd (idup), xv6 fork.
        for fd in 0..(*me).ofile.len() {
            if !(*me).ofile[fd].is_null() {
                (*np).ofile[fd] = crate::fs::file_table::filedup((*me).ofile[fd]);
            }
        }
        // Linux fork semantics: the child inherits the scheduling class
        // and RT priority (freeproc reset the slot to Normal/0 above).
        (*np).policy = (*me).policy;
        (*np).rt_prio = (*me).rt_prio;
        // Linux fork semantics: the child inherits the block mask and
        // dispositions; pending does not carry over.
        (*np).sig_blocked = (*me).sig_blocked;
        core::ptr::copy_nonoverlapping(
            core::ptr::addr_of!((*me).sig_actions) as *const u8,
            core::ptr::addr_of_mut!((*np).sig_actions) as *mut u8,
            core::mem::size_of::<[crate::kernel::sigset::SigAction; crate::kernel::sigset::NSIG]>(),
        );
        if !(*me).cwd.is_null() {
            (*np).cwd = crate::fs::xv6fs::inode::idup((*me).cwd);
        }
    }
    let pid = unsafe { (*np).pid };
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; the slot leaves EMBRYO for the scheduler
    // and joins the CFS ready queue at min_vruntime.
    unsafe {
        sched::runnable_new(np);
    }
    pid
}

/// xv6 kill() body for a known slot: mark killed and wake a sleeper.
/// Safe from timer-interrupt context: ptable-lock holders run with
/// interrupts disabled, so acquiring the lock here cannot self-deadlock
/// (the same discipline time::tick's wakeup already relies on).
pub fn kill_proc(p: *mut Proc) {
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; the slot stays pinned by the caller.
    unsafe {
        (*p).killed = 1;
        if (*p).state == ProcState::SLEEPING {
            // SAFETY: ptable lock held; p sleeps in this context.
            sched::runnable_wake(p);
        }
    }
}

/// alarm(2) expiry (wheel callback, s6): mark alarm_fired and deliver
/// SIGALRM — ignored dispositions drop here, handlers/defaults land at
/// the sleeper's next user return (interruptible waits wake inside
/// sig_send).
pub fn alarm_fire(t: *mut crate::kernel::time::wheel::LrTimer) {
    let p = ((t as usize) - core::mem::offset_of!(Proc, alarm)) as *mut Proc;
    // SAFETY: the timer is the Proc's own alarm field; slot alive.
    unsafe {
        let _guard = PTABLE_LOCK.lock();
        (*p).alarm_fired = 1;
    }
    crate::kernel::signal::sig_send(p, crate::kernel::sigset::SIGALRM);
}

/// First user process: exec `image` as pid 1 named "init" (BootMode::Full).
pub fn userinit(image: &[u8]) -> bool {
    userinit_named(image, b"init", &[b"init"])
}

/// First user process with an explicit name/argv (suite driver, S4 init).
/// False when the image fails to load (caller decides the verdict).
pub fn userinit_named(image: &[u8], name: &[u8], argv: &[&[u8]]) -> bool {
    // SAFETY: boot context, ptable lock free.
    let p = unsafe { allocproc() };
    if p.is_null() {
        return false;
    }
    // SAFETY: owned EMBRYO slot.
    unsafe {
        let dst = core::ptr::addr_of_mut!((*p).name) as *mut u8;
        core::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len().min(16));
        let tf = kstack_tf(p);
        // The embedded seed image streams through the same reader surface
        // every later exec uses (bytes from a flat slice).
        let read = |dst: &mut [u8], off: u32| -> i32 {
            let start = off as usize;
            if start >= image.len() {
                return 0;
            }
            let n = dst.len().min(image.len() - start);
            dst[..n].copy_from_slice(&image[start..start + n]);
            n as i32
        };
        match exec::exec_elf(p, &mut *tf, &read, image.len(), argv) {
            Ok(()) => {
                // Root cwd (xv6 userinit's namei("/")); fds stay closed —
                // the driver opens the console like xv6 init.c.
                (*p).cwd = crate::fs::xv6fs::inode::iget(
                    crate::fs::xv6fs::layout::ROOTDEV,
                    crate::fs::xv6fs::layout::ROOTINO,
                );
                let _guard = PTABLE_LOCK.lock();
                // SAFETY: ptable lock held; p is EMBRYO, exclusively ours.
                sched::runnable_new(p);
                set_initproc(p);
                drop(_guard);
                true
            }
            Err(_) => {
                let _guard = PTABLE_LOCK.lock();
                freeproc(p);
                drop(_guard);
                false
            }
        }
    }
}

/// Re-export the segment selectors the seeded frame needs.
pub use crate::arch::x86_64::cpu::{SEL_UCODE64_RPL as USER_CS, SEL_UDATA_RPL as USER_SS};

/// Fill a trapframe for first entry (used by exec and userinit seeding).
pub fn seed_user_tf(tf: &mut Trapframe, rip: usize, rsp: usize) {
    *tf = Trapframe {
        rdi: 0,
        rsi: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        r11: 0,
        rbx: 0,
        rbp: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        vector: T_SYSCALL,
        err: 0,
        rip: rip as u64,
        cs: USER_CS,
        rflags: 0x202, // IF set: user code runs with interrupts on
        rsp: rsp as u64,
        ss: USER_SS,
    };
}
