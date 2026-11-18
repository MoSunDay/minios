//! M1R kernel-mode suite v1 (S2, 20-roadmap M1R): runs entirely in CPL0
//! boot context — no user ELF — and exercises the real trap runtime:
//! the CPL0 interrupt path (first timer tick through frame normalization
//! and iretq), UserPtr fixup on unmapped user memory, and the runtime
//! symbol table, and the USER_TOP preflight that rejects kernel-half
//! pointers before any byte moves. Output reuses the frozen M0R CASE
//! protocol; the final
//! PASS marker exits 0x10 -> QEMU 33.

use super::ksyms;
use super::ksymtab;
use super::traps;
use crate::arch::x86_64::io;
use crate::console;
use crate::mm::uaccess;

/// A canonical user VA far above the low boot mappings and far below the
/// stack region: unmapped in the boot page tables, so a kernel access
/// takes a not-present #PF (22 §6).
pub const USER_UNMAPPED_VA: usize = 0x0000_5555_0000_0000;

unsafe extern "C" {
    fn _copyin_bytes(dst: *mut u8, src: *const u8, len: usize) -> i64;
}

/// Suite entry (never returns; exits through isa-debug-exit).
pub fn run() -> ! {
    let mut total = 0u8;
    let mut passed = 0u8;
    let checks: [(&str, fn() -> bool); 5] = [
        ("m1r.irq.pit", case_irq_pit),
        ("m1r.uaccess.copyin_efault", case_copyin_efault),
        ("m1r.uaccess.copyout_efault", case_copyout_efault),
        ("m1r.uaccess.kernelptr_efault", case_kernelptr_efault),
        ("m1r.ksym.self", case_ksym_self),
    ];
    for (case_id, check) in checks {
        total += 1;
        if run_case(case_id, check()) {
            passed += 1;
        }
    }

    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m1r run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m1r run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x10 } else { 0x11 });
    loop {
        io::hlt();
    }
}

fn run_case(case_id: &str, passed: bool) -> bool {
    console::write_str("CASE ");
    console::write_str(case_id);
    console::write_str(if passed { " PASS\n" } else { " FAIL\n" });
    passed
}

/// First real CPL0 interrupt round trip: unmask is done at boot; enable
/// interrupts and halt until the PIT tick handler ran (trapasm frame
/// normalization -> traps_dispatch -> time::tick -> iretq) and returned.
fn case_irq_pit() -> bool {
    if super::time::ticks() != 0 {
        return false; // ticks before sti(): a previous IRQ would mask a dead path
    }
    // Boot context on the boot stack; the IRQ frame nests below rsp.
    io::sti();
    // hlt with IF=1 returns only via an interrupt; each wake re-checks.
    // If the timer never fires, hlt never returns and the runner deadline
    // produces the TIMEOUT evidence (never a false PASS).
    while super::time::ticks() == 0 {
        io::hlt();
    }
    io::cli();
    super::time::ticks() > 0
}

/// copyin from an unmapped user VA must return exactly -EFAULT (fixup
/// recovery, no oops); a partial copy is never reported as success.
fn case_copyin_efault() -> bool {
    let mut kbuf = [0u8; 16];
    uaccess::copyin(kbuf.as_mut_ptr(), USER_UNMAPPED_VA as *const u8, 8) == uaccess::EFAULT
}

/// copyout to an unmapped user VA: same contract, store direction.
fn case_copyout_efault() -> bool {
    let kbuf = [0u8; 16];
    uaccess::copyout((USER_UNMAPPED_VA + 0x1000) as *mut u8, kbuf.as_ptr(), 8) == uaccess::EFAULT
}

/// Kernel-half pointers must be rejected by the USER_TOP preflight
/// before any byte moves: neither a copyin from kernel memory nor a
/// copyout into it may touch the target (22 §6).
fn case_kernelptr_efault() -> bool {
    let mut kbuf = [0u8; 8];
    // SAFETY: taking the address of a kernel static.
    let kva = core::ptr::addr_of!(traps::m1r_force_iret) as usize as *const u8;
    uaccess::copyin(kbuf.as_mut_ptr(), kva, 8) == uaccess::EFAULT
        && uaccess::copyout(kva as *mut u8, kbuf.as_ptr(), 8) == uaccess::EFAULT
}

/// Runtime self-proof of the generated symbol table: an exact hit on a
/// no-mangle object symbol, and prefix symbolization one byte inside an
/// asm primitive.
fn case_ksym_self() -> bool {
    let table = ksymtab::table();
    if table.is_empty() {
        return false;
    }
    // SAFETY: reading the address of a kernel static.
    let hook = core::ptr::addr_of!(traps::m1r_force_iret) as u64;
    let exact =
        matches!(ksyms::lookup(table, hook), Some((0, s)) if s.name_str() == "m1r_force_iret");
    let mut buf = [0u8; 96];
    let n = ksyms::symbolize_into(table, _copyin_bytes as *const () as u64 + 2, &mut buf);
    // SAFETY: bytes come from the generated ASCII symbol table.
    let sym = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    exact && sym.starts_with("_copyin_bytes+0x")
}

fn write_decimal(value: u8) {
    const DIGITS: &[u8; 10] = b"0123456789";
    if value >= 10 {
        console::write_byte(DIGITS[usize::from(value / 10)]);
    }
    console::write_byte(DIGITS[usize::from(value % 10)]);
}
