//! trap: trap entry assembly (trapasm.S) + dispatch (trap.c), Rust-side.
//!
//! 256 vector stubs are emitted below with `global_asm!` — no build.rs:
//! `vector N` pushes the (possibly dummy) error code, then the vector
//! number, and jumps to `alltraps`, exactly like the generated
//! vectors.S. `alltraps` completes the Trapframe and calls `trap`.
//!
//! DIVERGENCE from trapasm.S: our frozen Trapframe struct has no
//! ds/es/fs/gs fields, so alltraps pushes only `pushal` after the
//! stub's {err, trapno} pair (xv6 also pushes the four segment registers
//! and reloads kernel segments + per-CPU GS). With minios's flat
//! segments — user DS/ES are DPL-3 aliases of the kernel's — a trap
//! handler may keep running on the user data selectors, so nothing is
//! lost; trapret correspondingly pops only `popal` + skips 8 bytes.
//!
//! Integration points (proc layer already on disk):
//!   crate::usercall::syscall(tf: &mut Trapframe) — T_SYSCALL (to land)
//!   crate::drivers::block::ide::ideintr()        — IRQ_IDE (on disk)
//!   crate::kernel::sched::{myproc, exit, yield_cpu, procdump}

use crate::arch::x86::kernel::idt;
use crate::arch::x86::kernel::pic;
use crate::arch::x86::kernel::time;
use crate::arch::x86::kernel::{
    Trapframe, DPL_USER, IRQ_COM1, IRQ_IDE, IRQ_KBD, IRQ_SPURIOUS, IRQ_TIMER, T_IRQ0, T_SYSCALL,
};
use crate::arch::x86::mm::pgtable::rcr2;
use crate::drivers::r#char::console;
use crate::kernel::sched::{exit, myproc, yield_cpu, ProcState};
use crate::lib::printk::Arg;

/// Vector numbers whose CPU entry already pushed an error code
/// (vectors.pl: 8, 10..14, 17): the stub pushes only the vector number.
macro_rules! vector_stubs_err {
    ($($n:literal),* $(,)?) => {
        concat!(
            $(
                concat!(
                    ".globl vector", stringify!($n),
                    "\nvector", stringify!($n), ":\n",
                    "  pushl $", stringify!($n), "\n  jmp alltraps\n"
                ),
            )*
        )
    };
}

/// Vector numbers with no CPU error code: the stub pushes a dummy 0
/// before the vector number so every frame has the {err, trapno} pair.
macro_rules! vector_stubs {
    ($($n:literal),* $(,)?) => {
        concat!(
            $(
                concat!(
                    ".globl vector", stringify!($n),
                    "\nvector", stringify!($n), ":\n",
                    "  pushl $0\n  pushl $", stringify!($n), "\n  jmp alltraps\n"
                ),
            )*
        )
    };
}

/// `.long vectorN` rows for the __vectors table idt.rs reads.
macro_rules! vector_addrs {
    ($($n:literal),* $(,)?) => {
        concat!($(concat!(".long vector", stringify!($n), "\n"),)*)
    };
}

core::arch::global_asm!(
    ".section .text\n",
    vector_stubs!(
        0, 1, 2, 3, 4, 5, 6, 7, 9, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
        32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
        55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
        78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
        100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
        118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135,
        136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153,
        154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171,
        172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189,
        190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207,
        208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225,
        226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243,
        244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255,
    ),
    vector_stubs_err!(8, 10, 11, 12, 13, 14, 17,),
    // alltraps (trapasm.S:6): build the trap frame and call trap(tf).
    // Frame from low to high: pushal regs, trapno, err, eip, cs, eflags,
    // [esp, ss from user] — matching struct Trapframe exactly.
    ".globl alltraps\nalltraps:\n",
    "  pushal\n",
    "  pushl %esp\n",
    "  call trap\n",
    "  addl $4, %esp\n",
    // trapret (trapasm.S:28): see the DIVERGENCE note above — no segment
    // pops, just popal + skip {trapno, err} + iret.
    ".globl trapret\ntrapret:\n",
    "  popal\n",
    "  addl $0x8, %esp  # trapno and errcode\n",
    "  iret\n",
    ".section .rodata\n.align 4\n.globl __vectors\n__vectors:\n",
    vector_addrs!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
        71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
        94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112,
        113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130,
        131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148,
        149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166,
        167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184,
        185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202,
        203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220,
        221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238,
        239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255,
    ),
    options(att_syntax)
);

/// tvinit (trap.c:21): build the IDT from the stub table and load IDTR.
pub fn tvinit() {
    idt::tvinit();
}

/// Trap entry point called by alltraps with `tf` pointing at the frame
/// it just pushed on the kernel stack (trap.c:41 `trap`).
#[no_mangle]
pub extern "C" fn trap(tf: *mut Trapframe) {
    // SAFETY: alltraps passes a pointer to the complete frame it built.
    let tf = unsafe { &mut *tf };
    dispatch(tf);
}

/// trap (trap.c:41): the actual dispatch, on a safe reference.
fn dispatch(tf: &mut Trapframe) {
    // xv6 global `proc`: the currently-running process, or null in the
    // scheduler. SAFETY: reads this CPU's slot.
    let proc = unsafe { myproc() };
    if tf.trapno == T_SYSCALL {
        if !proc.is_null() && unsafe { (*proc).killed } != 0 {
            unsafe { exit() };
        }
        crate::usercall::syscall(tf); // proc->tf = tf is implicit: by-ref arg
        if !proc.is_null() && unsafe { (*proc).killed } != 0 {
            unsafe { exit() };
        }
        return;
    }

    if tf.trapno == T_IRQ0 + IRQ_TIMER {
        // ticks++, wakeup(&ticks), PIC EOI — all inside time::timerintr.
        time::timerintr();
    } else if tf.trapno == T_IRQ0 + IRQ_IDE {
        crate::drivers::block::ide::ideintr();
        pic::eoi(IRQ_IDE);
    } else if tf.trapno == T_IRQ0 + IRQ_IDE + 1 {
        // Bochs generates spurious IDE1 interrupts.
    } else if tf.trapno == T_IRQ0 + IRQ_KBD {
        console::kbdintr();
        pic::eoi(IRQ_KBD);
    } else if tf.trapno == T_IRQ0 + IRQ_COM1 {
        console::uartintr();
        pic::eoi(IRQ_COM1);
    } else if tf.trapno == T_IRQ0 + 7 || tf.trapno == T_IRQ0 + IRQ_SPURIOUS {
        crate::printk!(
            "cpu0: spurious interrupt at %x:%x\n",
            Arg::X(tf.cs),
            Arg::X(tf.eip)
        );
        // PIC nuance (xv6 just calls lapiceoi): a *real* IRQ7 still needs
        // an EOI; a spurious one must not be acknowledged.
        if !pic::spurious(tf.trapno - T_IRQ0) {
            pic::eoi(tf.trapno - T_IRQ0);
        }
    } else if proc.is_null() || (tf.cs & 3) == 0 {
        // In kernel, it must be our mistake.
        crate::printk!(
            "unexpected trap %d from cpu 0 eip %x (cr2=0x%x)\n",
            Arg::D(tf.trapno as i32),
            Arg::X(tf.eip),
            Arg::P(rcr2())
        );
        panic!("trap");
    } else {
        // In user space, assume the process misbehaved and kill it.
        // (T_PGFLT from user code lands here, per the port plan.)
        crate::printk!(
            "trap %d err %d on cpu 0 eip 0x%x addr 0x%x--kill proc\n",
            Arg::D(tf.trapno as i32),
            Arg::D(tf.err as i32),
            Arg::X(tf.eip),
            Arg::P(rcr2())
        );
        // SAFETY: proc is non-null here (checked above).
        unsafe { (*proc).killed = 1 };
    }

    // Force process exit if it has been killed (trap.c:99..105).
    if !proc.is_null() && unsafe { (*proc).killed } != 0 && (tf.cs & 3) == DPL_USER {
        unsafe { exit() };
    }
    // Give up the CPU on a timer tick while a process is RUNNING.
    if !proc.is_null()
        && unsafe { (*proc).state } == ProcState::RUNNING
        && tf.trapno == T_IRQ0 + IRQ_TIMER
    {
        unsafe { yield_cpu() };
    }
}
