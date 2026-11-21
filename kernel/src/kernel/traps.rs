//! Trap frames, IDT wiring, and the shared trap/syscall dispatchers.

use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, Ordering};

use super::fixup;
use super::oops;
pub use super::tf_layout::Trapframe;
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::io;
use crate::console;

global_asm!(
    include_str!("../arch/x86_64/trapasm.S"),
    options(att_syntax)
);
global_asm!(
    include_str!("../arch/x86_64/trapcommon.S"),
    options(att_syntax)
);

/// Vector numbers used by this kernel.
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_DIVIDE: u64 = 0;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_DEBUG: u64 = 1;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_NMI: u64 = 2;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_BRKPT: u64 = 3;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_OFLOW: u64 = 4;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_BOUND: u64 = 5;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_ILLOP: u64 = 6;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_DEVICE: u64 = 7;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_DBLFLT: u64 = 8;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_TSS: u64 = 10;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_SEGNP: u64 = 11;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_STACK: u64 = 12;
pub const T_GPFLT: u64 = 13;
pub const T_PGFLT: u64 = 14;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_FPERR: u64 = 16;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_ALIGN: u64 = 17;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_MCE: u64 = 18;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const T_SIMDERR: u64 = 19;
pub const T_SYSCALL: u64 = 0x80;
pub const IRQ_BASE: u64 = 32;
pub const IRQ_TIMER: u64 = IRQ_BASE + 0;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const IRQ_KBD: u64 = IRQ_BASE + 1;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const IRQ_COM1: u64 = IRQ_BASE + 4;
#[allow(dead_code)] // S2 dispatch / S4 devices
pub const IRQ_IDE: u64 = IRQ_BASE + 14;
pub const IRQ_ERROR: u64 = IRQ_BASE + 19;
pub const IRQ_SPURIOUS: u64 = IRQ_BASE + 7;

#[repr(C, packed)]
struct IdtGate {
    low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    middle: u16,
    high: u32,
    reserved: u32,
}

#[repr(C)]
struct Idt {
    gates: [IdtGate; 256],
}

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base: u64,
}

static mut IDT: Idt = Idt {
    gates: [const {
        IdtGate {
            low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            middle: 0,
            high: 0,
            reserved: 0,
        }
    }; 256],
};
static mut IDTR: IdtDescriptor = IdtDescriptor { limit: 0, base: 0 };

unsafe extern "C" {
    static _vector_table: [*const (); 256];
}

/// Address of the asm syscall entry installed in LSTAR.
pub fn syscall_entry() -> usize {
    unsafe extern "C" {
        fn _syscall_entry();
    }
    _syscall_entry as *const () as usize
}

/// Build the IDT from the asm vector table and load it. One-shot at boot.
pub fn init_idt() {
    // SAFETY: one-shot initialization before interrupts are unmasked.
    unsafe {
        for i in 0..256 {
            let addr = _vector_table[i] as u64;
            IDT.gates[i] = IdtGate {
                low: (addr & 0xFFFF) as u16,
                selector: cpu::SEL_KCODE,
                ist: 0,
                type_attr: 0x8E, // present | interrupt gate | DPL 0
                middle: ((addr >> 16) & 0xFFFF) as u16,
                high: (addr >> 32) as u32,
                reserved: 0,
            };
        }
        IDTR.limit = (core::mem::size_of::<Idt>() - 1) as u16;
        IDTR.base = core::ptr::addr_of_mut!(IDT) as u64;
        let idtr = core::ptr::addr_of_mut!(IDTR);
        core::arch::asm!("lidt ({0})", in(reg) idtr as u64, options(att_syntax, nostack));
    }
    cpu::idt_mark_ready();
}

/// Test hook: when nonzero, the shared return path takes iretq instead of
/// sysretq (m1r-fault boot mode only). Declared `static mut` for the asm
/// symbol reference in trapret_common.
#[unsafe(no_mangle)]
pub static mut m1r_force_iret: u8 = 0;

/// Expected-fault observer switch (m1r-fault boot mode only): when set, a
/// fixup-table hit first prints one structured M1RFault line (40 §2.1.6);
/// it never changes fixup/recovery semantics, and ordinary suites keep it
/// off so their serial output stays untouched.
pub static OBSERVE_FAULTS: AtomicBool = AtomicBool::new(false);

/// Shared kernel-fault path: fixup recovery (optionally observed) or oops.
fn fixup_or_oops(tf: &mut Trapframe, why: &str) {
    match fixup::lookup(tf.rip) {
        Some(fix_rip) => {
            if OBSERVE_FAULTS.load(Ordering::SeqCst) {
                oops::fault_line(
                    super::faultsuite::current_fault_id(),
                    oops::vector_kind(tf.vector),
                    tf,
                    io::in64_cr2(),
                );
                if tf.vector == T_PGFLT {
                    console::write_str("  fault-kind: ");
                    console::write_str(oops::pf_ec_string(tf.err));
                    console::write_str("\n");
                }
                super::faultsuite::note_observed();
            }
            tf.rip = fix_rip;
        }
        None => oops::kernel_fault(tf, why),
    }
}

/// C entry from isr_common.
#[unsafe(no_mangle)]
pub extern "C" fn traps_dispatch(tf: &mut Trapframe) {
    // xv6 usertrap(): a killed process exits on its next trap from user
    // mode; kernel/boot-context traps (CPL0) have no such owner.
    if tf.from_user() {
        let p = crate::proc::proc::myproc();
        if !p.is_null() && unsafe { (*p).killed } != 0 {
            // Ack an in-service IRQ line before abandoning this trap: exit()
            // never returns, so skipping the normal EOI would leave the PIC
            // in-service bit set and starve every lower-priority line (no
            // timer ticks => no preemption => total deadlock).
            if tf.vector >= IRQ_BASE {
                crate::drivers::pic::eoi(tf.vector as u8);
            }
            crate::proc::proc::exit(-1);
        }
    }
    match tf.vector {
        IRQ_TIMER => super::time::tick(tf),
        IRQ_IDE => crate::drivers::block::ide::ideintr(),
        IRQ_COM1 => crate::drivers::char::console::consoleintr(),
        IRQ_SPURIOUS | IRQ_ERROR => {}
        v if v >= IRQ_BASE && v < IRQ_BASE + 16 => {
            // Spurious/unhandled PIC lines: acknowledge conservatively.
            crate::drivers::pic::eoi(v as u8);
        }
        T_PGFLT if !tf.from_user() => {
            // Kernel-mode #PF on a user address (uaccess copy into a
            // fork-COW/lazy page): resolve against the running process's
            // mm before fixup recovery; only unresolvable faults EFAULT.
            let cr2 = crate::arch::x86_64::io::in64_cr2() as usize;
            if crate::mm::fault::handle_kernel_user_fault(cr2, tf.err) {
                return; // resolved: retry the faulting instruction
            }
            fixup_or_oops(tf, "page fault in kernel");
        }
        // User #PF (22 §4): try the M2 resolver first — stack growth, lazy
        // anonymous zero-fill; unresolved faults kill only the process.
        T_PGFLT if tf.from_user() => {
            let cr2 = crate::arch::x86_64::io::in64_cr2() as usize;
            let p = crate::proc::proc::myproc();
            let handled = if p.is_null() {
                false
            } else {
                // SAFETY: the running process owns mm.
                match unsafe { (*p).mm.as_mut() } {
                    Some(mm) => crate::mm::fault::handle_user_fault(mm, cr2, tf.err),
                    None => false,
                }
            };
            if !handled {
                oops::user_fault(tf, T_PGFLT);
                super::signal::fault_cur(tf, super::sigset::SIGSEGV);
            }
        }
        // Exception -> signal mapping (21 §6): #DE -> SIGFPE, #UD ->
        // SIGILL, #BP -> SIGTRAP, every other user-mode exception ->
        // SIGSEGV. force semantics: a blocked disposition cannot defer a
        // synchronous fault forever.
        T_DIVIDE if tf.from_user() => {
            oops::user_fault(tf, T_DIVIDE);
            super::signal::fault_cur(tf, super::sigset::SIGFPE);
        }
        T_ILLOP if tf.from_user() => {
            oops::user_fault(tf, T_ILLOP);
            super::signal::fault_cur(tf, super::sigset::SIGILL);
        }
        T_BRKPT if tf.from_user() => {
            oops::user_fault(tf, T_BRKPT);
            super::signal::fault_cur(tf, super::sigset::SIGTRAP);
        }
        T_GPFLT if !tf.from_user() => fixup_or_oops(tf, "general protection fault in kernel"),
        // Syscalls must arrive through LSTAR (_syscall_entry), never int
        // 0x80: the IDT gate has no swapgs/stack-switch, so a CPL0 entry
        // here is an ABI invariant violation, not a recoverable fault.
        T_SYSCALL => oops::kernel_fault(tf, "syscall via IDT vector (LSTAR only)"),
        v if v < IRQ_BASE && tf.from_user() => {
            oops::user_fault(tf, v);
            super::signal::fault_cur(tf, super::sigset::SIGSEGV);
        }
        v if v < IRQ_BASE => fixup_or_oops(tf, "kernel exception"),
        _ => fixup_or_oops(tf, "unexpected trap"),
    }
    // 2.6 irq_exit: hardware IRQ paths raise bottom halves; run them
    // before returning (IF still masked — our do_softirq keeps IRQs
    // disabled, a UP simplification vs Linux re-enabling; ADR-0020).
    if tf.vector >= IRQ_BASE {
        super::softirq::irq_exit();
    }
    // xv6 usertrap-tail delivery point: any unblocked pending signal
    // runs its handler/default before the frame returns to user mode.
    if tf.from_user() {
        super::signal::userret_deliver(tf);
    }
}

/// C entry from the syscall stub.
#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(tf: &mut Trapframe) {
    // GS-window invariant (R-C): kernel GS base must be the percpu block.
    let gs = io::rdmsr(io::MSR_IA32_GS_BASE);
    if gs != core::ptr::addr_of!(cpu::PERCPU) as u64 {
        oops::kernel_fault(tf, "syscall entry with wrong kernel GS base");
    }
    // The syscall number must survive the restart path (Linux orig_ax):
    // `syscall` clobbers rax, so a rewound re-entry would dispatch on
    // the old return value (-ERESTARTSYS became nr=-512 -> ENOSYS in
    // the wild; QEMU serial -38).
    let orig_nr = tf.rax;
    let ret = super::syscall::dispatch(tf);
    if ret == abi::ERESTARTSYS {
        // Restartable interruption (21 §3): rewind to the `syscall`
        // insn (2 bytes) and restore rax to the number so the call
        // re-executes once the interrupt is handled. s6: the
        // interrupter is an unblocked signal — the delivery below runs
        // the handler first; sigreturn resumes at the rewound insn
        // with the same frame values (rip/rax included). Fatal kills
        // still take the pre-s6 route (re-entered call answers -1,
        // exit at the next user trap).
        tf.rip -= 2;
        tf.rax = orig_nr;
        super::signal::userret_deliver(tf);
        return;
    }
    if ret == super::signal::SIGRETURN_RET {
        // rt_sigreturn restored the whole user context (rax included):
        // the dispatch tail must not touch the frame again.
        super::signal::userret_deliver(tf);
        return;
    }
    tf.rax = ret as u64;
    super::signal::userret_deliver(tf);
}
