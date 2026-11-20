//! Kernel threads (21 §3): CPL0 tasks sharing the kernel address space
//! (mm == None -> boot PML4 via switchuvm). A kthread is an ordinary
//! Proc whose first-entry swtch image is re-seeded to enter the
//! trampoline with fn/arg in the callee-saved image registers instead
//! of the forkret -> _trapret_user -> user-trapframe path; the entry
//! releases the ptable lock the scheduler transferred (forkret
//! discipline) and runs interruptible on its own kstack. Returning
//! from the thread function exits through the standard zombie path;
//! boot-context drivers reap via kthread_reap (no parent waits).

use super::{proc, sched, Proc, ProcState};
use crate::arch::x86_64::io;
use crate::klib::sync::SpinlockGuard;

pub type KthreadFn = extern "C" fn(usize) -> i32;

extern "C" fn kthread_body(f: KthreadFn, arg: usize) -> ! {
    // Mirror forkret: release the scheduler's transferred acquire
    // (raw_release also pops the cli the scheduler's guard pushed).
    unsafe { super::PTABLE_LOCK.raw_release() };
    // Kthreads run interruptible in CPL0; traps land on this kstack.
    io::sti();
    let code = f(arg);
    proc::exit(code);
}

/// First entry into a fresh kthread: swtch `ret`s here with the ptable
/// lock held by the scheduler and fn/arg in r12/r13 (image slots 2/3).
/// Stack alignment is forced for the SysV call; the body never returns.
#[unsafe(no_mangle)]
#[unsafe(naked)]
unsafe extern "C" fn _kthread_trampoline() {
    {
        core::arch::naked_asm!(
            "mov rdi, r12",
            "mov rsi, r13",
            "and rsp, -16",
            "call {body}",
            "ud2",
            body = sym kthread_body,
        )
    }
}

/// Create an EMBRYO kernel thread (not yet runnable).
///
/// # Safety
/// Boot or process context, ptable lock free; caller owns the returned
/// slot until kthread_run hands it to the scheduler.
pub unsafe fn kthread_create(f: KthreadFn, arg: usize, name: &[u8]) -> *mut Proc {
    // SAFETY: ptable lock free here.
    let p = unsafe { proc::allocproc() };
    if p.is_null() {
        return p;
    }
    // SAFETY: exclusively owned EMBRYO slot; re-seed the swtch image.
    // Image memory order from ctx_sp is the REVERSE of the swtch.S
    // push order: [r15][r14][r13][r12][rbx][rbp][resume-rip] — the
    // allocproc comment lists push order, not slot order.
    unsafe {
        let sp = (*p).context.sp;
        *(sp as *mut usize).add(2) = arg; // r13 <- arg
        *(sp as *mut usize).add(3) = f as usize; // r12 <- fn
        *(sp as *mut usize).add(6) = _kthread_trampoline as *const () as usize;
        // Kernel-only: no user trapframe (fork() refuses a null tf).
        (*p).tf = core::ptr::null_mut();
        let dst = core::ptr::addr_of_mut!((*p).name) as *mut u8;
        core::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len().min(16));
    }
    p
}

/// Make a created kthread runnable (Linux kthread_run tail).
///
/// # Safety
/// `p` is an EMBRYO owned by the caller (from kthread_create).
pub unsafe fn kthread_run(p: *mut Proc) {
    let _guard: SpinlockGuard<'static, ()> = super::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; p is EMBRYO and ours.
    unsafe { sched::runnable_new(p) };
}

/// Reap an exited kthread from boot context (no parent can wait4 it).
/// Returns the thread's exit code.
///
/// # Safety
/// `p` came from kthread_create and has run to exit (ZOMBIE); no other
/// references exist. Must not be used once scheduler() owns reaping.
pub unsafe fn kthread_reap(p: *mut Proc) -> i32 {
    let _guard: SpinlockGuard<'static, ()> = super::PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; slot exclusively ours.
    unsafe {
        assert!(
            (*p).state == ProcState::ZOMBIE,
            "kthread_reap: not a zombie"
        );
        let code = (*p).xstate;
        proc::freeproc(p);
        code
    }
}
