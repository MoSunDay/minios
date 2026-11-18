//! file_table.rs: xv6 file.c port — the system file-descriptor table.
//!
//! `FileState` replaces the C `type` tag union: Closed == FD_NONE,
//! Inode{ip,off} == FD_INODE, Pipe{pipe,writable} == FD_PIPE. `readable`
//! stays a per-file flag (xv6 struct file); for pipes, `writable` is
//! carried in the enum variant, exactly the pair pipealloc sets.
//!
//! Deviation from xv6: fileread/filewrite take KERNEL buffers. The sys
//! layer (kernel/sysfile.rs) bounces user memory through the fixup-guarded
//! copyin/copyout, so a bad user pointer fails with EFAULT instead of
//! faulting inside readi/writei (this kernel's uaccess discipline).

use core::ptr::addr_of_mut;

use crate::fs::xv6fs::inode::{ilock, iput, iunlock, readi, stati, writei, Inode};
use crate::fs::xv6fs::layout::{Stat, LOGSIZE, NFILE};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::ipc::pipe::{pipeclose, piperead, pipewrite, Pipe};
use crate::klib::sync::Spinlock;

/// file.h enum: FD_NONE / FD_PIPE / FD_INODE.
#[derive(Clone, Copy)]
pub enum FileState {
    Closed,
    Inode { ip: *mut Inode, off: u32 },
    Pipe { pipe: *mut Pipe, writable: i32 },
}

/// file.h `struct file`.
#[derive(Clone, Copy)]
pub struct File {
    pub state: FileState,
    pub ref_: i32, // reference count
    pub readable: i32,
    pub writable: i32,
}

const FILE_CLOSED: File = File {
    state: FileState::Closed,
    ref_: 0,
    readable: 0,
    writable: 0,
};

/// file.c `ftable` (the lock is a sibling static like the icache split).
// SAFETY: slots are guarded by FTABLE_LOCK; the statics are unique.
pub static mut FTABLE: [File; NFILE] = [FILE_CLOSED; NFILE];
pub static FTABLE_LOCK: Spinlock<()> = Spinlock::new("ftable", ());

/// xv6 acquire()/release() spelling over the spinlock primitive.
/// SAFETY: caller must balance each acquire with a release of the same lock.
#[inline]
unsafe fn acquire(l: &Spinlock<()>) {
    l.raw_lock();
}

/// SAFETY: the lock must be held by this context.
#[inline]
unsafe fn release(l: &Spinlock<()>) {
    l.raw_release();
}

/// Allocate a file structure.
///
/// SAFETY: returns a raw pointer into the static ftable; holds FTABLE_LOCK
/// only while scanning (ref == 1 on success, like xv6).
pub unsafe fn filealloc() -> *mut File {
    // SAFETY: FTABLE_LOCK guards every slot; the raw pointer stays inside.
    unsafe {
        acquire(&FTABLE_LOCK);
        for i in 0..NFILE {
            let f = addr_of_mut!(FTABLE[i]);
            if (*f).ref_ == 0 {
                (*f).ref_ = 1;
                release(&FTABLE_LOCK);
                return f;
            }
        }
        release(&FTABLE_LOCK);
        core::ptr::null_mut()
    }
}

/// Increment ref count for file f.
///
/// SAFETY: `f` must come from filealloc (ref >= 1).
pub unsafe fn filedup(f: *mut File) -> *mut File {
    // SAFETY: FTABLE_LOCK guards the ref count.
    unsafe {
        acquire(&FTABLE_LOCK);
        if (*f).ref_ < 1 {
            release(&FTABLE_LOCK);
            panic!("filedup");
        }
        (*f).ref_ += 1;
        release(&FTABLE_LOCK);
        f
    }
}

/// Close file f. (Decrement ref count, close when reaches 0.)
///
/// SAFETY: `f` must come from filealloc/filedup; the pointer must not be
/// used again once this drops the last reference.
pub unsafe fn fileclose(f: *mut File) {
    let ff;
    // SAFETY: FTABLE_LOCK guards the ref/state transitions.
    unsafe {
        acquire(&FTABLE_LOCK);
        if (*f).ref_ < 1 {
            release(&FTABLE_LOCK);
            panic!("fileclose");
        }
        (*f).ref_ -= 1;
        if (*f).ref_ > 0 {
            release(&FTABLE_LOCK);
            return;
        }
        // Copy the state out under the lock, then close without it
        // (xv6 `ff` — pipeclose/iput may sleep).
        ff = (*f).state;
        (*f).ref_ = 0;
        (*f).state = FileState::Closed;
        (*f).readable = 0;
        (*f).writable = 0;
        release(&FTABLE_LOCK);
    }

    match ff {
        FileState::Pipe { pipe, writable } => unsafe { pipeclose(pipe, writable) },
        FileState::Inode { ip, .. } => unsafe {
            begin_op();
            iput(ip);
            end_op();
        },
        FileState::Closed => {}
    }
}

/// Get metadata about file f into `st` (a KERNEL address — sys_fstat
/// copies the result out to user space afterwards).
///
/// SAFETY: `st` must point at writable kernel memory of sizeof(Stat).
pub unsafe fn filestat(f: *mut File, st: *mut Stat) -> i32 {
    if let FileState::Inode { ip, .. } = unsafe { &(*f).state } {
        let ip = *ip;
        unsafe {
            ilock(ip);
            stati(ip, st);
            iunlock(ip);
        }
        return 0;
    }
    -1
}

/// Read from file f into a KERNEL buffer.
///
/// SAFETY: `dst` must be valid for `n` bytes of kernel memory; `f` from
/// filealloc.
pub unsafe fn fileread(f: *mut File, dst: *mut u8, n: i32) -> i32 {
    if unsafe { (*f).readable } == 0 {
        return -1;
    }
    // SAFETY: FileState is Copy; snapshot, advance the offset, write back.
    match unsafe { (*f).state } {
        FileState::Pipe { pipe, .. } => unsafe { piperead(pipe, dst, n) },
        FileState::Inode { ip, mut off } => unsafe {
            ilock(ip);
            let r = readi(ip, dst, off, n as u32);
            if r > 0 {
                off += r as u32;
                (*f).state = FileState::Inode { ip, off };
            }
            iunlock(ip);
            r
        },
        FileState::Closed => panic!("fileread"),
    }
}

/// Write to file f from a KERNEL buffer.
///
/// SAFETY: `src` must be valid for `n` bytes of kernel memory.
pub unsafe fn filewrite(f: *mut File, src: *const u8, n: i32) -> i32 {
    if unsafe { (*f).writable } == 0 {
        return -1;
    }
    // SAFETY: FileState is Copy; snapshot, advance the offset, write back.
    match unsafe { (*f).state } {
        FileState::Pipe { pipe, .. } => unsafe { pipewrite(pipe, src, n) },
        FileState::Inode { ip, mut off } => unsafe {
            // Write a few blocks at a time to avoid exceeding the maximum
            // log transaction size, including i-node, indirect block,
            // allocation blocks, and 2 blocks of slop for non-aligned
            // writes (file.c filewrite).
            let max = (((LOGSIZE - 1 - 1 - 2) / 2) * 512) as i32;
            let mut i: i32 = 0;
            while i < n {
                let mut n1 = n - i;
                if n1 > max {
                    n1 = max;
                }
                begin_op();
                ilock(ip);
                let r = writei(ip, src.add(i as usize), off, n1 as u32);
                if r > 0 {
                    off += r as u32;
                    (*f).state = FileState::Inode { ip, off };
                }
                iunlock(ip);
                end_op();

                if r < 0 {
                    break;
                }
                if r != n1 {
                    panic!("short filewrite");
                }
                i += r;
            }
            if i == n {
                n
            } else {
                -1
            }
        },
        FileState::Closed => panic!("filewrite"),
    }
}
