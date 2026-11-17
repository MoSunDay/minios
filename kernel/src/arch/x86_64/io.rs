pub fn out32(port: u16, value: u32) {
    // SAFETY: port I/O is the x86 hardware boundary used here for debug-exit.
    unsafe {
        core::arch::asm!("outl %eax, %dx", in("dx") port, in("eax") value, options(att_syntax, nomem, nostack))
    }
}

// First used by PIC/PIT programming in M3; keep the full port-width boundary here.
#[expect(dead_code)]
pub fn out16(port: u16, value: u16) {
    // SAFETY: port I/O is the x86 hardware boundary used here for serial and debug-exit.
    unsafe {
        core::arch::asm!("outw %ax, %dx", in("dx") port, in("ax") value, options(att_syntax, nomem, nostack))
    }
}

pub fn out8(port: u16, value: u8) {
    // SAFETY: port I/O is the x86 hardware boundary used here for serial output.
    unsafe {
        core::arch::asm!("outb %al, %dx", in("dx") port, in("al") value, options(att_syntax, nomem, nostack))
    }
}

pub fn in8(port: u16) -> u8 {
    let value: u8;
    // SAFETY: port I/O is the x86 hardware boundary; this read has no memory alias.
    unsafe {
        core::arch::asm!("inb %dx, %al", in("dx") port, out("al") value, options(att_syntax, nomem, nostack))
    }
    value
}

#[allow(dead_code)] // S4 IDE
pub fn in32(port: u16) -> u32 {
    let value: u32;
    // SAFETY: port I/O is the x86 hardware boundary; this read has no memory alias.
    unsafe {
        core::arch::asm!("inl %dx, %eax", in("dx") port, out("eax") value, options(att_syntax, nomem, nostack))
    }
    value
}

/// Small delay via port 0x80: required between some PIC/PIT operations.
pub fn io_wait() {
    // SAFETY: port 0x80 is the traditional POST diagnostic byte; writing it
    // takes ~1us on legacy chipsets and has no other effect.
    unsafe {
        core::arch::asm!("outb %al, %dx", in("dx") 0x80 as u16, in("al") 0u8, options(att_syntax, nomem, nostack))
    }
}

pub fn hlt() {
    // SAFETY: halt until an interrupt arrives.
    unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
}

#[allow(dead_code)] // S2 hard-irq sections
pub fn cli() {
    // SAFETY: disable maskable interrupts.
    unsafe { core::arch::asm!("cli", options(nomem, nostack, preserves_flags)) };
}

pub fn sti() {
    // SAFETY: enable maskable interrupts.
    unsafe { core::arch::asm!("sti", options(nomem, nostack, preserves_flags)) };
}

/// Are maskable interrupts enabled right now (EFLAGS.IF)?
pub fn intr_state() -> bool {
    let flags: u64;
    // SAFETY: raw pushfq/popq read.
    unsafe {
        core::arch::asm!("pushfq; popq %rax", out("rax") flags, options(att_syntax, nomem, nostack))
    };
    flags & 0x200 != 0
}

pub fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: model-specific register read.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(att_syntax, nomem, nostack)
        )
    }
    (high as u64) << 32 | low as u64
}

pub fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    // SAFETY: model-specific register write.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(att_syntax, nomem, nostack)
        )
    }
}

/// Read the value of the GS base that is *currently* in effect.
#[allow(dead_code)] // diagnostics
pub fn rdgsbase() -> u64 {
    // GS base is reachable through MSR_IA32_GS_BASE only while the kernel GS
    // is swapped in; callers must be in kernel context.
    rdmsr(MSR_IA32_GS_BASE)
}

pub const MSR_IA32_GS_BASE: u32 = 0xC000_0101;
pub const MSR_IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
pub const MSR_EFER: u32 = 0xC000_0080;
pub const MSR_STAR: u32 = 0xC000_0081;
pub const MSR_LSTAR: u32 = 0xC000_0082;
pub const MSR_FMASK: u32 = 0xC000_0084;

pub fn in64_cr2() -> u64 {
    let value: u64;
    // SAFETY: control-register read.
    unsafe {
        core::arch::asm!("movq %cr2, %rax", out("rax") value, options(att_syntax, nomem, nostack))
    };
    value
}

pub fn in64_cr3() -> u64 {
    let value: u64;
    // SAFETY: control-register read.
    unsafe {
        core::arch::asm!("movq %cr3, %rax", out("rax") value, options(att_syntax, nomem, nostack))
    };
    value
}

pub fn set_cr3(value: u64) {
    // SAFETY: control-register write (address-space switch).
    unsafe { core::arch::asm!("movq %rax, %cr3", in("rax") value, options(att_syntax, nomem)) };
}

#[allow(dead_code)] // S2 partial unmap paths
pub fn invlpg(addr: u64) {
    // SAFETY: TLB invalidate for one page.
    unsafe {
        core::arch::asm!("invlpg (%rax)", in("rax") addr, options(att_syntax, preserves_flags, nomem, nostack))
    };
}
