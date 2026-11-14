//! arch/x86/kernel: CPU-level infrastructure (GDT/IDT/traps/pic/pit).

// Full mmu.h/trap.h constant set: handlers only use a subset today.
#![allow(dead_code)]

pub mod idt;
pub mod pic;
pub mod time;
pub mod trap;

/// Trapframe: exactly what the trap entry asm pushes (xv6 trap.h).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Trapframe {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub oesp: u32, // esp before switch to kernel stack
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    pub trapno: u32,
    pub err: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub esp: u32,
    pub ss: u32,
}

/// Saved context for swtch (xv6 context in x86.h): callee-saved + eip.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    pub edi: u32,
    pub esi: u32,
    pub ebx: u32,
    pub ebp: u32,
    pub eip: u32,
}

// Trap numbers (xv6 traps.h)
pub const T_DIVIDE: u32 = 0;
pub const T_DEBUG: u32 = 1;
pub const T_NMI: u32 = 2;
pub const T_BRKPT: u32 = 3;
pub const T_OFLOW: u32 = 4;
pub const T_BOUND: u32 = 5;
pub const T_ILLOP: u32 = 6;
pub const T_DEVICE: u32 = 7;
pub const T_DBLFLT: u32 = 8;
pub const T_TSS: u32 = 10;
pub const T_SEGNP: u32 = 11;
pub const T_STACK: u32 = 12;
pub const T_GPFLT: u32 = 13;
pub const T_PGFLT: u32 = 14;
pub const T_FPERR: u32 = 16;
pub const T_ALIGN: u32 = 17;
pub const T_MCHK: u32 = 19;
pub const T_SIMDERR: u32 = 20;
pub const T_SYSCALL: u32 = 64; // system call ONLY from user code
pub const T_DEFAULT: u32 = 500; // catchall

pub const T_IRQ0: u32 = 32; // IRQ 0 occurs at this vector

// IRQ numbers
pub const IRQ_TIMER: u32 = 0;
pub const IRQ_KBD: u32 = 1;
pub const IRQ_COM1: u32 = 4;
pub const IRQ_IDE: u32 = 14;
pub const IRQ_ERROR: u32 = 19;
pub const IRQ_SPURIOUS: u32 = 31;

/// User DPL (mmu.h:81 DPL_USER); used by trap dispatch to test tf->cs&3.
pub const DPL_USER: u32 = 0x3;
