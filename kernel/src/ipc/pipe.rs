//! pipe.rs: xv6 pipe.c port — anonymous pipes with sleep/wakeup flow
//! control and a 512-byte ring buffer. Sleep channels are the addresses of
//! p->nread / p->nwrite, exactly like xv6.

use core::ptr::{addr_of, addr_of_mut, null_mut, write};

use crate::fs::file_table::{filealloc, fileclose, File, FileState};
use crate::kernel::sched::{acquire, myproc, release, sleep, wakeup, RawLock};
use crate::mm::frame::{kalloc, kfree};

pub const PIPESIZE: usize = 512;

/// pipe.c `struct pipe` (allocated with kalloc; one page is plenty).
pub struct Pipe {
    pub lock: RawLock,
    pub data: [u8; PIPESIZE],
    pub nread: u32,     // number of bytes read
    pub nwrite: u32,    // number of bytes written
    pub readopen: i32,  // read fd is still open
    pub writeopen: i32, // write fd is still open
}

/// pipealloc: create a pipe, returning the read end in *f0 and the write
/// end in *f1. Both files take a reference the caller then owns.
/// Returns 0 on success, -1 on failure.
///
/// SAFETY: f0/f1 must point at live slots; on success the slots hold file
/// pointers with ref==1 owned by the caller.
pub unsafe fn pipealloc(f0: &mut *mut File, f1: &mut *mut File) -> i32 {
    *f0 = null_mut();
    *f1 = null_mut();

    // SAFETY: filealloc manages its own lock; we own the returned refs.
    *f0 = unsafe { filealloc() };
    if (*f0).is_null() {
        return -1;
    }
    *f1 = unsafe { filealloc() };
    if (*f1).is_null() {
        // SAFETY: drop the ref we own.
        unsafe { fileclose(*f0) };
        *f0 = null_mut();
        return -1;
    }

    let page = kalloc();
    if page.is_null() {
        // SAFETY: drop both refs we own.
        unsafe {
            fileclose(*f0);
            fileclose(*f1);
        }
        *f0 = null_mut();
        *f1 = null_mut();
        return -1;
    }
    let p = page as *mut Pipe;
    // SAFETY: fresh kalloc page; Pipe fits in one page; construct in place.
    unsafe {
        write(
            p,
            Pipe {
                lock: RawLock::new("pipe"),
                data: [0; PIPESIZE],
                nread: 0,
                nwrite: 0,
                readopen: 1,
                writeopen: 1,
            },
        );

        // f0: read end.
        (**f0).state = FileState::Pipe {
            pipe: p,
            writable: 0,
        };
        (**f0).readable = 1;
        (**f0).writable = 0;

        // f1: write end.
        (**f1).state = FileState::Pipe {
            pipe: p,
            writable: 1,
        };
        (**f1).readable = 0;
        (**f1).writable = 1;
    }
    0
}

/// pipeclose: close a pipe end; frees the pipe when both ends are closed.
///
/// SAFETY: `p` must be a live Pipe from pipealloc; must not be touched
/// again after the last end closes.
pub unsafe fn pipeclose(p: *mut Pipe, writable: i32) {
    let lk = unsafe { &*addr_of_mut!((*p).lock) };
    acquire(lk);
    if writable != 0 {
        unsafe {
            (*p).writeopen = 0;
            wakeup(addr_of!((*p).nread) as usize);
        }
    } else {
        unsafe {
            (*p).readopen = 0;
            wakeup(addr_of!((*p).nwrite) as usize);
        }
    }
    if unsafe { (*p).readopen } == 0 && unsafe { (*p).writeopen } == 0 {
        release(lk);
        // SAFETY: both ends closed; the kalloc page is exclusively ours.
        kfree(p as *mut u8);
    } else {
        release(lk);
    }
}

/// pipewrite: write n bytes from `addr` (USER pointer) into the pipe.
///
/// SAFETY: `addr` is a user VA within proc->sz (validated by argptr) while
/// this process's page table is active.
pub unsafe fn pipewrite(p: *mut Pipe, addr: *const u8, n: i32) -> i32 {
    let proc = myproc();
    let lk = unsafe { &*addr_of_mut!((*p).lock) };
    acquire(lk);
    for i in 0..n {
        //DOC: pipewrite-full
        while unsafe { (*p).nwrite } == unsafe { (*p).nread } + PIPESIZE as u32 {
            if unsafe { (*p).readopen } == 0 || unsafe { (*proc).killed } != 0 {
                release(lk);
                return -1;
            }
            unsafe {
                wakeup(addr_of!((*p).nread) as usize);
                sleep(addr_of!((*p).nwrite) as usize, lk); //DOC: pipewrite-sleep
            }
        }
        // SAFETY: user address; current (user) pgdir is active — see fn doc.
        let byte = unsafe { *addr.add(i as usize) };
        unsafe {
            let idx = ((*p).nwrite % PIPESIZE as u32) as usize;
            (*p).data[idx] = byte;
            (*p).nwrite += 1;
        }
    }
    unsafe { wakeup(addr_of!((*p).nread) as usize) }; //DOC: pipewrite-wakeup1
    release(lk);
    n
}

/// piperead: read up to n bytes from the pipe into `addr` (USER pointer).
///
/// SAFETY: see pipewrite.
pub unsafe fn piperead(p: *mut Pipe, addr: *mut u8, n: i32) -> i32 {
    let proc = myproc();
    let lk = unsafe { &*addr_of_mut!((*p).lock) };
    acquire(lk);
    //DOC: pipe-empty
    while unsafe { (*p).nread } == unsafe { (*p).nwrite } && unsafe { (*p).writeopen } != 0 {
        if unsafe { (*proc).killed } != 0 {
            release(lk);
            return -1;
        }
        unsafe { sleep(addr_of!((*p).nread) as usize, lk) }; //DOC: piperead-sleep
    }
    let mut i: i32 = 0;
    //DOC: piperead-copy
    while i < n {
        if unsafe { (*p).nread } == unsafe { (*p).nwrite } {
            break;
        }
        // SAFETY: user address; current (user) pgdir is active — see fn doc.
        unsafe {
            let idx = ((*p).nread % PIPESIZE as u32) as usize;
            *addr.add(i as usize) = (*p).data[idx];
            (*p).nread += 1;
        }
        i += 1;
    }
    unsafe { wakeup(addr_of!((*p).nwrite) as usize) }; //DOC: piperead-wakeup
    release(lk);
    i
}
