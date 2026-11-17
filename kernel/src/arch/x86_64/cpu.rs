//! CPU state for the M1R kernel: GDT with user segments, TSS (RSP0 only),
//! MSR plumbing for fast syscalls, and the GS-based per-CPU block.
//!
//! Segment layout (selector / purpose):
//!   0x00 null            0x08 kernel code 64    0x10 kernel data
//!   0x18 reserved        0x20 user code 32      0x28 user data
//!   0x30 user code 64    0x38..0x47 TSS descriptor
//! STAR expects: kernel CS 0x08; user base 0x20 so sysretq picks
//! CS=0x30 / SS=0x28, and iretq-to-user uses CS=0x33 / SS=0x2b.

use core::arch::asm;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::io::{
    rdmsr, wrmsr, MSR_EFER, MSR_FMASK, MSR_IA32_GS_BASE, MSR_IA32_KERNEL_GS_BASE, MSR_LSTAR,
    MSR_STAR,
};
use crate::kernel::traps::syscall_entry;

pub const SEL_KCODE: u16 = 0x08;
#[allow(dead_code)] // frozen GDT layout (boot.S); STAR/ltr use the raw values
pub const SEL_KDATA: u16 = 0x10;
#[allow(dead_code)]
pub const SEL_UCODE32: u16 = 0x20;
#[allow(dead_code)]
pub const SEL_UDATA: u16 = 0x28;
#[allow(dead_code)]
pub const SEL_UCODE64: u16 = 0x30;
pub const SEL_UCODE64_RPL: u64 = 0x33;
pub const SEL_UDATA_RPL: u64 = 0x2b;
#[allow(dead_code)]
pub const SEL_TSS: u16 = 0x38;

/// Kernel stack size per process (4 pages) — mirrors KSTACKSIZE.
pub const KSTACKSIZE: usize = 4096 * 4;
pub const NPROC: usize = 64;

#[repr(C, align(16))]
struct Tss {
    reserved0: u32,
    privilege_stacks: [u64; 3],
    interrupt_stacks: [u64; 7],
    reserved1: u64,
    iopb_offset: u16,
}

#[repr(C, align(8))]
struct Gdt {
    null: u64,
    kernel_code: u64,
    kernel_data: u64,
    reserved: u64,
    user_code32: u64,
    user_data: u64,
    user_code64: u64,
    tss_low: u64,
    tss_high: u64,
}

#[repr(C, packed)]
struct PseudoDescriptor {
    limit: u16,
    base: u64,
}

/// GS-relative per-CPU block. Offsets are frozen for the entry asm
/// (PERCPU_KERNEL_RSP=0x00, PERCPU_USER_RSP=0x08, PERCPU_CPU=0x10).
#[repr(C)]
pub struct Percpu {
    pub kernel_rsp: AtomicU64,
    pub user_rsp: AtomicU64,
    /// Self pointer: `%gs:0x10` hands the kernel a `&'static Percpu`.
    pub cpu: *const Percpu,
}

pub static mut PERCPU: Percpu = Percpu {
    kernel_rsp: AtomicU64::new(0),
    user_rsp: AtomicU64::new(0),
    cpu: core::ptr::null(),
};

static mut TSS: Tss = Tss {
    reserved0: 0,
    privilege_stacks: [0; 3],
    interrupt_stacks: [0; 7],
    reserved1: 0,
    iopb_offset: core::mem::size_of::<Tss>() as u16,
};

static mut GDT: Gdt = Gdt {
    null: 0,
    kernel_code: 0x0020_9a00_0000_ffff,
    kernel_data: 0x0000_9200_0000_ffff,
    reserved: 0,
    user_code32: 0x00cf_fa00_0000_ffff,
    user_data: 0x00cf_f200_0000_ffff,
    user_code64: 0x0020_fa00_0000_ffff,
    tss_low: 0,
    tss_high: 0,
};

static mut GDTR: PseudoDescriptor = PseudoDescriptor { limit: 0, base: 0 };

static IDT_READY: AtomicBool = AtomicBool::new(false);

/// One-shot CPU bring-up: GDT/TSS, per-CPU GS base, syscall MSRs.
pub fn init(kernel_stack_top: usize) {
    // SAFETY: these tables are initialized exactly once during uniprocessor
    // boot, before any interrupt or syscall can occur.
    unsafe {
        TSS.privilege_stacks[0] = kernel_stack_top as u64;
        let tss_base = addr_of_mut!(TSS) as u64;
        GDT.tss_low = 0x0000_8900_0000_0067
            | ((tss_base & 0x00ff_ffff) << 16)
            | ((tss_base & 0xff00_0000) << 32);
        GDT.tss_high = tss_base >> 32;
        GDTR.limit = (core::mem::size_of::<Gdt>() - 1) as u16;
        GDTR.base = addr_of_mut!(GDT) as u64;
        let gdtr = addr_of_mut!(GDTR);
        asm!("lgdt ({0})", in(reg) gdtr as u64, options(att_syntax, nostack));
        asm!(
            "mov $0x10, %ax\n\tmov %ax, %ds\n\tmov %ax, %es\n\tmov %ax, %ss",
            out("ax") _,
            options(att_syntax, nostack)
        );
        asm!("mov $0x38, %ax\n\tltr %ax", out("ax") _, options(att_syntax, nostack));

        (*core::ptr::addr_of_mut!(PERCPU)).cpu = addr_of_mut!(PERCPU);
        (*core::ptr::addr_of_mut!(PERCPU))
            .kernel_rsp
            .store(kernel_stack_top as u64, Ordering::SeqCst);
        // While in the kernel GS.base points at PERCPU; the swapped-out
        // value (IA32_KERNEL_GS_BASE) is the user's, zero for M1R.
        wrmsr(MSR_IA32_GS_BASE, addr_of_mut!(PERCPU) as u64);
        wrmsr(MSR_IA32_KERNEL_GS_BASE, 0);

        // EFER.SCE was set by boot.S; LSTAR/STAR/FMASK are ours.
        let efer = rdmsr(MSR_EFER);
        wrmsr(MSR_EFER, efer | 1);
        // Kernel CS=0x08; sysretq user base=0x20 (CS 0x30, SS 0x28).
        wrmsr(MSR_STAR, (0x20u64 << 48) | (0x08u64 << 32));
        wrmsr(MSR_LSTAR, syscall_entry as *const () as usize as u64);
        // Clear trace/alignment flags; keep IF so the syscall body observes
        // the same interrupt discipline as the IDT trap-gate path.
        wrmsr(MSR_FMASK, 0x3f7fd5);
    }
}

/// Points TSS.RSP0 (and the syscall-entry kernel stack) at `top`.
/// Called on every process switch; interrupts are off.
pub fn set_kernel_stack(top: usize) {
    // SAFETY: single CPU, caller holds the switch context.
    unsafe {
        TSS.privilege_stacks[0] = top as u64;
        (*core::ptr::addr_of_mut!(PERCPU))
            .kernel_rsp
            .store(top as u64, Ordering::SeqCst);
    }
}

pub fn idt_mark_ready() {
    IDT_READY.store(true, Ordering::SeqCst);
}

#[allow(dead_code)] // asserted by S2 trap-entry diagnostics
pub fn idt_is_ready() -> bool {
    IDT_READY.load(Ordering::SeqCst)
}

/// `%gs:0x10` — current per-CPU block (kernel context only).
#[allow(dead_code)] // S2 current-proc fast path
pub fn my_percpu() -> &'static Percpu {
    // SAFETY: kernel GS base always points at PERCPU (boot sets it before
    // the first interrupt; swapgs maintains the invariant).
    unsafe {
        let p: *const Percpu;
        asm!("movq %gs:0x10, %rax", out("rax") p, options(att_syntax, nomem, nostack, preserves_flags));
        &*p
    }
}

const _: () = {
    assert!(core::mem::offset_of!(Percpu, kernel_rsp) == 0x00);
    assert!(core::mem::offset_of!(Percpu, user_rsp) == 0x08);
    assert!(core::mem::offset_of!(Percpu, cpu) == 0x10);
};
