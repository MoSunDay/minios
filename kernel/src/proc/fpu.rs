//! Eager FPU/SSE state (M1R): fxsave64 on switch-out, fxrstor64 on
//! switch-in; CR0/CR4 configured once at boot (plans/linux26/21 §1).

use core::arch::asm;

use super::FpuArea;

pub fn init_fpu() {
    // SAFETY: one-shot CR0/CR4 configuration; kernel codegen has SSE off.
    unsafe {
        let mut cr0: u64;
        asm!("movq %cr0, %rax", out("rax") cr0, options(att_syntax, nomem, nostack));
        cr0 &= !(1 << 2); // clear EM (no emulation)
        cr0 &= !(1 << 3); // clear TS (eager, no lazy traps)
        asm!("movq %rax, %cr0", in("rax") cr0, options(att_syntax, nomem));
        let mut cr4: u64;
        asm!("movq %cr4, %rax", out("rax") cr4, options(att_syntax, nomem, nostack));
        cr4 |= 1 << 9;  // OSFXSR
        cr4 |= 1 << 10; // OSXMMEXCPT
        asm!("movq %rax, %cr4", in("rax") cr4, options(att_syntax, nomem));
    }
}

/// fxsave64 into `area` (must be 16-byte aligned, 512 bytes).
pub fn save(area: &FpuArea) {
    // SAFETY: area satisfies fxsave64 alignment/size requirements.
    unsafe {
        asm!(
            "fxsaveq (%rax)",
            in("rax") area as *const FpuArea as *const u8,
            options(att_syntax, nomem)
        );
    }
}

/// fxrstor64 from `area`.
pub fn restore(area: &FpuArea) {
    // SAFETY: see save.
    unsafe {
        asm!(
            "fxrstorq (%rax)",
            in("rax") area as *const FpuArea as *const u8,
            options(att_syntax, nomem)
        );
    }
}

/// Initialize an area to the arch default state for a fresh process.
pub fn init_area(area: &mut FpuArea) {
    for b in area.0.iter_mut() {
        *b = 0;
    }
    // x87 control word 0x037f (LE): MM/X87 exceptions masked, 64-bit
    // precision; fxsave restores it as the fresh process state.
    area.0[0] = 0x7f;
    area.0[1] = 0x03;
    // MXCSR: default 0x1f80 at offset 24.
    area.0[24] = 0x80;
    area.0[25] = 0x1f;
}
