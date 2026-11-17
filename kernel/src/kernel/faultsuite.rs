//! M1R expected-fault suite (S2, 40 §2.1.6): controlled #PF/#GP/#DE/#BP
//! injections in kernel context. Each site lives in m1rfault.S with a
//! .m1r.fixup pair; the traps.rs observer prints one structured M1RFault
//! line per hit (OBSERVE_FAULTS), the fixup recovers execution, and the
//! driver verifies the recovery contract. The suite has its own serial
//! namespace and verdict; its PASS never counts toward ordinary suites.

use core::arch::global_asm;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::suite::USER_UNMAPPED_VA;
use super::traps;
use crate::arch::x86_64::io;
use crate::console;

global_asm!(
    include_str!("../arch/x86_64/m1rfault.S"),
    options(att_syntax)
);

unsafe extern "C" {
    fn _m1rf_inject_pf_unmapped(addr: u64) -> i64;
    fn _m1rf_inject_pf_rodata(arg: u64) -> i64;
    fn _m1rf_inject_gp_noncanon(arg: u64) -> i64;
    fn _m1rf_inject_de_div0(arg: u64) -> i64;
    fn _m1rf_inject_bp_int3(arg: u64) -> i64;
}

/// The frozen S2 fault list (decision D-2): two #PF flavors, #GP, #DE, #BP.
/// The host verdict mirrors it (fault_id sequence, rip symbol prefixes,
/// magic registers); any extra or missing line is red.
pub const FAULT_IDS: [&str; 5] = [
    "m1rfault.pf.unmapped",
    "m1rfault.pf.rodata",
    "m1rfault.gp.noncanon",
    "m1rfault.de.div0",
    "m1rfault.bp.int3",
];

/// Index of the fault currently being injected (observer reads it).
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// M1RFault lines printed so far (sanity: exactly one per plan entry).
pub static OBSERVED: AtomicUsize = AtomicUsize::new(0);

/// The fault_id the observer must print for the in-flight injection.
pub fn current_fault_id() -> &'static str {
    let i = CURRENT.load(Ordering::SeqCst);
    FAULT_IDS.get(i).copied().unwrap_or("unexpected")
}

/// Called by traps.rs after printing one observed fault line.
pub fn note_observed() {
    OBSERVED.fetch_add(1, Ordering::SeqCst);
}

fn pf_unmapped(arg: u64) -> i64 {
    // SAFETY: single injection; faults recover through .m1r.fixup.
    unsafe { _m1rf_inject_pf_unmapped(arg) }
}
fn pf_rodata(arg: u64) -> i64 {
    // SAFETY: see pf_unmapped.
    unsafe { _m1rf_inject_pf_rodata(arg) }
}
fn gp_noncanon(arg: u64) -> i64 {
    // SAFETY: see pf_unmapped.
    unsafe { _m1rf_inject_gp_noncanon(arg) }
}
fn de_div0(arg: u64) -> i64 {
    // SAFETY: see pf_unmapped.
    unsafe { _m1rf_inject_de_div0(arg) }
}
fn bp_int3(arg: u64) -> i64 {
    // SAFETY: see pf_unmapped.
    unsafe { _m1rf_inject_bp_int3(arg) }
}

/// Fixed-frame driver hop: the injections push their own frame, so the
/// first backtrace entry symbolizes into this stable no-mangle name
/// (verdict-declared; the deeper Rust frames may inline/rename freely).
/// inline(never) + the volatile self-read keep LTO from folding the frame
/// away, so the return address at fault time always lands inside it.
#[inline(never)]
#[unsafe(no_mangle)]
extern "C" fn _m1r_fault_driver_frame(idx: u64, arg: u64) -> i64 {
    // black_box keeps LTO from tail-jumping into the injector: the call
    // must return through this frame so the fault-time backtrace head
    // stays _m1r_fault_driver_frame (verdict D-3).
    let r = core::hint::black_box(match idx {
        0 => pf_unmapped(arg),
        1 => pf_rodata(arg),
        2 => gp_noncanon(arg),
        3 => de_div0(arg),
        4 => bp_int3(arg),
        _ => -1,
    });
    r
}

/// Suite entry (never returns). PASS exits 0x11 -> QEMU 35 (D-1); any
/// failure exits 0x12 -> 37, distinct from PASS in this namespace.
pub fn run() -> ! {
    // Belt and braces for kernel-frame returns (never a sysretq here).
    // SAFETY: boot-time, single CPU, before any interrupt traffic.
    unsafe { traps::m1r_force_iret = 1 };
    traps::OBSERVE_FAULTS.store(true, Ordering::SeqCst);
    // Keep the driver frame symbol materialized under LTO (backtrace head).
    // SAFETY: reading one byte of text; the pointer is valid by construction.
    unsafe {
        core::ptr::read_volatile(_m1r_fault_driver_frame as *const u8);
    }

    let mut total = 0u8;
    let mut passed = 0u8;
    for (i, fault_id) in FAULT_IDS.iter().enumerate() {
        total += 1;
        if run_case(fault_id, inject_one(i)) {
            passed += 1;
        }
    }

    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m1r-fault run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m1r-fault run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x11 } else { 0x12 });
    loop {
        io::hlt();
    }
}

/// Inject fault `i`: exactly one M1RFault line must be observed, the fixup
/// must land, and the wrapper must return 0 (recovery contract).
fn inject_one(i: usize) -> bool {
    let before = OBSERVED.load(Ordering::SeqCst);
    CURRENT.store(i, Ordering::SeqCst);
    let r = _m1r_fault_driver_frame(i as u64, USER_UNMAPPED_VA as u64);
    CURRENT.store(usize::MAX, Ordering::SeqCst);
    let lines = OBSERVED.load(Ordering::SeqCst) - before;
    r == 0 && lines == 1
}

fn run_case(case_id: &str, passed: bool) -> bool {
    console::write_str("CASE ");
    console::write_str(case_id);
    console::write_str(if passed { " PASS\n" } else { " FAIL\n" });
    passed
}

fn write_decimal(value: u8) {
    const DIGITS: &[u8; 10] = b"0123456789";
    if value >= 10 {
        console::write_byte(DIGITS[usize::from(value / 10)]);
    }
    console::write_byte(DIGITS[usize::from(value % 10)]);
}
