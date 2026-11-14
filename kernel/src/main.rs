//! minios kernel entry (M0 skeleton).
//!
//! Boot flow: multiboot1 loader -> `_start` (asm stub) -> `rust_main`.

#![no_std]
#![no_main]

#[macro_use]
mod console;

mod io;
mod multiboot;
mod panic;

use core::arch::global_asm;

// Boot stub: disable interrupts, clear `.bss`, install the boot stack and
// call `rust_main` with the multiboot magic (EAX) and info pointer (EBX).
global_asm!(
    ".section .text.boot, \"ax\"",
    ".global _start",
    ".type _start, @function",
    "_start:",
    "    cli",
    "    cld",
    "    movl    %eax, %esi",
    "    movl    $__bss_start, %edi",
    "    movl    $__bss_end, %ecx",
    "    subl    %edi, %ecx",
    "    shrl    $2, %ecx",
    "    xorl    %eax, %eax",
    "    rep stosl",
    "    movl    $boot_stack_top, %esp",
    "    xorl    %ebp, %ebp",
    "    pushl   %ebx",
    "    pushl   %esi",
    "    call    rust_main",
    "1:  hlt",
    "    jmp     1b",
    ".size _start, . - _start",
    options(att_syntax)
);

/// C-ABI kernel entry called from the boot stub; never returns.
#[no_mangle]
pub extern "C" fn rust_main(magic: u32, mbi_addr: u32) -> ! {
    if magic != multiboot::BOOT_LOADER_MAGIC {
        panic!("bad multiboot magic {:#010x}", magic);
    }

    console::early_banner();

    println!("multiboot: info block at {:#010x}", mbi_addr);
    match multiboot::mem_info(mbi_addr) {
        Some((lower, upper)) => {
            println!("memory: lower {} KiB, upper {} KiB", lower, upper);
        }
        None => println!("memory: loader did not pass memory info"),
    }
    println!("boot ok");

    halt()
}

/// Idle forever with interrupts disabled.
pub fn halt() -> ! {
    loop {
        io::hlt();
    }
}
