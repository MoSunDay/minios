//! usercall: xv6 syscall.c port — argument fetching for system calls and
//! the 1..21 dispatch table (numbers from xv6 syscall.h).
//!
//! ---------------------------------------------------------------------------
//! EXPECTED from Agent A in crate::arch::x86::mm::pgtable (see also the
//! full EXPECT block in kernel/sched/mod.rs):
//!   pub unsafe fn copyout(pgdir: *mut u32, dstva: u32,
//!                         src: *const u8, len: u32) -> i32;   // 0 ok, -1 bad
//!   pub unsafe fn copyin(pgdir: *mut u32, dst: *mut u8,
//!                        srcva: u32, len: u32) -> i32;        // 0 ok, -1 bad
//! fetchint/fetchstr below go through copyin (a hardening of xv6's direct
//! user-memory read, same observable semantics under xv6's "user can't
//! write kernel memory" invariant).
//! ---------------------------------------------------------------------------

#![allow(non_upper_case_globals)] // syscall.h names are kept verbatim

pub mod file;
pub mod fsops;
pub mod proc;

use crate::arch::x86::kernel::Trapframe;
use crate::arch::x86::mm::pgtable as vm;

use crate::kernel::sched::myproc;

// syscall.h: system call numbers.
pub const SYS_fork: i32 = 1;
pub const SYS_exit: i32 = 2;
pub const SYS_wait: i32 = 3;
pub const SYS_pipe: i32 = 4;
pub const SYS_read: i32 = 5;
pub const SYS_kill: i32 = 6;
pub const SYS_exec: i32 = 7;
pub const SYS_fstat: i32 = 8;
pub const SYS_chdir: i32 = 9;
pub const SYS_dup: i32 = 10;
pub const SYS_getpid: i32 = 11;
pub const SYS_sbrk: i32 = 12;
pub const SYS_sleep: i32 = 13;
pub const SYS_uptime: i32 = 14;
pub const SYS_open: i32 = 15;
pub const SYS_write: i32 = 16;
pub const SYS_mknod: i32 = 17;
pub const SYS_unlink: i32 = 18;
pub const SYS_link: i32 = 19;
pub const SYS_mkdir: i32 = 20;
pub const SYS_close: i32 = 21;

// Compile-time tie between the symbolic numbers and the dispatch slots.
const _: () = {
    assert!(SYS_fork == 1);
    assert!(SYS_exit == 2);
    assert!(SYS_wait == 3);
    assert!(SYS_pipe == 4);
    assert!(SYS_read == 5);
    assert!(SYS_kill == 6);
    assert!(SYS_exec == 7);
    assert!(SYS_fstat == 8);
    assert!(SYS_chdir == 9);
    assert!(SYS_dup == 10);
    assert!(SYS_getpid == 11);
    assert!(SYS_sbrk == 12);
    assert!(SYS_sleep == 13);
    assert!(SYS_uptime == 14);
    assert!(SYS_open == 15);
    assert!(SYS_write == 16);
    assert!(SYS_mknod == 17);
    assert!(SYS_unlink == 18);
    assert!(SYS_link == 19);
    assert!(SYS_mkdir == 20);
    assert!(SYS_close == 21);
};

/// Every sys_* has type SysFn: no real arguments — they fetch their own
/// arguments from the trap frame (xv6 convention).
pub type SysFn = unsafe fn() -> i32;

/// syscall.c syscalls[] dispatch table, indexed by syscall number.
pub static SYSCALLS: [Option<SysFn>; 22] = [
    None,                    // 0: unused
    Some(proc::sys_fork),    // 1
    Some(proc::sys_exit),    // 2
    Some(proc::sys_wait),    // 3
    Some(file::sys_pipe),    // 4
    Some(file::sys_read),    // 5
    Some(proc::sys_kill),    // 6
    Some(file::sys_exec),    // 7
    Some(file::sys_fstat),   // 8
    Some(fsops::sys_chdir),  // 9
    Some(file::sys_dup),     // 10
    Some(proc::sys_getpid),  // 11
    Some(proc::sys_sbrk),    // 12
    Some(proc::sys_sleep),   // 13
    Some(proc::sys_uptime),  // 14
    Some(fsops::sys_open),   // 15
    Some(file::sys_write),   // 16
    Some(fsops::sys_mknod),  // 17
    Some(fsops::sys_unlink), // 18
    Some(fsops::sys_link),   // 19
    Some(fsops::sys_mkdir),  // 20
    Some(file::sys_close),   // 21
];

/// Fetch the int at addr from the current process (0 ok, -1 bad).
///
/// SAFETY: `ip` must point at writable kernel memory.
pub unsafe fn fetchint(addr: u32, ip: *mut i32) -> i32 {
    let p = myproc();
    // SAFETY: p is the running process; bounds checked before copyin.
    let sz = unsafe { (*p).sz };
    if addr >= sz || addr.wrapping_add(4) > sz {
        return -1;
    }
    // SAFETY: ip is a live kernel word; 4 bytes copied within proc->sz.
    if unsafe { vm::copyin((*p).pgdir, ip as *mut u8, addr, 4) } < 0 {
        return -1;
    }
    0
}

/// Fetch the nul-terminated string at addr from the current process.
/// Doesn't copy the string — sets *pp to point at it (user VA, valid under
/// the active user page table). Returns the length excluding NUL, -1 bad.
///
/// SAFETY: `pp` must point at writable kernel memory.
pub unsafe fn fetchstr(addr: u32, pp: *mut *const u8) -> i32 {
    let p = myproc();
    // SAFETY: p is the running process.
    let sz = unsafe { (*p).sz };
    if addr >= sz {
        return -1;
    }
    // SAFETY: store the user pointer into the caller's slot.
    unsafe { *pp = addr as *const u8 };
    let mut s = addr;
    while s < sz {
        let mut c: u8 = 0;
        // SAFETY: byte-wise copyin stays within proc->sz.
        if unsafe { vm::copyin((*p).pgdir, &mut c, s, 1) } < 0 {
            return -1;
        }
        if c == 0 {
            return (s - addr) as i32;
        }
        s += 1;
    }
    -1
}

/// Fetch the nth 32-bit system call argument (0 ok, -1 bad).
///
/// SAFETY: `ip` must point at writable kernel memory.
pub unsafe fn argint(n: i32, ip: *mut i32) -> i32 {
    let p = myproc();
    // SAFETY: p->tf is the current syscall's trap frame.
    let esp = unsafe { (*(*p).tf).esp };
    unsafe { fetchint(esp + 4 + 4 * n as u32, ip) }
}

/// Fetch the nth word-sized system call argument as a pointer to a block
/// of memory of `size` bytes; checks it lies within the process address
/// space (0 ok, -1 bad).
///
/// SAFETY: `pp` must point at writable kernel memory.
pub unsafe fn argptr(n: i32, pp: *mut *mut u8, size: i32) -> i32 {
    let mut i: i32 = 0;
    if unsafe { argint(n, &mut i) } < 0 {
        return -1;
    }
    let p = myproc();
    // SAFETY: p is the running process.
    let sz = unsafe { (*p).sz };
    if i as u32 >= sz || (i as u32).wrapping_add(size as u32) > sz {
        return -1;
    }
    // SAFETY: store validated pointer into caller's slot.
    unsafe { *pp = i as usize as *mut u8 };
    0
}

/// Fetch the nth word-sized system call argument as a string pointer;
/// checks the string is nul-terminated (returns length, -1 bad).
///
/// SAFETY: `pp` must point at writable kernel memory.
pub unsafe fn argstr(n: i32, pp: *mut *const u8) -> i32 {
    let mut addr: i32 = 0;
    if unsafe { argint(n, &mut addr) } < 0 {
        return -1;
    }
    unsafe { fetchstr(addr as u32, pp) }
}

/// syscall(): user code made a system call with INT T_SYSCALL; the number
/// is in tf->eax and the return value goes back into tf->eax.
pub fn syscall(tf: &mut Trapframe) {
    let num = tf.eax as i32;
    if num > 0 && (num as usize) < SYSCALLS.len() {
        if let Some(f) = SYSCALLS[num as usize] {
            // SAFETY: sys_* fetch their own arguments from myproc()->tf,
            // which is this trap frame.
            tf.eax = unsafe { f() } as u32;
            return;
        }
    }
    //SAFETY: diagnostics only.
    let p = unsafe { myproc() };
    crate::printk!("%d ", crate::lib::printk::Arg::D(unsafe { (*p).pid }));
    // Process name: raw bytes up to NUL (non-UTF8 degrades).
    let mut end = 0usize;
    while end < 16 && unsafe { (*p).name[end] } != 0 {
        end += 1;
    }
    // SAFETY: bounded read of p->name.
    let nb = unsafe { core::slice::from_raw_parts((*p).name.as_ptr(), end) };
    crate::console::write_str(core::str::from_utf8(nb).unwrap_or("?"));
    crate::printk!(": unknown sys call %d\n", crate::lib::printk::Arg::D(num));
    tf.eax = -1_i32 as u32;
}
