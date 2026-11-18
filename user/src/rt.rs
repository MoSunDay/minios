//! Process startup (plans/linux26/04 §3): `_start` reads the Linux x86-64
//! initial stack (argc, argv[], NULL, envp[], NULL, auxv) and lands in
//! the binary's `main(argc, argv)`; its return value becomes exit status.

use crate::sys;

core::arch::global_asm!(
    ".section .text.start",
    ".globl _start",
    "_start:",
    "    xor rbp, rbp",
    "    mov rdi, rsp",
    "    and rsp, -16",
    "    call _start_rust",
    "    ud2",
);

/// Rust side of `_start`: `rsp0` points at argc.
#[unsafe(no_mangle)]
unsafe extern "C" fn _start_rust(rsp0: *const u64) -> ! {
    // Zero .bss before any Rust code touches statics.
    // SAFETY: linker-provided bounds cover only our own bss.
    unsafe {
        let start = core::ptr::addr_of!(__bss_start) as usize;
        let end = core::ptr::addr_of!(__bss_end) as usize;
        core::ptr::write_bytes(start as *mut u8, 0, end - start);
    }
    // SAFETY: caller (asm entry) passed the initial user rsp.
    let argc = unsafe { *rsp0 } as usize;
    // SAFETY: argv[] starts one word above argc.
    let argv = unsafe { rsp0.add(1) } as *const *const u8;
    let code = main(argc as i32, argv);
    sys::exit(code)
}

unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

// Each binary provides this; returning from main exits with the code.
extern "C" {
    fn main(argc: i32, argv: *const *const u8) -> i32;
}
