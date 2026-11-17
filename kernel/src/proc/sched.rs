//! Scheduler (xv6 round-robin, UP): the per-CPU idle thread scans the
//! ptable and swtches to runnable processes.
//!
//! Lock protocol (xv6 proc.c): the scheduler acquires PTABLE_LOCK, marks a
//! RUNNABLE process RUNNING and swtches into it. The process context then
//! *owns* that acquire; it releases it either through forkret (first entry)
//! or by dropping its own guard after a later swtch resumes it. sched()
//! must run with interrupts off and the ptable lock held.

use core::arch::global_asm;
use core::ptr::addr_of_mut;

use super::proc::myproc;
use super::{fpu, Context, Proc, ProcState, PTABLE_LOCK, NPROC};
use crate::arch::x86_64::{cpu, io};
use crate::mm::pgtable;

global_asm!(include_str!("../arch/x86_64/swtch.S"), options(att_syntax));

unsafe extern "C" {
    fn _swtch(old_sp: *mut usize, new_sp: usize);
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

/// Raw swtch wrapper (xv6: called with the ptable lock held and IF off).
///
/// # Safety
/// `old`/`new` are valid context slots; the caller owns the switch protocol.
pub unsafe fn swtch(old: *mut Context, new: *const Context) {
    unsafe { _swtch(addr_of_mut!((*old).sp), (*new).sp) }
}

/// Switch from the current process to the scheduler. Called with the ptable
/// lock held and interrupts disabled (yield/sleep/exit paths).
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
    // addr_of_mut! on a static-mut field is the lint-blessed pointer form.
    let sched_ctx = addr_of_mut!(CPU0.scheduler);
    unsafe { swtch(sched_ctx, &proc.context) };
}

/// Timer preemption from a user trap: give up the CPU for one round.
pub fn yield_cpu() {
    let p = myproc();
    if p.is_null() {
        return;
    }
    let guard = PTABLE_LOCK.lock();
    unsafe {
        (*p).state = ProcState::RUNNABLE;
        sched_raw();
    }
    drop(guard);
}

/// switchuvm: install the process address space + kernel stack (IF off).
///
/// # Safety
/// Caller holds the ptable lock (scheduler context switch-in).
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

/// First entry into a fresh process: swtch `ret`s here with the ptable lock
/// held by the scheduler. Release it (restoring IF), then return through
/// the seeded `_trapret_user` address into user mode.
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
        // SAFETY: ptable lock held for the whole scan/switch window.
        unsafe {
            let table = super::ptable_mut();
            for i in 0..NPROC {
                let p = &mut table[i] as *mut Proc;
                if (*p).state != ProcState::RUNNABLE {
                    continue;
                }
                found = true;
                (*p).state = ProcState::RUNNING;
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

