//! Trap frames, IDT wiring, and the shared trap/syscall dispatchers.

use core::arch::global_asm;

use super::fixup;
use super::oops;
pub use super::tf_layout::Trapframe;
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::io;

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

/// C entry from isr_common.
#[unsafe(no_mangle)]
pub extern "C" fn traps_dispatch(tf: &mut Trapframe) {
    match tf.vector {
        IRQ_TIMER => super::time::tick(tf),
        IRQ_SPURIOUS | IRQ_ERROR => {}
        v if v >= IRQ_BASE && v < IRQ_BASE + 16 => {
            // Spurious/unhandled PIC lines: acknowledge conservatively.
            crate::drivers::pic::eoi(v as u8);
        }
        T_PGFLT if !tf.from_user() => {
            if fixup::apply(tf) {
                return;
            }
            oops::kernel_fault(tf, "page fault in kernel");
        }
        T_GPFLT if !tf.from_user() => {
            if fixup::apply(tf) {
                return;
            }
            oops::kernel_fault(tf, "general protection fault in kernel");
        }
        v if v < IRQ_BASE && tf.from_user() => {
            oops::user_fault(tf, v);
            crate::proc::proc::kill_cur_on_fault();
        }
        v if v < IRQ_BASE => {
            if fixup::apply(tf) {
                return;
            }
            oops::kernel_fault(tf, "kernel exception");
        }
        _ => {
            if fixup::apply(tf) {
                return;
            }
            oops::kernel_fault(tf, "unexpected trap");
        }
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
    let ret = super::syscall::dispatch(tf);
    tf.rax = ret as u64;
}
