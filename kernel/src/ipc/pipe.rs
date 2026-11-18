//! pipe.rs: xv6 pipe.c port — anonymous pipes with sleep/wakeup flow
//! control and a 512-byte ring buffer. Sleep channels are the addresses of
//! p->nread / p->nwrite, exactly like xv6.

use core::ptr::addr_of;

use crate::fs::file_table::{filealloc, fileclose, File, FileState};
use crate::klib::sync::Spinlock;
use crate::mm::frame::{kalloc, kfree};
use crate::proc::proc::myproc;
use crate::proc::sleep::{sleep, wakeup};

pub const PIPESIZE: usize = 512;

/// pipe.c `struct pipe` (allocated with kalloc; one page is plenty; the
/// embedded `Spinlock<()>` composes with sleep(chan, &p->lock) exactly
/// like xv6's sleep(&p->nread, &p->lock)).
pub struct Pipe {
    pub lock: Spinlock<()>,
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
    *f0 = core::ptr::null_mut();
    *f1 = core::ptr::null_mut();

    // SAFETY: filealloc manages its own lock; we own the returned refs.
    *f0 = unsafe { filealloc() };
    if (*f0).is_null() {
        return -1;
    }
    *f1 = unsafe { filealloc() };
    if (*f1).is_null() {
        // SAFETY: drop the ref we own.
        unsafe { fileclose(*f0) };
        *f0 = core::ptr::null_mut();
        return -1;
    }

    let page = kalloc();
    if page.is_null() {
        // SAFETY: drop both refs we own.
        unsafe {
            fileclose(*f0);
            fileclose(*f1);
        }
        *f0 = core::ptr::null_mut();
        *f1 = core::ptr::null_mut();
        return -1;
    }
    let p = page as *mut Pipe;
    // SAFETY: fresh kalloc page; Pipe fits in one page; construct in place.
    unsafe {
        core::ptr::write(
            p,
            Pipe {
                lock: Spinlock::new("pipe", ()),
                data: [0; PIPESIZE],
                nread: 0,
                nwrite: 0,
                readopen: 1,
                writeopen: 1,
            },
        );
    }
    // SAFETY: we exclusively own both fresh slots until the states are set.
    unsafe {
        (*(*f0)).readable = 1;
        (*(*f0)).writable = 0;
        (*(*f0)).state = FileState::Pipe {
            pipe: p,
            writable: 0,
        };
        (*(*f1)).readable = 0;
        (*(*f1)).writable = 1;
        (*(*f1)).state = FileState::Pipe {
            pipe: p,
            writable: 1,
        };
    }
    0
}

/// pipewrite: append `n` bytes from the KERNEL buffer `src`.
///
/// SAFETY: `p` from pipealloc; `src` valid for `n` bytes; caller context
/// must be a schedulable process (sleeps when the ring is full).
pub unsafe fn pipewrite(p: *mut Pipe, src: *const u8, n: i32) -> i32 {
    let mut i: i32 = 0;
    unsafe {
        (*p).lock.raw_lock();
        while i < n {
            let me = myproc();
            if !me.is_null() && (*me).killed != 0 {
                (*p).lock.raw_release();
                return -1;
            }
            while (*p).nwrite == (*p).nread + PIPESIZE as u32 {
                // doc: pipewrite going to sleep
                wakeup(addr_of!((*p).nread) as usize);
                sleep(addr_of!((*p).nwrite) as usize, &(*p).lock);
            }
            (*p).data[((*p).nwrite % PIPESIZE as u32) as usize] = *src.add(i as usize);
            (*p).nwrite += 1;
            i += 1;
            wakeup(addr_of!((*p).nread) as usize);
        }
        (*p).lock.raw_release();
    }
    n
}

/// piperead: read up to `n` bytes into the KERNEL buffer `dst`.
///
/// SAFETY: see pipewrite (sleeps until data or all writers closed).
pub unsafe fn piperead(p: *mut Pipe, dst: *mut u8, n: i32) -> i32 {
    unsafe {
        (*p).lock.raw_lock();
        let me = myproc();
        if !me.is_null() && (*me).killed != 0 {
            (*p).lock.raw_release();
            return -1;
        }
        while (*p).nread == (*p).nwrite {
            if (*p).writeopen == 0 {
                (*p).lock.raw_release();
                return 0; // EOF: write end closed
            }
            // doc: piperead going to sleep
            wakeup(addr_of!((*p).nwrite) as usize);
            sleep(addr_of!((*p).nread) as usize, &(*p).lock);
        }
        let mut i: i32 = 0;
        while i < n && (*p).nread != (*p).nwrite {
            *dst.add(i as usize) = (*p).data[((*p).nread % PIPESIZE as u32) as usize];
            (*p).nread += 1;
            i += 1;
        }
        wakeup(addr_of!((*p).nwrite) as usize);
        (*p).lock.raw_release();
        i
    }
}

/// pipeclose: close a pipe end (`writable` selects which). Frees the pipe
/// when both ends are gone.
///
/// SAFETY: `p` from pipealloc; must not be used again after the last close.
pub unsafe fn pipeclose(p: *mut Pipe, writable: i32) {
    unsafe {
        (*p).lock.raw_lock();
        if writable != 0 {
            (*p).writeopen = 0;
        } else {
            (*p).readopen = 0;
        }
        if (*p).readopen == 0 && (*p).writeopen == 0 {
            // SAFETY: both ends closed: this was the last reference.
            let page = p as *mut u8;
            (*p).lock.raw_release();
            core::ptr::drop_in_place(p);
            kfree(page);
            return;
        }
        wakeup(addr_of!((*p).nread) as usize);
        wakeup(addr_of!((*p).nwrite) as usize);
        (*p).lock.raw_release();
    }
}
