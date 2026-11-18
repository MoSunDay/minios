//! M1R suite (S2 kernel-mode cases + S3 user chain): the CPL0 phase
//! exercises the trap runtime (timer round trip, UserPtr fixup, USER_TOP
//! preflight, runtime symbol table), then seeds pid 1 from the embedded
//! m1r-suite image. The user driver prints its own CASE lines and exits
//! with (total<<4)|failed; when pid 1 exits, finish() merges both phases
//! into the frozen verdict marker and exits 0x10 -> QEMU 33 (0x11 on any
//! failure).

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

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Kernel-phase totals for the merged verdict.
static KERNEL_TOTAL: AtomicU8 = AtomicU8::new(0);
static KERNEL_PASSED: AtomicU8 = AtomicU8::new(0);
/// Set once the user driver phase is armed (pid 1 seeded).
static USER_PHASE: AtomicBool = AtomicBool::new(false);

/// True while pid 1 (the suite driver) runs the user phase.
pub fn user_phase_armed() -> bool {
    USER_PHASE.load(Ordering::SeqCst)
}

/// Merge verdicts and exit QEMU. Called from proc::exit when the armed
/// suite driver exits; `status` is its encoded (total<<4)|failed exit.
pub fn finish_driver(status: i32) -> ! {
    let user_total = ((status >> 4) & 0xf) as u8;
    let user_failed = (status & 0xf) as u8;
    let total = KERNEL_TOTAL
        .load(Ordering::SeqCst)
        .saturating_add(user_total);
    let passed = KERNEL_PASSED
        .load(Ordering::SeqCst)
        .saturating_add(user_total.saturating_sub(user_failed));
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

/// Suite entry (never returns): kernel cases, then the user chain.
pub fn run() -> ! {
    let mut total = 0u8;
    let mut passed = 0u8;
    let checks: [(&str, fn() -> bool); 6] = [
        ("m1r.irq.pit", case_irq_pit),
        ("m1r.uaccess.copyin_efault", case_copyin_efault),
        ("m1r.uaccess.copyout_efault", case_copyout_efault),
        ("m1r.uaccess.kernelptr_efault", case_kernelptr_efault),
        ("m1r.ksym.self", case_ksym_self),
        ("m1r.fs.blockread", case_fs_blockread),
    ];
    for (case_id, check) in checks {
        total += 1;
        if run_case(case_id, check()) {
            passed += 1;
        }
    }
    KERNEL_TOTAL.store(total, Ordering::SeqCst);
    KERNEL_PASSED.store(passed, Ordering::SeqCst);

    // S3 user chain: pid 1 from the embedded m1r-suite image; the driver
    // prints its CASE lines through write(2) and exits with its counts.
    let image = match super::userimg::lookup(b"/m1r-suite") {
        Some(img) => img,
        None => crate::m1r_fatal("m1r-suite image missing"),
    };
    if !crate::proc::proc::userinit_named(image, b"m1r-suite", &[b"m1r-suite"]) {
        crate::m1r_fatal("userinit failed");
    }
    USER_PHASE.store(true, Ordering::SeqCst);
    crate::proc::sched::sched_entry();
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

/// S4 block layer: read the mkfs root image through bread/bcache+IDE
/// and verify the on-disk superblock and root dinode byte-for-byte
/// (tools/fs.h layout; values frozen by minios-tools geometry). Read-only
/// by design: the suite round's root.raw must stay byte-identical to the
/// template (40 §2.2 per-round image accounting).
fn case_fs_blockread() -> bool {
    use crate::fs::buffer::{bread, brelse};
    use crate::fs::BSIZE;
    const IPB: usize = BSIZE / 64; // dinode = 64 bytes (tools/fs.h)
    const T_DIR: u16 = 1;
    let ok = unsafe {
        // Superblock: block 1, six u32 fields little-endian.
        let sb = bread(0, 1);
        let u32at = |o: usize| -> u32 {
            let d: &[u8; BSIZE] = &(*sb).data;
            u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
        };
        let mut good = (
            u32at(0),
            u32at(4),
            u32at(8),
            u32at(12),
            u32at(16),
            u32at(20),
        ) == (1024, 965, 200, 30, 2, 28);
        brelse(sb);
        // Root dinode: inum 1 lives in inode block 2, slot 1.
        let ib = bread(0, 2);
        let base = 1 % IPB; // inum 1 slot within inode block 2
        let d: &[u8; BSIZE] = &(*ib).data;
        let b = base * 64;
        let type_ = u16::from_le_bytes(d[b..b + 2].try_into().unwrap());
        let size = u32::from_le_bytes(d[b + 8..b + 12].try_into().unwrap());
        let addr0 = u32::from_le_bytes(d[b + 12..b + 16].try_into().unwrap());
        // Root directory: 4 entries (., .., m1r-suite, m1r-exec-child),
        // rounded to one block; first data block = 29.
        good = good && type_ == T_DIR && size == 512 && addr0 == 29;
        brelse(ib);
        // First dirent pair sanity straight off the data block.
        let db = bread(0, 29);
        let d: &[u8; BSIZE] = &(*db).data;
        let dot_inum = u16::from_le_bytes(d[0..2].try_into().unwrap());
        let name_ok = d[2] == b'.';
        good = good && dot_inum == 1 && name_ok;
        brelse(db);
        good
    };
    ok
}

fn write_decimal(value: u8) {
    const DIGITS: &[u8; 10] = b"0123456789";
    if value >= 10 {
        console::write_byte(DIGITS[usize::from(value / 10)]);
    }
    console::write_byte(DIGITS[usize::from(value % 10)]);
}
