//! minios kernel entry: multiboot trampoline + xv6-style init sequence.
#![no_std]
#![no_main]
#![allow(special_module_name)] // Linux-style `lib/` module tree

mod arch;
mod console; // M0: printk sinks (VGA+COM1)
mod drivers;
mod fs;
mod io;
mod ipc;
mod kernel;
mod lib;
mod mm;
mod panic;
mod usercall;

use arch::x86::mm::layout::p2v;

// ---- linker symbols from x86-minios.ld ------------------------------------
unsafe extern "C" {
    static LOW_MB_MAGIC: u8; // low-phys slot holding multiboot eax
    static LOW_MB_INFO: u8; // low-phys slot holding multiboot ebx
    #[allow(dead_code)] // referenced by the asm below, invisible to rustc
    static BOOT_STACK_TOP: u8; // kernel boot stack (high .bss)
}

core::arch::global_asm!(
    "
    .section .multiboot, \"a\"
    .align 4
    .long 0x1BADB002          /* magic */
    .long 0x00000003          /* flags: page-align mods, want mem info */
    .long -(0x1BADB002+3)     /* checksum */

    .section .lowentry, \"ax\"
    .code32
    .globl _low_start
    _low_start:
        cli
        cld
        /* stash multiboot eax(magic)/ebx(info block) before clobbering */
        movl $LOW_MB_MAGIC, %edi
        movl %eax, (%edi)
        movl $LOW_MB_INFO, %edi
        movl %ebx, (%edi)
        /* zero the boot page directory */
        movl $LOW_PDE, %edi
        xorl %eax, %eax
        movl $1024, %ecx
        rep stosl
        /* pde[0] = identity first 4MB; pde[512] = KERNBASE alias, PSE 4MB */
        movl $LOW_PDE, %edi
        movl $0x83, %eax
        movl %eax, 0(%edi)
        movl %eax, 2048(%edi)
        /* load CR3, enable PSE + PG */
        movl $LOW_PDE, %eax
        movl %eax, %cr3
        movl %cr4, %eax
        orl $0x10, %eax
        movl %eax, %cr4
        movl %cr0, %eax
        orl $0x80000000, %eax
        movl %eax, %cr0
        ljmp $0x08, $_high_entry

    .globl _high_entry
    _high_entry:
        /* enable SSE before the first Rust instruction: CR0.EM=0, MP=1,
           CR4.OSFXSR|OSXMMEXCPT (the kernel is built with SSE enabled; the
           trampoline runs before seginit, which repeats this setup). */
        movl %cr0, %eax
        andl $0xfffffffb, %eax
        orl $0x2, %eax
        movl %eax, %cr0
        movl %cr4, %eax
        orl $0x600, %eax
        movl %eax, %cr4
        movl $BOOT_STACK_TOP, %esp
        movl $0, %ebp
        call _high_start
    1:  hlt
        jmp 1b
    ",
    options(att_syntax)
);

/// Entry from the trampoline, running on the low stack with paging on.
#[no_mangle]
unsafe extern "C" fn _high_start() -> ! {
    // SAFETY: the low region is identity-mapped and KERNBASE-aliased by the
    // trampoline page table, so reading the stash via p2v is valid here.
    unsafe {
        let magic = core::ptr::read_volatile(p2v(addr_of!(LOW_MB_MAGIC) as usize) as *const u32);
        let info = core::ptr::read_volatile(p2v(addr_of!(LOW_MB_INFO) as usize) as *const u32);
        BOOT_MAGIC = magic;
        BOOT_INFO = info;
    }
    rust_main()
}

use core::ptr::addr_of;

#[no_mangle]
static mut BOOT_MAGIC: u32 = 0;
#[no_mangle]
static mut BOOT_INFO: u32 = 0;

/// xv6 main(): kernel-wide initialization, first process, scheduler.
/// Order mirrors main.c exactly (timer source deviates: PIT, not lapic).
unsafe fn rust_main() -> ! {
    use crate::arch::x86::kernel::{idt, pic, time, trap};
    use crate::arch::x86::mm::{layout, pgtable as vm};
    use crate::drivers::block::ide;
    use crate::drivers::r#char::console as cons;
    use crate::fs::{buffer, xv6fs};
    use crate::kernel::sched::{self as sched, proc};
    use crate::mm::frame::{kinit1, kinit2};

    crate::console::early_banner(); // COM1 + VGA, version banner
    unsafe {
        kinit1(layout::kernel_end(), layout::p2v(4 << 20));
        vm::kvmalloc(); // builds kpgdir; boots off the trampoline page dir
        idt::seginit(); // GDT + TSS
        pic::picinit();

        cons::consoleinit();
        // console as device 1 (devsw) for init/sh io
        xv6fs::devsw::register_dev(1, Some(dev_cons_read), Some(dev_cons_write));

        // pinit/fileinit: ptable/ftable locks are const-initialized (RawLock)
        trap::tvinit();
        buffer::binit();
        ide::ideinit();
        time::timerinit(); // PIT 100Hz (plan-approved deviation from lapic)

        kinit2(layout::p2v(4 << 20), layout::p2v(layout::PHYSTOP));
        crate::fs::xv6fs::iinit(); // xv6 main.c: iinit() — inode cache
        proc::userinit();
        sched::scheduler(); // never returns
    }
}

// ---- devshim: console as char device 1 (xv6 devsw console ip is unused) --
use crate::drivers::r#char::console as cons;
use crate::fs::xv6fs::inode::Inode;

unsafe fn dev_cons_read(_ip: *mut Inode, dst: *mut u8, n: i32) -> i32 {
    cons::consread(dst, n)
}

unsafe fn dev_cons_write(_ip: *mut Inode, src: *const u8, n: i32) -> i32 {
    cons::conswrite(src, n)
}

/// Halt forever with interrupts disabled (used by panic).
#[no_mangle]
pub fn halt() -> ! {
    unsafe {
        core::arch::asm!("cli; 1: hlt; jmp 1b", options(noreturn, att_syntax));
    }
}
