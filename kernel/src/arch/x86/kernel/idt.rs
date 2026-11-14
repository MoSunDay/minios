//! idt: GDT/TSS/segment init (x86.h lgdt/lidt/ltr + vm.c seginit) and the
//! IDT itself (trap.c tvinit/idtinit).
//!
//! GDT layout (6 entries; xv6 also has a per-cpu SEG_KCPU slot, dropped
//! here because minios is uniprocessor):
//!   0 null   1 KC32 sel 0x08   2 KD32 sel 0x10   3 TSS sel 0x18
//!   4 UC32 sel 0x20|3   5 UD32 sel 0x28|3
//! With flat base/limit everywhere, kernel and user segments differ only
//! in DPL, exactly like xv6's seginit (vm.c:19).

use core::arch::asm;
use core::mem::size_of;
use core::ptr::addr_of_mut;

use crate::arch::x86::kernel::T_SYSCALL;

// Segment indices (mmu.h:44) and selector form (index << 3 | rpl).
pub const SEG_KCODE: usize = 1;
pub const SEG_KDATA: usize = 2;
pub const SEG_UCODE: usize = 3;
pub const SEG_UDATA: usize = 4;
pub const SEG_TSS: usize = 5;
pub const NSEGS: usize = 6;
pub const DPL_USER: u32 = 0x3;

pub const SEG_KCODE_SEL: u16 = (SEG_KCODE << 3) as u16; // 0x08
pub const SEG_KDATA_SEL: u16 = (SEG_KDATA << 3) as u16; // 0x10
pub const SEG_TSS_SEL: u16 = (SEG_TSS << 3) as u16; // 0x28
#[allow(dead_code)] // selector form used by future user-entry code
pub const SEG_UCODE_SEL: u16 = (SEG_UCODE << 3) as u16; // 0x18 (|3 from user)
#[allow(dead_code)] // selector form used by future user-entry code
pub const SEG_UDATA_SEL: u16 = (SEG_UDATA << 3) as u16; // 0x20 (|3 from user)

// Application segment type bits (mmu.h:83).
const STA_X: u32 = 0x8; // executable
const STA_R: u32 = 0x2; // readable (exec) / writable (data)
const STA_W: u32 = 0x2;
// System segment types (mmu.h:74).
const STS_T32A: u32 = 0x9; // available 32-bit TSS
const STS_IG32: u32 = 0xE; // 32-bit interrupt gate
const STS_TG32: u32 = 0xF; // 32-bit trap gate

/// GDTR/IDTR operand (`ushort pd[3]` in x86.h:63).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Pseudodesc {
    pub limit: u16,
    pub base: u32,
}

/// Task state segment (mmu.h:151), byte-for-byte.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Taskstate {
    pub link: u32,
    pub esp0: u32,
    pub ss0: u16,
    pub padding1: u16,
    pub esp1: u32,
    pub ss1: u16,
    pub padding2: u16,
    pub esp2: u32,
    pub ss2: u16,
    pub padding3: u16,
    pub cr3: u32,
    pub eip: u32,
    pub eflags: u32,
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub es: u16,
    pub padding4: u16,
    pub cs: u16,
    pub padding5: u16,
    pub ss: u16,
    pub padding6: u16,
    pub ds: u16,
    pub padding7: u16,
    pub fs: u16,
    pub padding8: u16,
    pub gs: u16,
    pub padding9: u16,
    pub ldt: u16,
    pub padding10: u16,
    pub t: u16,
    pub iomb: u16, // I/O map base; == sizeof(TSS) means "no bitmap"
}

// The GDT/TSS/IDT shared by the (single) CPU. xv6 keeps gdt/ts per-cpu.
static mut GDT: [u64; NSEGS] = [0; NSEGS];
static mut TS: Taskstate = Taskstate {
    link: 0,
    esp0: 0,
    ss0: 0,
    padding1: 0,
    esp1: 0,
    ss1: 0,
    padding2: 0,
    esp2: 0,
    ss2: 0,
    padding3: 0,
    cr3: 0,
    eip: 0,
    eflags: 0,
    eax: 0,
    ecx: 0,
    edx: 0,
    ebx: 0,
    esp: 0,
    ebp: 0,
    esi: 0,
    edi: 0,
    es: 0,
    padding4: 0,
    cs: 0,
    padding5: 0,
    ss: 0,
    padding6: 0,
    ds: 0,
    padding7: 0,
    fs: 0,
    padding8: 0,
    gs: 0,
    padding9: 0,
    ldt: 0,
    padding10: 0,
    t: 0,
    iomb: 0,
};
static mut IDT: [u64; 256] = [0; 256];
static mut GDTR: Pseudodesc = Pseudodesc { limit: 0, base: 0 };
static mut IDTR: Pseudodesc = Pseudodesc { limit: 0, base: 0 };

/// Build a normal segment descriptor — the SEG macro (mmu.h:69).
/// `lim` is byte granularity with G=1, i.e. pass 0xffffffff for 4GB.
const fn seg(typ: u32, base: usize, lim: usize, dpl: u32) -> u64 {
    let b = base as u64;
    let l = lim as u64;
    (l >> 12) & 0xffff // lim_15_0
        | (b & 0xffff) << 16 // base_15_0
        | ((b >> 16) & 0xff) << 32 // base_23_16
        | (typ as u64) << 40
        | 1 << 44 // s = 1
        | ((dpl as u64) << 45)
        | 1 << 47 // p = 1
        | ((l >> 28) & 0xf) << 48 // lim_19_16 (high dword bits 16-19)
        | 1 << 54 // db = 1 (32-bit)
        | 1 << 55 // g = 1 (4K granularity)
        | ((b >> 24) & 0xff) << 56 // base_31_24
}

/// Build a TSS system descriptor — the SEG16 macro (mmu.h:73): byte
/// granularity, g = 0; caller clears the s bit like vm.c:168 does.
const fn seg16(typ: u32, base: usize, lim: usize, dpl: u32) -> u64 {
    let b = base as u64;
    let l = lim as u64;
    (l & 0xffff)
        | (b & 0xffff) << 16
        | ((b >> 16) & 0xff) << 32
        | (typ as u64) << 40
        | 1 << 44
        | ((dpl as u64) << 45)
        | 1 << 47
        | ((l >> 28) & 0xf) << 48 // lim_19_16
        | 1 << 54 // db
        | ((b >> 24) & 0xff) << 56
}

/// Build an interrupt/trap gate — SETGATE (mmu.h:213).
const fn setgate(off: usize, sel: u16, istrap: bool, dpl: u32) -> u64 {
    let typ = if istrap { STS_TG32 } else { STS_IG32 };
    (off as u64 & 0xffff) // off_15_0
        | ((sel as u64) << 16) // cs
        | (typ as u64) << 40
        | ((dpl as u64) << 45)
        | 1 << 47 // p = 1
        | ((off as u64 >> 16) & 0xffff) << 48 // off_31_16
}

/// lgdt (x86.h:63).
fn lgdt(pd: *const Pseudodesc) {
    // SAFETY: `lgdt` loads the descriptor register from our packed static.
    unsafe { asm!("lgdt (%eax)", in("eax") pd as u32, options(att_syntax)) };
}

/// lidt (x86.h:77).
fn lidt(pd: *const Pseudodesc) {
    // SAFETY: `lidt` loads the descriptor register from our packed static.
    unsafe { asm!("lidt (%eax)", in("eax") pd as *const Pseudodesc as u32, options(att_syntax)) };
}

/// ltr (x86.h:89): load the task register with a TSS selector.
fn ltr(sel: u16) {
    // SAFETY: selector must point at a valid (present) TSS descriptor.
    unsafe { asm!("ltr %ax", in("eax") sel as u32, options(att_syntax)) };
}

/// seginit (vm.c:19): install the flat kernel/user segments. The xv6
/// per-cpu SEG_KCPU/gs base is not needed on this uniprocessor port.
pub fn seginit() {
    // FPU/SSE bring-up (xv6 vm.c seginit: clear CR0.EM, set MP, and set
    // CR4.OSFXSR|OSXMMEXCPT). The boot trampoline already does this before
    // the first Rust instruction; repeat it here to keep seginit faithful.
    unsafe {
        let mut cr0: u32;
        let mut cr4: u32;
        core::arch::asm!("mov %cr0, {}", out(reg) cr0, options(att_syntax));
        cr0 &= !(1 << 2); // EM: no x87 emulation trap
        cr0 |= 1 << 1; // MP: monitor coprocessor
        core::arch::asm!("mov {}, %cr0", in(reg) cr0, options(att_syntax));
        core::arch::asm!("mov %cr4, {}", out(reg) cr4, options(att_syntax));
        cr4 |= (1 << 9) | (1 << 10); // OSFXSR | OSXMMEXCPT
        core::arch::asm!("mov {}, %cr4", in(reg) cr4, options(att_syntax));
    }
    // SAFETY: GDT/GDTR are only touched here and in load_tss, before any
    // concurrent access exists (called with interrupts off at boot).
    unsafe {
        GDT[0] = 0; // null
        GDT[SEG_KCODE] = seg(STA_X | STA_R, 0, 0xffff_ffff, 0);
        GDT[SEG_KDATA] = seg(STA_W, 0, 0xffff_ffff, 0);
        GDT[SEG_UCODE] = seg(STA_X | STA_R, 0, 0xffff_ffff, DPL_USER);
        GDT[SEG_UDATA] = seg(STA_W, 0, 0xffff_ffff, DPL_USER);
        GDT[SEG_TSS] = 0; // installed later by load_tss (switchuvm)
        GDTR = Pseudodesc {
            limit: (size_of::<[u64; NSEGS]>() - 1) as u16,
            base: addr_of_mut!(GDT) as u32,
        };
        lgdt(addr_of_mut!(GDTR));
    }
}

/// Program the TSS for the current task and load the task register.
/// Called by pgtable::switchuvm with `kstack_top` = proc->kstack+KSTACKSIZE
/// (vm.c:163..171: esp0 is where the CPU switches on a trap from user mode;
/// the Trapframe the stub builds sits just below it).
pub fn load_tss(kstack_top: usize) {
    // SAFETY: TS/GDT are serialized by switchuvm's push_intr_off.
    unsafe {
        let ts = addr_of_mut!(TS);
        (*ts).ss0 = SEG_KDATA_SEL;
        (*ts).esp0 = kstack_top as u32;
        (*ts).iomb = size_of::<Taskstate>() as u16; // no I/O bitmap
        GDT[SEG_TSS] = seg16(STS_T32A, ts as usize, size_of::<Taskstate>() - 1, 0) & !(1 << 44); // s = 0
        ltr(SEG_TSS_SEL);
    }
}

// vector entry points, defined by the global_asm! stubs in trap.rs.
unsafe extern "C" {
    static __vectors: [u32; 256];
}

/// tvinit + idtinit (trap.c:21, trap.c:37): point all 256 IDT entries at
/// the generated vector stubs; T_SYSCALL is a trap gate with DPL_USER so
/// `int $T_SYSCALL` from user mode reaches us. Loads the IDTR at the end.
pub fn tvinit() {
    // SAFETY: __vectors is the 256-entry table emitted by trap.rs's asm.
    let vecs = core::ptr::addr_of!(__vectors);
    // SAFETY: IDT/IDTR are only touched here, before interrupts are enabled.
    unsafe {
        for i in 0..256 {
            IDT[i] = setgate((*vecs)[i] as usize, SEG_KCODE_SEL, false, 0);
        }
        IDT[T_SYSCALL as usize] = setgate(
            (*vecs)[T_SYSCALL as usize] as usize,
            SEG_KCODE_SEL,
            true,
            DPL_USER,
        );
        IDTR = Pseudodesc {
            limit: (size_of::<[u64; 256]>() - 1) as u16,
            base: addr_of_mut!(IDT) as u32,
        };
        lidt(addr_of_mut!(IDTR));
    }
}
