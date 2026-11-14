//! kernel/sched: xv6-public proc.c/param.h port — process table, per-CPU
//! state, raw spinlock, context switch, first user process and scheduler.
//!
//! Faithful structure: one global ptable lock (`PTABLE_LOCK`) + flat
//! `PTABLE: [Proc; NPROC]` array; per-proc state lives in the `Proc`
//! struct exactly like proc.h.
//!
//! ---------------------------------------------------------------------------
//! EXPECTED CONTRACTS (implemented by other agents; do not redefine here):
//!
//! From Agent A, `crate::arch::x86::mm::pgtable` (vm.c):
//!   pub unsafe fn setupkvm() -> *mut u32;                        // 0 on fail
//!   pub unsafe fn kvmalloc();                                    // install kpgdir
//!   pub unsafe fn switchkvm();                                   // back to kpgdir
//!   pub unsafe fn switchuvm(pgdir: *mut u32, tf: *const Trapframe);
//!   pub unsafe fn walkpgdir(pgdir: *mut u32, va: u32, alloc: i32) -> *mut u32;
//!   pub unsafe fn mappages(pgdir: *mut u32, va: u32, size: u32,
//!                          pa: u32, perm: u32) -> i32;
//!   pub unsafe fn allocuvm(pgdir: *mut u32, oldsz: u32, newsz: u32) -> u32;
//!   pub unsafe fn deallocuvm(pgdir: *mut u32, oldsz: u32, newsz: u32) -> u32;
//!   pub unsafe fn freevm(pgdir: *mut u32);
//!   pub unsafe fn copyuvm(pgdir: *mut u32, sz: u32) -> *mut u32; // 0 on fail
//!   pub unsafe fn clearpteu(pgdir: *mut u32, va: u32);
//!   pub unsafe fn copyout(pgdir: *mut u32, dstva: u32, src: *const u8,
//!                         len: u32) -> i32;
//!   pub unsafe fn copyin(pgdir: *mut u32, dst: *mut u8, srcva: u32,
//!                        len: u32) -> i32;
//!
//! From Agent A, `crate::arch::x86::kernel::time` (timer.c):
//!   pub static mut TICKS: u32;
//!   pub static TICKS_LOCK: crate::kernel::sched::RawLock;  // RawLock::new("ticks")
//!   The timer ISR must call crate::kernel::sched::wakeup(addr of TICKS).
//!
//! From Agent A trap.rs asm (trapasm.S): extern "C" label `trapret`
//! (pops a Trapframe then iret to user), reached via `ret` from forkret.
//!
//! From Agent C, `crate::fs::xv6fs::inode` — see crate::fs (fs/mod.rs).
//!
//! From Agent A, already on disk: crate::mm::frame::{kalloc, kfree},
//! crate::lib::string::*, crate::lib::printk::printk!, crate::lib::sync.
//! ---------------------------------------------------------------------------

pub mod dump;
pub mod fork;
pub mod initcode;
pub mod proc;
pub mod sleep;
pub mod swtch;

pub use dump::procdump;
pub use fork::{exit, fork, kill, wait};
pub use initcode::INITCODE;
pub use proc::{allocproc, growproc, myproc, sched, scheduler, yield_cpu, Proc, ProcState};
pub use sleep::{sleep, wakeup, wakeup1};
pub use swtch::swtch_slot;

use core::ptr::{addr_of_mut, null_mut};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::x86::kernel::Context;

// param.h
pub const NPROC: usize = 64; // maximum number of processes
pub const KSTACKSIZE: usize = 4096; // size of per-process kernel stack
pub const NCPU: usize = 8; // maximum number of CPUs
pub const MAXARG: usize = 32; // max exec arguments
pub const LOGSIZE: i32 = 10; // max data sectors in on-disk log

/// Proc placeholder import guard (never used; keeps the shape explicit).
/// mmu.h segment selectors used by userinit (SEG_UCODE<<3|3 == 0x1b etc).
pub const SEG_UCODE: u32 = 3;
pub const SEG_UDATA: u32 = 4;
pub const DPL_USER: u32 = 3;
/// mmu.h: EFLAGS interrupt-enable bit.
pub const FL_IF: u32 = 0x200;
/// mmu.h PTE permission bits (used by inituvm / exec).
pub const PTE_W: u32 = 0x002;
pub const PTE_U: u32 = 0x004;
/// mmu.h: mask off PTE flags to get physical address.
pub const PTE_ADDR: u32 = 0xFFFF_F000;

// ---------------------------------------------------------------------------
// Per-CPU state (proc.h `struct cpu`). xv6 reaches this via %gs; until the
// per-cpu segment machinery lands we keep a static array and a boot-CPU
// accessor (single-CPU bring-up is exact for cpus[0]).
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
pub struct Cpu {
    /// swtch() here to enter scheduler (holds scheduler's own context ptr).
    pub scheduler: *mut Context,
    /// The currently-running process; null while in scheduler.
    pub proc: *mut Proc,
}

pub static mut CPUS: [Cpu; NCPU] = [Cpu {
    scheduler: null_mut(),
    proc: null_mut(),
}; NCPU];

/// xv6 mycpu(): pointer to this CPU's state.
///
/// SAFETY: reads the static CPU array; correct on the boot CPU. Replace the
/// index with lapicid() lookup when Agent A's lapic module lands.
pub unsafe fn mycpu() -> *mut Cpu {
    addr_of_mut!(CPUS[0])
}

// ---------------------------------------------------------------------------
// RawLock: xv6 spinlock.c acquire/release as free functions.
//
// The scheduler hands ptable.lock between kernel contexts (scheduler
// acquires, the scheduled process releases it in forkret, and vice versa),
// which a Drop-guard API (lib::sync::Spinlock) cannot express. RawLock keeps
// xv6's manual acquire/release shape; interrupt state via lib::sync.
// ---------------------------------------------------------------------------
pub struct RawLock {
    pub name: &'static str,
    locked: AtomicU32,
}

impl RawLock {
    /// xv6 initlock(): const-initializable for statics.
    pub const fn new(name: &'static str) -> Self {
        RawLock {
            name,
            locked: AtomicU32::new(0),
        }
    }
}

/// Is the lock currently held? (xv6 holding(); owner CPU not tracked —
/// single-CPU boot makes held-by-other-CPU impossible.)
pub fn holding(lk: &RawLock) -> bool {
    lk.locked.load(Ordering::Acquire) == 1
}

/// xv6 acquire(): disable interrupts, spin on xchg, record nesting.
///
/// SAFETY: caller must not already hold `lk` (panics like xv6) and must
/// eventually release it on the same CPU.
pub unsafe fn acquire(lk: &RawLock) {
    crate::lib::sync::pushcli();
    if holding(lk) {
        panic!("acquire {}", lk.name);
    }
    while lk.locked.swap(1, Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
}

/// xv6 release(): store 0, then re-enable interrupts if outermost.
///
/// SAFETY: caller must hold `lk`.
pub unsafe fn release(lk: &RawLock) {
    if !holding(lk) {
        panic!("release {}", lk.name);
    }
    lk.locked.store(0, Ordering::Release);
    crate::lib::sync::popcli();
}

// ---------------------------------------------------------------------------
// The process table (proc.c `ptable`) + ptable lock + initproc + nextpid.
// ---------------------------------------------------------------------------
pub static PTABLE_LOCK: RawLock = RawLock::new("ptable");
pub static mut PTABLE: [Proc; NPROC] = [Proc::EMPTY; NPROC];
/// xv6 `initproc`: first user process, orphan re-parenting target.
pub static mut INITPROC: *mut Proc = null_mut();
/// xv6 `nextpid` (guarded by PTABLE_LOCK).
pub static mut NEXT_PID: i32 = 1;
