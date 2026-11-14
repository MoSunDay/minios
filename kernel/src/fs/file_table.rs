//! file_table.rs: xv6 file.c port — the system file-descriptor table.
//!
//! `FileState` replaces the C `type` tag union: Closed == FD_NONE,
//! Inode{ip,off} == FD_INODE, Pipe{pipe,writable} == FD_PIPE. `readable`
//! stays a per-file flag (xv6 struct file); for pipes, `writable` is
//! carried in the enum variant, exactly the pair pipealloc sets.

use core::ptr::{addr_of_mut, null_mut, read};

use crate::fs::xv6fs::inode::{ilock, iput, iunlock, readi, stati, writei, Inode};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::fs::{Stat, NFILE};
use crate::ipc::pipe::{pipeclose, piperead, pipewrite, Pipe};
use crate::kernel::sched::LOGSIZE;
use crate::kernel::sched::{acquire, release, RawLock};

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

/// file.c `ftable`.
pub static mut FTABLE: [File; NFILE] = [File {
    state: FileState::Closed,
    ref_: 0,
    readable: 0,
    writable: 0,
}; NFILE];
pub static FTABLE_LOCK: RawLock = RawLock::new("ftable");

/// Allocate a file structure.
///
/// SAFETY: returns a raw pointer into the static ftable; holds FTABLE_LOCK
/// only while scanning (ref == 1 on success, like xv6).
pub unsafe fn filealloc() -> *mut File {
    acquire(&FTABLE_LOCK);
    for i in 0..NFILE {
        // SAFETY: distinct slot, lock held.
        let f = unsafe { addr_of_mut!(FTABLE[i]) };
        if unsafe { (*f).ref_ } == 0 {
            unsafe { (*f).ref_ = 1 };
            release(&FTABLE_LOCK);
            return f;
        }
    }
    release(&FTABLE_LOCK);
    null_mut()
}

/// Increment ref count for file f.
///
/// SAFETY: `f` must come from filealloc (ref >= 1).
pub unsafe fn filedup(f: *mut File) -> *mut File {
    acquire(&FTABLE_LOCK);
    if unsafe { (*f).ref_ } < 1 {
        release(&FTABLE_LOCK);
        panic!("filedup");
    }
    unsafe { (*f).ref_ += 1 };
    release(&FTABLE_LOCK);
    f
}

/// Close file f. (Decrement ref count, close when reaches 0.)
///
/// SAFETY: `f` must come from filealloc/filedup; the pointer must not be
/// used again once this drops the last reference.
pub unsafe fn fileclose(f: *mut File) {
    acquire(&FTABLE_LOCK);
    if unsafe { (*f).ref_ } < 1 {
        release(&FTABLE_LOCK);
        panic!("fileclose");
    }
    unsafe { (*f).ref_ -= 1 };
    if unsafe { (*f).ref_ } > 0 {
        release(&FTABLE_LOCK);
        return;
    }
    // Copy the state out under the lock, then close without it (xv6 `ff`).
    // SAFETY: snapshot of the slot we own exclusively (ref hit 0).
    let ff = unsafe { read(&(*f).state) };
    unsafe {
        (*f).ref_ = 0;
        (*f).state = FileState::Closed;
        (*f).readable = 0;
        (*f).writable = 0;
    }
    release(&FTABLE_LOCK);

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

/// Read from file f.
///
/// SAFETY: `addr` is a USER virtual address (myproc()'s page table is
/// active); it was validated by argptr to lie within proc->sz.
pub unsafe fn fileread(f: *mut File, addr: *mut u8, n: i32) -> i32 {
    if unsafe { (*f).readable } == 0 {
        return -1;
    }
    // SAFETY: dispatch on the snapshot of f's state.
    match unsafe { &mut (*f).state } {
        FileState::Pipe { pipe, .. } => unsafe { piperead(*pipe, addr, n) },
        FileState::Inode { ip, ref mut off } => unsafe {
            let ip = *ip;
            ilock(ip);
            let r = readi(ip, addr, *off, n as u32);
            if r > 0 {
                *off += r as u32;
            }
            iunlock(ip);
            r
        },
        FileState::Closed => panic!("fileread"),
    }
}

/// Write to file f.
///
/// SAFETY: `addr` is a USER virtual address; see fileread.
pub unsafe fn filewrite(f: *mut File, addr: *const u8, n: i32) -> i32 {
    if unsafe { (*f).writable } == 0 {
        return -1;
    }
    match unsafe { &mut (*f).state } {
        FileState::Pipe { pipe, .. } => unsafe { pipewrite(*pipe, addr, n) },
        FileState::Inode { ip, ref mut off } => unsafe {
            // Write a few blocks at a time to avoid exceeding the maximum
            // log transaction size, including i-node, indirect block,
            // allocation blocks, and 2 blocks of slop for non-aligned
            // writes. This really belongs lower down, since writei() might
            // be writing a device like the console.
            let max = ((LOGSIZE - 1 - 1 - 2) / 2) * 512;
            let mut i: i32 = 0;
            while i < n {
                let mut n1 = n - i;
                if n1 > max {
                    n1 = max;
                }
                let ip = *ip;

                begin_op();
                ilock(ip);
                let r = writei(ip, addr.add(i as usize), *off, n1 as u32);
                if r > 0 {
                    *off += r as u32;
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
