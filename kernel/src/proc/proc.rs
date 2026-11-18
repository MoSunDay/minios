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
        // Wake the parent (if any) — xv6 wakeup1 with the ptable held.
        super::sleep::wakeup1(super::ptable_chan());
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
/// Returns (pid, exit status) or (-1, 0) when childless/killed.
/// The lock is acquired once; sleep() transfers it across the swtch.
pub fn wait_status(want_pid: i32) -> (i32, i32) {
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
                    freeproc(p);
                    drop(_guard);
                    return (pid, xstate);
                }
            }
            if !have_kids || (*me).killed != 0 {
                drop(_guard);
                return (-1, 0);
            }
            // Sleep on the ptable channel; the scheduler takes the lock.
            super::sleep::sleep(super::ptable_chan(), &PTABLE_LOCK);
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
        if !(*me).cwd.is_null() {
            (*np).cwd = crate::fs::xv6fs::inode::idup((*me).cwd);
        }
    }
    let pid = unsafe { (*np).pid };
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; the slot leaves EMBRYO for the scheduler.
    unsafe {
        (*np).state = ProcState::RUNNABLE;
    }
    pid
}

/// Mark `pid` killed; the next trap/sched path exits it.
pub fn kill(pid: i32) -> i32 {
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held.
    unsafe {
        let table = super::ptable_mut();
        for i in 0..NPROC {
            let p = &mut table[i] as *mut Proc;
            if (*p).pid == pid {
                (*p).killed = 1;
                if (*p).state == ProcState::SLEEPING {
                    (*p).state = ProcState::RUNNABLE;
                }
                drop(_guard);
                return 0;
            }
        }
    }
    drop(_guard);
    -1
}

/// User-mode fault path: report already printed; exit without returning.
pub fn kill_cur_on_fault() -> ! {
    // SAFETY: process context.
    unsafe {
        let p = myproc();
        (*p).killed = 1;
    }
    exit(-1)
}

/// Grow the process heap by `n` (brk); returns the old break or 0 on error.
pub fn growproc(n: i64) -> usize {
    // SAFETY: process context.
    let p = myproc();
    let proc = unsafe { &mut *p };
    let mm = match proc.mm.as_mut() {
        Some(mm) => mm,
        None => return 0,
    };
    let old = mm.sz;
    let new = if n >= 0 {
        old.saturating_add(n as usize)
    } else {
        old.saturating_sub((-n) as usize)
    };
    // Keep the heap far below the user stack region.
    const HEAP_LIMIT: usize = 0x0000_1000_0000_0000;
    if new > HEAP_LIMIT {
        return 0;
    }
    if n >= 0 {
        if mm.alloc(old, new) == 0 {
            return 0;
        }
    } else if mm.dealloc(old, new) == 0 {
        return 0;
    }
    mm.sz = new;
    new
}

/// First user process: exec `image` as pid 1 named "init".
#[allow(dead_code)] // S4: the interactive boot execs the fs init image
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
                (*p).state = ProcState::RUNNABLE;
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
