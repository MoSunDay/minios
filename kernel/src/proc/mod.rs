//! proc: process table and per-process state (xv6 semantics, 64-bit).

pub mod exec;
pub mod fpu;
pub mod proc;
pub mod sched;
pub mod sleep;

use crate::kernel::traps::Trapframe;
use crate::klib::sync::Spinlock;
use crate::mm::vm::Mm;

pub use crate::arch::x86_64::cpu::{KSTACKSIZE, NPROC};

#[allow(dead_code)] // S4 fd table
pub const NOFILE: usize = 16;

/// swtch context: only the saved sp. The callee-saved registers live in the
/// 7-slot stack image pushed by swtch.S (see its header comment).
#[derive(Clone, Copy)]
pub struct Context {
    pub sp: usize,
}

impl Context {
    pub const ZERO: Context = Context { sp: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    UNUSED = 0,
    EMBRYO = 1,
    SLEEPING = 2,
    RUNNABLE = 3,
    RUNNING = 4,
    ZOMBIE = 5,
}

/// Per-process FPU area: fxsave64 needs 512 bytes, 16-byte aligned.
#[repr(C, align(16))]
pub struct FpuArea(pub [u8; 512]);

pub struct Proc {
    pub state: ProcState,
    pub pid: i32,
    pub parent: *mut Proc,
    pub mm: Option<Mm>,
    pub tf: *mut Trapframe,
    pub context: Context,
    pub chan: usize,
    pub killed: i32,
    pub xstate: i32,
    pub fpu: FpuArea,
    /// Open files (xv6 proc.h ofile; fd == index; null == closed).
    pub ofile: [*mut crate::fs::file_table::File; NOFILE],
    /// Current directory (xv6 proc.h cwd); set by userinit/chdir.
    pub cwd: *mut crate::fs::xv6fs::inode::Inode,
    pub name: [u8; 16],
}

/// Kernel stacks: a fixed reservation inside kernel .bss (no dynamic
/// mapping in M1R; NPROC * KSTACKSIZE = 1 MiB, page aligned per stack).
#[repr(C, align(4096))]
pub struct KstackBuf(pub [u8; KSTACKSIZE]);

pub static mut KSTACKS: [KstackBuf; NPROC] = [const { KstackBuf([0; KSTACKSIZE]) }; NPROC];

/// The xv6 ptable: one flat array + one lock whose ownership transfers
/// across scheduler swtches (see sched.rs and sleep.rs).
pub static mut PTABLE: [Proc; NPROC] = [const {
    Proc {
        state: ProcState::UNUSED,
        pid: 0,
        parent: core::ptr::null_mut(),
        mm: None,
        tf: core::ptr::null_mut(),
        context: Context::ZERO,
        chan: 0,
        killed: 0,
        xstate: 0,
        fpu: FpuArea([0; 512]),
        ofile: [core::ptr::null_mut(); NOFILE],
        cwd: core::ptr::null_mut(),
        name: [0; 16],
    }
}; NPROC];

pub static PTABLE_LOCK: Spinlock<()> = Spinlock::new("ptable", ());

/// PTABLE base address, also the wait() sleep channel (xv6 uses &ptable).
pub fn ptable_chan() -> usize {
    core::ptr::addr_of!(PTABLE) as usize
}

/// # Safety
/// Single-CPU kernel; callers must respect the ptable lock discipline.
pub unsafe fn ptable_mut() -> &'static mut [Proc; NPROC] {
    unsafe { &mut *core::ptr::addr_of_mut!(PTABLE) }
}
