//! Controlled fault reporting: registers, fault RIP, error-code decode and
//! frame-pointer backtrace through the kernel symbol table (M1R oops path).

use super::ksymtab;
use super::traps::*;
use crate::console;

fn print_sym(addr: u64) {
    let mut buf = [0u8; 96];
    let n = super::ksyms::symbolize_into(ksymtab::table(), addr, &mut buf);
    // SAFETY: bytes come from the generated ASCII symbol table.
    let s = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    console::write_str(s);
}

fn print_hex(v: u64) {
    let hex = b"0123456789abcdef";
    let mut digits = [0u8; 16];
    let mut d = 0;
    let mut x = v;
    if x == 0 {
        digits[d] = b'0';
        d += 1;
    }
    while x > 0 {
        digits[d] = hex[(x & 0xf) as usize];
        d += 1;
        x >>= 4;
    }
    console::write_str("0x");
    while d > 0 {
        d -= 1;
        // Single ASCII byte is valid UTF-8.
        console::write_str(unsafe { core::str::from_utf8_unchecked(&[digits[d]]) });
    }
}

#[allow(dead_code)]
fn print_dec(mut v: u64) {
    let mut digits = [0u8; 20];
    let mut d = 0;
    if v == 0 {
        digits[d] = b'0';
        d += 1;
    }
    while v > 0 {
        digits[d] = b'0' + (v % 10) as u8;
        d += 1;
        v /= 10;
    }
    while d > 0 {
        d -= 1;
        console::write_str(unsafe { core::str::from_utf8_unchecked(&[digits[d]]) });
    }
}

/// Short fault-kind tag for one vector (observer lines and oops paths).
pub fn vector_kind(v: u64) -> &'static str {
    match v {
        super::traps::T_PGFLT => "PF",
        super::traps::T_GPFLT => "GP",
        super::traps::T_DIVIDE => "DE",
        super::traps::T_BRKPT => "BP",
        _ => "EX",
    }
}

/// Decode #PF error-code bits (P W/U RSV I/D PK SGL).
pub fn pf_ec_string(ec: u64) -> &'static str {
    match (ec & 1, ec & 2, ec & 4) {
        (0, _, _) => "not-present",
        (1, 0, 0) => "write-to-read-only",
        (1, 0, 4) => "user-read-of-kernel",
        (1, 2, 0) => "kernel-write-to-readonly",
        (1, 2, 4) => "user-write-to-readonly",
        _ => "permission",
    }
}

/// Print one structured M1RFault line: the exact contract the expected-fault
/// verdict matches (fault_id, kind, error code, rip, registers, backtrace).
pub fn fault_line(fault_id: &str, kind: &str, tf: &Trapframe, cr2: u64) {
    console::write_str("M1RFault fault_id=");
    console::write_str(fault_id);
    console::write_str(" kind=");
    console::write_str(kind);
    console::write_str(" ec=");
    print_hex(tf.err);
    console::write_str(" cr2=");
    print_hex(cr2);
    console::write_str(" rip=");
    print_sym(tf.rip);
    console::write_str(" rax=");
    print_hex(tf.rax);
    console::write_str(" rbx=");
    print_hex(tf.rbx);
    console::write_str(" rdi=");
    print_hex(tf.rdi);
    console::write_str(" rsi=");
    print_hex(tf.rsi);
    console::write_str(" rdx=");
    print_hex(tf.rdx);
    console::write_str(" r10=");
    print_hex(tf.r10);
    console::write_str(" cs=");
    print_hex(tf.cs);
    console::write_str(" backtrace=");
    backtrace(tf.rbp as usize);
    console::write_str("\n");
}

/// Frame-pointer backtrace (kernel frames; force-frame-pointers is frozen in
/// .cargo/config.toml and statically verified by the runner).
pub fn backtrace(mut rbp: usize) {
    const MAX_FRAMES: usize = 12;
    for i in 0..MAX_FRAMES {
        if rbp == 0 || rbp & 7 != 0 || rbp > 0xffff_ffff_ffff_f000 {
            break;
        }
        // SAFETY: frame-pointer chain is validated to be canonical; reads
        // inside kernel half only.
        let ra = unsafe { core::ptr::read_volatile((rbp + 8) as *const u64) };
        let next = unsafe { core::ptr::read_volatile(rbp as *const u64) };
        if i > 0 {
            console::write_str("|");
        }
        print_sym(ra);
        if next <= rbp as u64 {
            break;
        }
        rbp = next as usize;
    }
}

/// Unrecoverable kernel fault: full oops then halt (0x12 -> QEMU 37).
pub fn kernel_fault(tf: &Trapframe, why: &str) -> ! {
    let cr2 = crate::arch::x86_64::io::in64_cr2();
    console::write_str("minios kernel oops: ");
    console::write_str(why);
    console::write_str("\n");
    fault_line("kernel-oops", vector_kind(tf.vector), tf, cr2);
    if tf.vector == T_PGFLT {
        console::write_str("  fault-kind: ");
        console::write_str(pf_ec_string(tf.err));
        console::write_str("\n");
    }
    console::write_str("MINIOS_X86_64_TEST_FATAL_V1 suite=");
    console::write_str(ksymtab::active_suite());
    console::write_str(" run=core-up reason=kernel-oops\n");
    crate::arch::x86_64::io::out32(0xf4, 0x12);
    loop {
        crate::arch::x86_64::io::hlt();
    }
}

/// User-mode fault: shorter oops line, then the caller kills the process.
pub fn user_fault(tf: &Trapframe, vector: u64) {
    let cr2 = crate::arch::x86_64::io::in64_cr2();
    fault_line("user-fault", vector_kind(vector), tf, cr2);
    if vector == T_PGFLT {
        console::write_str("  fault-kind: ");
        console::write_str(pf_ec_string(tf.err));
        console::write_str("\n");
    }
}
