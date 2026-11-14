//! proc.rs: xv6 sysproc.c port — process-related system calls.
//!
//! EXPECTED from Agent A, crate::arch::x86::kernel::time (timer.c):
//!   pub static mut TICKS: u32;                          // tick counter
//!   pub static TICKS_LOCK: crate::kernel::sched::RawLock; // init at boot
//!   The timer tick handler must additionally
//!     crate::kernel::sched::wakeup(core::ptr::addr_of!(TICKS) as usize)
//!   so sys_sleep's sleep(&ticks, ...) wakes up, exactly like timerintr().

use core::ptr::addr_of;

use crate::arch::x86::kernel::time::{TICKS, TICKS_LOCK};
use crate::kernel::sched::{acquire, exit, fork, growproc, kill, myproc, release, sleep, wait};
use crate::usercall::argint;

pub unsafe fn sys_fork() -> i32 {
    unsafe { fork() }
}

pub unsafe fn sys_exit() -> i32 {
    unsafe { exit() } // does not return
}

pub unsafe fn sys_wait() -> i32 {
    unsafe { wait() }
}

pub unsafe fn sys_kill() -> i32 {
    let mut pid: i32 = 0;
    if unsafe { argint(0, &mut pid) } < 0 {
        return -1;
    }
    unsafe { kill(pid) }
}

pub unsafe fn sys_getpid() -> i32 {
    // SAFETY: running process.
    unsafe { (*myproc()).pid }
}

pub unsafe fn sys_sbrk() -> i32 {
    let mut n: i32 = 0;
    if unsafe { argint(0, &mut n) } < 0 {
        return -1;
    }
    let addr = unsafe { (*myproc()).sz };
    if unsafe { growproc(n) } < 0 {
        return -1;
    }
    addr as i32
}

pub unsafe fn sys_sleep() -> i32 {
    let mut n: i32 = 0;
    if unsafe { argint(0, &mut n) } < 0 {
        return -1;
    }
    acquire(&TICKS_LOCK);
    // SAFETY: ticks read under its lock.
    let ticks0 = unsafe { *addr_of!(TICKS) };
    loop {
        //SAFETY: arithmetic on the tick snapshot.
        let now = unsafe { *addr_of!(TICKS) };
        if now.wrapping_sub(ticks0) >= n as u32 {
            break;
        }
        if unsafe { (*myproc()).killed } != 0 {
            release(&TICKS_LOCK);
            return -1;
        }
        //DOC: sys_sleep-sleep
        unsafe { sleep(addr_of!(TICKS) as usize, &TICKS_LOCK) };
    }
    release(&TICKS_LOCK);
    0
}

/// Return how many clock tick interrupts have occurred since start.
pub unsafe fn sys_uptime() -> i32 {
    crate::arch::x86::kernel::time::ticks() as i32
}
