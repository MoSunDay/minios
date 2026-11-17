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
        (*p).cwd = 0;
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
    (*p).pid = 0;
    (*p).state = ProcState::UNUSED;
}

/// Current process exits with `status`; never returns.
pub fn exit(status: i32) -> ! {
    // SAFETY: process context on its own kernel stack.
    unsafe {
        let p = myproc();
        // S4: close files and cwd here.
        let _guard = PTABLE_LOCK.lock();
        // Wake the parent (if any) — xv6 wakeup1 with the ptable held.
        super::sleep::wakeup1(super::ptable_chan());
        (*p).state = ProcState::ZOMBIE;
        (*p).xstate = status;
        sched::sched_raw();
    }
    unreachable!("zombie rescheduled");
}

/// xv6 wait(): reap one zombie child, -1 when childless/killed.
/// The lock is acquired once; sleep() transfers it across the swtch.
#[allow(dead_code)] // S2 sys_wait4/sys_kill/sys_brk
pub fn wait() -> i32 {
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
                have_kids = true;
                if (*p).state == ProcState::ZOMBIE {
                    let pid = (*p).pid;
                    freeproc(p);
                    drop(_guard);
                    return pid;
                }
            }
            if !have_kids || (*me).killed != 0 {
                drop(_guard);
                return -1;
            }
            // Sleep on the ptable channel; the scheduler takes the lock.
            super::sleep::sleep(super::ptable_chan(), &PTABLE_LOCK);
        }
    }
}

/// Mark `pid` killed; the next trap/sched path exits it.
#[allow(dead_code)] // S2 sys_wait4/sys_kill/sys_brk
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
#[allow(dead_code)] // S2 sys_wait4/sys_kill/sys_brk
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
    new
}

/// First user process: exec `image` as pid 1. False when nothing to run.
pub fn userinit(image: &[u8]) -> bool {
    // SAFETY: boot context, ptable lock free.
    let p = unsafe { allocproc() };
    if p.is_null() {
        return false;
    }
    let name = b"init";
    // SAFETY: owned EMBRYO slot.
    unsafe {
        let dst = core::ptr::addr_of_mut!((*p).name) as *mut u8;
        core::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
        let tf = kstack_tf(p);
        match exec::exec_elf(p, &mut *tf, image, &[b"init"]) {
            Ok(()) => {
                let _guard = PTABLE_LOCK.lock();
                (*p).state = ProcState::RUNNABLE;
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
