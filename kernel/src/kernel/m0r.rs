//! The M0R smoke milestone, frozen byte-for-byte (plans/linux26/40): the
//! `m0r-smoke` boot mode (and the no-cmdline default, matching the frozen
//! M0R ISO) runs exactly these cases against the boot page tables and
//! exits through the isa-debug-exit device. Do not reorder the output.

use crate::arch::x86_64::console;
use crate::arch::x86_64::io;
use crate::arch::x86_64::{KERNEL_VMA, PAGE_NX, PAGE_PRESENT, PAGE_WRITE};
use crate::arch::DATA_SENTINEL;

#[no_mangle]
static M0R_RODATA: u8 = 0x3c;

/// Serial and CPU state must already be initialized by main.
pub fn run(multiboot_magic: u32, multiboot_info_phys: usize) -> ! {
    console::write_str("minios x86_64 M0R boot\n");
    console::write_str("multiboot_info=");
    console::write_hex(multiboot_info_phys);
    console::write_str("\n");

    let mut total = 0;
    let mut passed = 0;

    total += 1;
    if run_case("m0r.multiboot", multiboot_magic == 0x2bad_b002) {
        passed += 1;
    }
    let checks: [(&str, fn() -> bool); 5] = [
        ("m0r.long_mode", || {
            (run as *const () as usize) >= KERNEL_VMA
        }),
        ("m0r.low_identity", low_identity_readable),
        ("m0r.text_permissions", text_permissions_executable),
        ("m0r.rodata_permissions", rodata_permissions_protected),
        ("m0r.data_permissions", data_permissions_protected),
    ];
    for (case_id, check) in checks {
        total += 1;
        if run_case(case_id, check()) {
            passed += 1;
        }
    }

    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m0r run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x10 } else { 0x11 });
    loop {}
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

fn low_identity_readable() -> bool {
    const BOOT_START: *const u8 = 0x0010_0000 as *const u8;
    // SAFETY: the low boot block remains mapped readable by the boot page tables.
    let first = unsafe { core::ptr::read_volatile(BOOT_START) };
    first == 0x02
}
fn text_permissions_executable() -> bool {
    matches!(translate(run as *const () as usize),
        Some((_, flags)) if flags & PAGE_PRESENT != 0
            && flags & PAGE_WRITE == 0 && flags & PAGE_NX == 0)
}

fn rodata_permissions_protected() -> bool {
    matches!(translate(core::ptr::addr_of!(M0R_RODATA) as usize),
        Some((_, flags)) if flags & PAGE_PRESENT != 0
            && flags & PAGE_WRITE == 0 && flags & PAGE_NX != 0)
}

fn data_permissions_protected() -> bool {
    matches!(translate(core::ptr::addr_of!(DATA_SENTINEL) as usize),
        Some((_, flags)) if flags & PAGE_PRESENT != 0
            && flags & PAGE_WRITE != 0 && flags & PAGE_NX != 0)
}

fn table_entry(table: usize, index: usize) -> u64 {
    // SAFETY: boot page tables are addressed through the low identity mapping.
    unsafe { core::ptr::read_volatile((table + index * 8) as *const u64) }
}

fn translate(virtual_address: usize) -> Option<(usize, u64)> {
    const PML4_ADDRESS: usize = 0x0020_0000;
    let pml4 = PML4_ADDRESS;
    let pml4e = table_entry(pml4, (virtual_address >> 39) & 0x1ff);
    if pml4e & PAGE_PRESENT == 0 {
        return None;
    }
    let pdpt = (pml4e & 0x000f_ffff_ffff_f000) as usize;
    let pdpte = table_entry(pdpt, (virtual_address >> 30) & 0x1ff);
    if pdpte & PAGE_PRESENT == 0 {
        return None;
    }
    let directory = (pdpte & 0x000f_ffff_ffff_f000) as usize;
    let directory_entry = table_entry(directory, (virtual_address >> 21) & 0x1ff);
    if directory_entry & PAGE_PRESENT == 0 {
        return None;
    }
    let table = (directory_entry & 0x000f_ffff_ffff_f000) as usize;
    let leaf = table_entry(table, (virtual_address >> 12) & 0x1ff);
    if leaf & PAGE_PRESENT == 0 {
        return None;
    }
    Some(((leaf & 0x000f_ffff_ffff_f000) as usize, leaf))
}
