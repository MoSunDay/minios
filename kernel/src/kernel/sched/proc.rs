//! proc.rs: xv6 proc.c core — process struct, allocproc, userinit,
//! scheduler/sched/yield, growproc. fork/exit/wait/kill live in fork.rs,
//! sleep/wakeup in sleep.rs, procdump/getcallerpcs in dump.rs.

use core::mem::size_of;
use core::ptr::{addr_of_mut, null_mut};

use crate::arch::x86::kernel::{Context, Trapframe};
use crate::arch::x86::mm::layout::v2p;
use crate::arch::x86::mm::pgtable as vm;
use crate::fs::file_table::File;
use crate::fs::xv6fs::inode::Inode;
use crate::kernel::sched::{
    acquire, holding, mycpu, release, swtch_slot, DPL_USER, FL_IF, INITCODE, INITPROC, KSTACKSIZE,
    NEXT_PID, PTABLE, PTABLE_LOCK, PTE_U, PTE_W, SEG_UCODE, SEG_UDATA,
};
use crate::lib::string::{memmove, memset, safestrcpy};
use crate::mm::frame::{kalloc, PGSIZE};

// trapasm.S label exported by trap.rs asm: pops a Trapframe and
// irets to user space. allocproc seeds a fresh kernel stack with its
// address so forkret "returns" into it.
extern "C" {
    fn trapret();
}

/// proc.h enum procstate.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    UNUSED = 0,
    EMBRYO = 1,
    SLEEPING = 2,
    RUNNABLE = 3,
    RUNNING = 4,
    ZOMBIE = 5,
}

/// proc.h `struct proc` (raw pointers + explicit refs, like xv6 C).
pub struct Proc {
    pub sz: u32,                // Size of process memory (bytes)
    pub pgdir: *mut u32,        // Page table
    pub kstack: *mut u8,        // Bottom of kernel stack for this process
    pub state: ProcState,       // Process state
    pub pid: i32,               // Process ID
    pub parent: *mut Proc,      // Parent process
    pub tf: *mut Trapframe,     // Trap frame for current syscall
    pub context: *mut Context,  // swtch() here to run process
    pub chan: usize,            // If non-zero, sleeping on chan
    pub killed: i32,            // If non-zero, have been killed
    pub ofile: [*mut File; 16], // Open files (NOFILE == 16)
    pub cwd: *mut Inode,        // Current directory
    pub name: [u8; 16],         // Process name (debugging)
}

impl Proc {
    /// Zeroed proc, as if the BSS-initialized ptable entry.
    pub const EMPTY: Proc = Proc {
        sz: 0,
        pgdir: null_mut(),
        kstack: null_mut(),
        state: ProcState::UNUSED,
        pid: 0,
        parent: null_mut(),
        tf: null_mut(),
        context: null_mut(),
        chan: 0,
        killed: 0,
        ofile: [null_mut(); 16],
        cwd: null_mut(),
        name: [0; 16],
    };
}

/// xv6 `proc` per-CPU variable: the currently-running process.
///
/// SAFETY: must run on a CPU that set cpu->proc (i.e. inside scheduler or
/// below); returns null while in the scheduler itself.
pub unsafe fn myproc() -> *mut Proc {
    // SAFETY: reads this CPU's slot; benign while in the scheduler (null).
    unsafe { (*mycpu()).proc }
}

//PAGEBREAK: 32
/// Look in the process table for an UNUSED proc. If found, change state to
/// EMBRYO and initialize state required to run in the kernel. Otherwise
/// return null.
///
/// SAFETY: touches the static ptable and allocates a kernel stack.
pub unsafe fn allocproc() -> *mut Proc {
    acquire(&PTABLE_LOCK);
    let mut p: *mut Proc = null_mut();
    for i in 0..crate::kernel::sched::NPROC {
        // SAFETY: distinct ptable slot, lock held.
        let q = unsafe { addr_of_mut!(PTABLE[i]) };
        if unsafe { (*q).state } == ProcState::UNUSED {
            p = q;
            break;
        }
    }
    if p.is_null() {
        release(&PTABLE_LOCK);
        return null_mut();
    }
    // SAFETY: lock still held; slot owned by us until release.
    unsafe {
        (*p).state = ProcState::EMBRYO;
        (*p).pid = NEXT_PID;
        NEXT_PID += 1;
    }
    release(&PTABLE_LOCK);

    // Allocate kernel stack.
    let kstack = kalloc();
    unsafe { (*p).kstack = kstack };
    if kstack.is_null() {
        unsafe { (*p).state = ProcState::UNUSED };
        return null_mut();
    }
    let mut sp = kstack as usize + KSTACKSIZE;

    // Leave room for trap frame.
    sp -= size_of::<Trapframe>();
    unsafe { (*p).tf = sp as *mut Trapframe };

    // Set up new context to start executing at forkret,
    // which returns to trapret.
    sp -= 4;
    // SAFETY: fresh kstack page, 4 bytes available below the trapframe.
    unsafe { *(sp as *mut u32) = trapret as *const () as usize as u32 };
    sp -= size_of::<Context>();
    unsafe {
        (*p).context = sp as *mut Context;
        memset(sp as *mut u8, 0, size_of::<Context>());
        (*(*p).context).eip = forkret as *const () as usize as u32;
    }
    p
}

/// vm.c inituvm(): map one page of user memory at va 0 and copy `init`
/// bytes in. Kept local because it only needs mappages (pgtable contract).
///
/// SAFETY: pgdir must be a live user page directory with va 0 unmapped.
unsafe fn inituvm(pgdir: *mut u32, init: *const u8, sz: u32) {
    let mem = kalloc();
    if mem.is_null() {
        panic!("inituvm: kalloc");
    }
    // SAFETY: fresh kalloc page; PGSIZE bytes are ours to write.
    memset(mem, 0, PGSIZE);
    if unsafe {
        vm::mappages(
            pgdir,
            0,
            PGSIZE as u32,
            v2p(mem as usize) as u32,
            PTE_W | PTE_U,
        )
    } < 0
    {
        panic!("inituvm: mappages");
    }
    // SAFETY: `mem` is a kernel-aliased fresh page; copy initcode into it.
    memmove(mem, init, sz as usize);
}

//PAGEBREAK: 32
/// Set up first user process.
///
/// SAFETY: called once from main() before scheduler(); allocates initproc.
pub unsafe fn userinit() {
    let p = allocproc();
    // SAFETY: initproc slot written once at boot.
    unsafe { INITPROC = p };

    let pgdir = unsafe { vm::setupkvm() };
    unsafe { (*p).pgdir = pgdir };
    if pgdir.is_null() {
        panic!("userinit: out of memory?");
    }
    unsafe { inituvm(pgdir, INITCODE.as_ptr(), INITCODE.len() as u32) };
    unsafe { (*p).sz = PGSIZE as u32 };

    // SAFETY: p->tf points into p's fresh kernel stack (allocproc).
    unsafe {
        memset((*p).tf as *mut u8, 0, size_of::<Trapframe>());
        (*(*p).tf).cs = (SEG_UCODE << 3) | DPL_USER;
        // NOTE: the frozen Trapframe has no ds/es words (they are folded
        // into the trap asm); xv6 also sets tf->ds/es = SEG_UDATA|DPL_USER,
        // which for user data reads/writes is exactly tf->ss below.
        (*(*p).tf).ss = (SEG_UDATA << 3) | DPL_USER;
        (*(*p).tf).eflags = FL_IF;
        (*(*p).tf).esp = PGSIZE as u32;
        (*(*p).tf).eip = 0; // beginning of initcode.S

        safestrcpy((*p).name.as_mut_ptr(), b"initcode\0".as_ptr(), 16);
        (*p).cwd = crate::fs::xv6fs::namei::namei(b"/\0".as_ptr() as *mut u8);
        (*p).state = ProcState::RUNNABLE;
    }
}

/// Grow current process's memory by n bytes. Return 0 on success, -1 on
/// failure.
///
/// SAFETY: acts on myproc(); may switch page directories.
pub unsafe fn growproc(n: i32) -> i32 {
    let p = myproc();
    // SAFETY: p is the running process.
    let sz0 = unsafe { (*p).sz };
    let newsz: u32;
    if n > 0 {
        newsz = unsafe { vm::allocuvm((*p).pgdir, sz0, sz0.wrapping_add(n as u32)) };
        if newsz == 0 {
            return -1;
        }
    } else if n < 0 {
        newsz = unsafe { vm::deallocuvm((*p).pgdir, sz0, sz0.wrapping_add(n as u32)) };
        if newsz == 0 {
            return -1;
        }
    } else {
        newsz = sz0;
    }
    unsafe {
        (*p).sz = newsz;
        vm::switchuvm((*p).pgdir, (*p).tf);
    }
    0
}

//PAGEBREAK: 42
/// Per-CPU process scheduler. Never returns. It loops, doing:
///  - choose a process to run
///  - swtch to start running that process
///  - eventually that process transfers control via swtch back to us.
///
/// SAFETY: called once per CPU after full kernel bring-up.
pub unsafe fn scheduler() -> ! {
    loop {
        // Enable interrupts on this processor.
        crate::lib::sync::intr_enable();

        // Loop over process table looking for process to run.
        let mut found = false;
        acquire(&PTABLE_LOCK);
        for i in 0..crate::kernel::sched::NPROC {
            // SAFETY: distinct ptable slot, ptable lock held.
            let p = unsafe { addr_of_mut!(PTABLE[i]) };
            if unsafe { (*p).state } != ProcState::RUNNABLE {
                continue;
            }
            found = true;

            // Switch to chosen process. It is the process's job
            // to release ptable.lock and then reacquire it
            // before jumping back to us.
            let c = mycpu();
            // SAFETY: scheduler-context bookkeeping on our CPU.
            unsafe {
                (*c).proc = p;
                vm::switchuvm((*p).pgdir, (*p).tf);
                (*p).state = ProcState::RUNNING;
                swtch_slot(addr_of_mut!((*c).scheduler), (*p).context);
                vm::switchkvm();

                // Process is done running for now.
                // It should have changed its p->state before coming back.
                (*c).proc = null_mut();
            }
        }
        release(&PTABLE_LOCK);

        if !found {
            // Idle: interrupts are on, next timer tick wakes us.
            // SAFETY: hlt with IF=1 is a normal x86 idle wait.
            unsafe { core::arch::asm!("hlt") };
        }
    }
}

/// Enter scheduler. Must hold only ptable.lock and have changed proc->state.
///
/// SAFETY: called with PTABLE_LOCK held and current proc state already
/// updated; never returns "normally" into a stale frame — the swtch target
/// resumes us later in scheduler().
pub unsafe fn sched() {
    let c = mycpu();
    let p = myproc();
    if !holding(&PTABLE_LOCK) {
        panic!("sched ptable.lock");
    }
    // SAFETY: pushcli nesting counter maintained by acquire/release.
    if crate::lib::sync::ncli() != 1 {
        panic!("sched locks");
    }
    if unsafe { (*p).state } == ProcState::RUNNING {
        panic!("sched running");
    }
    if crate::lib::sync::intr_state() & FL_IF != 0 {
        panic!("sched interruptible");
    }
    // SAFETY: sched() holds ptable; the pushcli depth and intena snapshot are
    // per-CPU and stay valid across swtch (xv6 keeps them in mycpu() too).
    unsafe {
        swtch_slot(addr_of_mut!((*p).context), (*c).scheduler);
    }
}

/// Give up the CPU for one scheduling round (xv6 `yield`; renamed because
/// `yield` is a Rust-2024 reserved keyword at call sites).
///
/// SAFETY: called from a trap handler on the running process.
pub unsafe fn yield_cpu() {
    acquire(&PTABLE_LOCK);
    let p = myproc();
    // SAFETY: ptable lock held; state change visible to scheduler.
    unsafe { (*p).state = ProcState::RUNNABLE };
    sched();
    release(&PTABLE_LOCK);
}

/// A fork child's very first scheduling by scheduler() will swtch here.
/// "Return" to user space (into trapret, planted by allocproc).
#[no_mangle]
pub unsafe extern "C" fn forkret() {
    use core::sync::atomic::{AtomicU32, Ordering};
    static FIRST: AtomicU32 = AtomicU32::new(1);

    // Still holding ptable.lock from scheduler.
    release(&PTABLE_LOCK);

    if FIRST.swap(0, Ordering::SeqCst) == 1 {
        // xv6: initlog() — must run in process context because it sleeps
        // on disk I/O (recover_from_log -> bread -> iderw -> sleep).
        crate::fs::xv6fs::log::initlog(crate::fs::xv6fs::layout::ROOTDEV);
    }
}
