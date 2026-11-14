//! dump.rs: procdump + getcallerpcs, the xv6 proc.c/spinlock.c
//! diagnostics. Split from proc.rs to keep the process core under the
//! 400-line file limit.

use core::ptr::addr_of_mut;

use crate::kernel::sched::{ProcState, PTABLE};

/// spinlock.c getcallerpcs(): walk the saved %ebp chain collecting eips.
///
/// SAFETY: ebp chain must come from a live kernel stack (p->context->ebp).
unsafe fn getcallerpcs(ebp0: *const u32, pcs: &mut [u32; 10]) {
    use crate::arch::x86::mm::layout::{KERNBASE, PHYSTOP};
    let mut ebp = ebp0;
    let mut i = 0usize;
    while i < 10 {
        let a = ebp as usize;
        if a == 0 || a < KERNBASE || a >= KERNBASE.wrapping_add(PHYSTOP) {
            break;
        }
        // SAFETY: ebp was validated to point into kernel memory.
        unsafe {
            pcs[i] = *ebp.add(1); // saved %eip just above saved %ebp
            ebp = *ebp as *const u32; // saved %ebp of caller
        }
        i += 1;
    }
    while i < 10 {
        pcs[i] = 0;
        i += 1;
    }
}

/// Print a process listing to console. For debugging; runs when user types
/// ^P on console. No lock to avoid wedging a stuck machine further.
pub unsafe fn procdump() {
    static STATES: [&str; 6] = ["unused", "embryo", "sleep ", "runble", "run   ", "zombie"];
    let mut pcs = [0u32; 10];
    for i in 0..crate::kernel::sched::NPROC {
        // SAFETY: unlocked diagnostic read, as in xv6.
        let p = unsafe { addr_of_mut!(PTABLE[i]) };
        let state = unsafe { (*p).state };
        if state == ProcState::UNUSED {
            continue;
        }
        let idx = state as usize;
        let sname = if idx < STATES.len() {
            STATES[idx]
        } else {
            "???"
        };
        crate::printk!(
            "%d %s ",
            crate::lib::printk::Arg::D(unsafe { (*p).pid }),
            crate::lib::printk::Arg::S(Some(sname))
        );
        // Process name: raw bytes up to NUL (non-UTF8 degrades to ???).
        let mut end = 0usize;
        while end < 16 && unsafe { (*p).name[end] } != 0 {
            end += 1;
        }
        // SAFETY: bounded read of p->name from the ptable slot.
        let nb = unsafe { core::slice::from_raw_parts((*p).name.as_ptr(), end) };
        crate::console::write_str(core::str::from_utf8(nb).unwrap_or("???"));
        if state == ProcState::SLEEPING {
            // SAFETY: context belongs to a stopped (sleeping) process.
            let ebp = unsafe { (*(*p).context).ebp as *const u32 };
            unsafe { getcallerpcs(ebp.add(2), &mut pcs) }; // +2 skips sched's frame
            let mut j = 0usize;
            while j < 10 && pcs[j] != 0 {
                crate::printk!(" %p", crate::lib::printk::Arg::P(pcs[j] as usize));
                j += 1;
            }
        }
        crate::printk!("\n");
    }
}
