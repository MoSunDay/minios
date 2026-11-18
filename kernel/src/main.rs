//! Dual-mode kernel entry (D5): the Multiboot cmdline selects the boot
//! path. No cmdline (the frozen M0R ISO) or `m0r-smoke` keeps the frozen
//! M0R byte-identical smoke; `m1r-suite` / `m1r-fault` select the M1R
//! paths; any other cmdline boots the full system (sh, from S4 on).

#![no_std]
#![no_main]

mod arch;
mod console;
mod drivers;
mod fs;
mod kernel;
#[path = "lib/mod.rs"]
mod klib;
mod mm;
mod multiboot;
mod proc;

use kernel::ksymtab;
use multiboot::cmdline;

unsafe extern "C" {
    static _boot_stack_top: u8;
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main(multiboot_magic: u32, multiboot_info_phys: usize) {
    console::init_serial();
    // Panic markers name the active suite; the M0R default is frozen.
    ksymtab::set_active_suite("m0r");
    match boot_mode(multiboot_info_phys) {
        BootMode::M0rSmoke => {
            arch::x86_64::cpu::init(core::ptr::addr_of!(_boot_stack_top) as usize);
            kernel::m0r::run(multiboot_magic, multiboot_info_phys);
        }
        mode => m1r_boot(mode, multiboot_magic, multiboot_info_phys),
    }
}

enum BootMode {
    M0rSmoke,
    M1rSuite,
    M1rFault,
    Full,
}

fn boot_mode(mbi_phys: usize) -> BootMode {
    let cmd = cmdline(mbi_phys);
    if cmd.contains("m1r-suite") {
        BootMode::M1rSuite
    } else if cmd.contains("m1r-fault") {
        BootMode::M1rFault
    } else if cmd.is_empty() || cmd.contains("m0r-smoke") {
        BootMode::M0rSmoke
    } else {
        BootMode::Full
    }
}

fn m1r_boot(mode: BootMode, multiboot_magic: u32, mbi_phys: usize) {
    use arch::x86_64::io;
    if multiboot_magic != 0x2bad_b002 {
        m1r_fatal("not loaded by Multiboot");
    }
    // One-shot boot context, single CPU, interrupts off.
    arch::x86_64::cpu::init(core::ptr::addr_of!(_boot_stack_top) as usize);
    kernel::traps::init_idt();
    drivers::pic::init();
    kernel::time::init_pit();
    drivers::pic::unmask(0); // timer only
    proc::fpu::init_fpu();
    mm::frame::init(mbi_phys);
    mm::pgtable::set_boot_pml4(io::in64_cr3() as usize);
    match mode {
        // Kernel-mode expected-fault suite: CPL0 injections + fixup + the
        // M1RFault observer; exits 0x11 -> QEMU 35 on PASS (40 §2.1.6).
        BootMode::M1rFault => {
            ksymtab::set_active_suite("m1r-fault");
            kernel::faultsuite::run();
        }
        // Kernel-mode suite: CPL0 interrupt path, UserPtr fixup, ksym,
        // and (S4) the block layer against the mkfs root image.
        BootMode::M1rSuite => {
            ksymtab::set_active_suite("m1r");
            fs::buffer::binit();
            drivers::block::ide::ideinit();
            kernel::suite::run();
        }
        BootMode::Full => {
            ksymtab::set_active_suite("minios");
            // The interactive boot (init -> sh) needs the S4 file system.
            m1r_fatal("full boot arrives with the S4 fs chain");
        }
        BootMode::M0rSmoke => unreachable!(),
    }
}

pub(crate) fn m1r_fatal(why: &str) -> ! {
    console::write_str("MINIOS_X86_64_TEST_FATAL_V1 suite=");
    console::write_str(ksymtab::active_suite());
    console::write_str(" run=core-up reason=");
    console::write_str(why);
    console::write_str("\n");
    arch::x86_64::io::out32(0xf4, 0x12);
    loop {
        arch::x86_64::io::hlt();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    console::write_str("MINIOS_X86_64_TEST_FATAL_V1 suite=");
    console::write_str(ksymtab::active_suite());
    console::write_str(" run=core-up reason=panic at ");
    if let Some(loc) = info.location() {
        console::write_str(loc.file());
        console::write_str(":");
        console::write_hex(loc.line() as usize);
    } else {
        console::write_str("unknown");
    }
    console::write_str("\n");
    arch::x86_64::io::out32(0xf4, 0x12);
    loop {
        arch::x86_64::io::hlt();
    }
}
