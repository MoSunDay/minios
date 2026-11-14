//! fork.rs: xv6 proc.c fork/exit/wait/kill (split from proc.rs for the
//! 400-line file limit).

use core::ptr::addr_of_mut;

use crate::arch::x86::mm::pgtable as vm;
use crate::fs::file_table::{fileclose, filedup};
use crate::fs::xv6fs::inode::{idup, iput};
use crate::kernel::sched::{
    acquire, myproc, release, sleep, wakeup1, ProcState, INITPROC, PTABLE, PTABLE_LOCK,
};
use crate::lib::string::safestrcpy;
use crate::mm::frame::kfree;

/// Create a new process copying p as the parent. Sets up stack to return as
/// if from system call. Caller must set state of returned proc to RUNNABLE.
///
/// SAFETY: acts on the running process; caller must be in process context.
pub unsafe fn fork() -> i32 {
    let proc = myproc();
    // Allocate process.
    let np = crate::kernel::sched::allocproc();
    if np.is_null() {
        return -1;
    }

    // Copy process state from p.
    let pgdir = unsafe { vm::copyuvm((*proc).pgdir, (*proc).sz) };
    unsafe { (*np).pgdir = pgdir };
    if pgdir.is_null() {
        unsafe {
            kfree((*np).kstack);
            (*np).kstack = core::ptr::null_mut();
            (*np).state = ProcState::UNUSED;
        }
        return -1;
    }
    unsafe {
        (*np).sz = (*proc).sz;
        (*np).parent = proc;
        (*(*np).tf) = *(*proc).tf;

        // Clear %eax so that fork returns 0 in the child.
        (*(*np).tf).eax = 0;

        for i in 0..16 {
            // NOFILE == 16
            if !(*proc).ofile[i].is_null() {
                (*np).ofile[i] = filedup((*proc).ofile[i]);
            }
        }
        (*np).cwd = idup((*proc).cwd);

        let pid = (*np).pid;
        (*np).state = ProcState::RUNNABLE;
        safestrcpy((*np).name.as_mut_ptr(), (*proc).name.as_ptr(), 16);
        pid
    }
}

/// Exit the current process. Does not return. An exited process remains in
/// the zombie state until its parent calls wait() to find out it exited.
///
/// SAFETY: must be called from the running process; never returns.
pub unsafe fn exit() -> ! {
    let proc = myproc();
    if proc == unsafe { INITPROC } {
        panic!("init exiting");
    }

    // Close all open files.
    for fd in 0..16 {
        // NOFILE == 16
        // SAFETY: p->ofile slots owned by the exiting process.
        let f = unsafe { (*proc).ofile[fd] };
        if !f.is_null() {
            fileclose(f);
            unsafe { (*proc).ofile[fd] = core::ptr::null_mut() };
        }
    }

    unsafe { iput((*proc).cwd) };
    unsafe { (*proc).cwd = core::ptr::null_mut() };

    acquire(&PTABLE_LOCK);

    // Parent might be sleeping in wait().
    unsafe { wakeup1((*proc).parent as usize) };

    // Pass abandoned children to init.
    for i in 0..crate::kernel::sched::NPROC {
        // SAFETY: ptable lock held.
        let p = unsafe { addr_of_mut!(PTABLE[i]) };
        if unsafe { (*p).parent } == proc {
            unsafe {
                (*p).parent = INITPROC;
                if (*p).state == ProcState::ZOMBIE {
                    wakeup1(INITPROC as usize);
                }
            }
        }
    }

    // Jump into the scheduler, never to return.
    unsafe { (*proc).state = ProcState::ZOMBIE };
    crate::kernel::sched::sched();
    panic!("zombie exit");
}

/// Wait for a child process to exit and return its pid. Return -1 if this
/// process has no children.
///
/// SAFETY: acts on the running process; sleeps holding ptable lock.
pub unsafe fn wait() -> i32 {
    let proc = myproc();
    acquire(&PTABLE_LOCK);
    loop {
        // Scan through table looking for zombie children.
        let mut havekids = false;
        for i in 0..crate::kernel::sched::NPROC {
            // SAFETY: ptable lock held.
            let p = unsafe { addr_of_mut!(PTABLE[i]) };
            if unsafe { (*p).parent } != proc {
                continue;
            }
            havekids = true;
            if unsafe { (*p).state } == ProcState::ZOMBIE {
                // Found one.
                let pid = unsafe { (*p).pid };
                unsafe {
                    kfree((*p).kstack);
                    (*p).kstack = core::ptr::null_mut();
                    vm::freevm((*p).pgdir);
                    (*p).pgdir = core::ptr::null_mut();
                    (*p).state = ProcState::UNUSED;
                    (*p).pid = 0;
                    (*p).parent = core::ptr::null_mut();
                    (*p).name[0] = 0;
                    (*p).killed = 0;
                }
                release(&PTABLE_LOCK);
                return pid;
            }
        }

        // No point waiting if we don't have any children.
        if !havekids || unsafe { (*proc).killed } != 0 {
            release(&PTABLE_LOCK);
            return -1;
        }

        // Wait for children to exit. (See wakeup1 call in exit.)
        unsafe { sleep(proc as usize, &PTABLE_LOCK) }; //DOC: wait-sleep
    }
}

/// Kill the process with the given pid. Process won't exit until it returns
/// to user space (see trap in trap.c).
pub unsafe fn kill(pid: i32) -> i32 {
    acquire(&PTABLE_LOCK);
    for i in 0..crate::kernel::sched::NPROC {
        // SAFETY: ptable lock held.
        let p = unsafe { addr_of_mut!(PTABLE[i]) };
        unsafe {
            if (*p).pid == pid {
                (*p).killed = 1;
                // Wake process from sleep if necessary.
                if (*p).state == ProcState::SLEEPING {
                    (*p).state = ProcState::RUNNABLE;
                }
                release(&PTABLE_LOCK);
                return 0;
            }
        }
    }
    release(&PTABLE_LOCK);
    -1
}
