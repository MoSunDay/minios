//! M4 kernel-mode suite: the VFS glue's namespace semantics on the
//! real mkfs root image (boot-context disk I/O through the polling
//! IDE path — the discipline init_m1r_fs established). Wave-1 cases
//! live in area submodules — `vfs` (namei: root/dots paths, DIRSIZ
//! truncation, negative-dentry flip, file-as-parent) and `fsops`
//! (create/link/unlink semantics with slot-ledger pins, inode-table
//! pressure) — while this file owns the merged runner table and the
//! verdict/exit path. Runs in the boot context before the scheduler
//! starts: absolute paths only (no proc cwd). Boot cmdline `m4-suite`
//! -> CASE lines -> merged verdict, exit 0x10 -> QEMU 33 (0x11 on
//! any failure).

mod fsops;
mod pc;
mod vfs;

use crate::arch::x86_64::io;
use crate::console;
use crate::fs::xv6fs::inode::Inode;

pub fn run() -> ! {
    // Boot-context disk I/O polls with interrupts enabled (the IDE
    // probe already ran sti/hlt cycles during init_m1r_fs; keep them
    // on for the suite's own reads/writes).
    io::sti();
    let checks: [(&str, fn() -> bool); 16] = [
        ("m4.namei.root_paths", vfs::case_namei_root_paths),
        ("m4.namei.dirsiz_trunc", vfs::case_namei_dirsiz_trunc),
        ("m4.namei.negative_flip", vfs::case_namei_negative_flip),
        ("m4.namei.file_as_parent", vfs::case_namei_file_as_parent),
        ("m4.fs.create_eexist", fsops::case_fs_create_eexist),
        ("m4.fs.dir_dots", fsops::case_fs_dir_dots),
        ("m4.fs.link_two_names", fsops::case_fs_link_two_names),
        ("m4.fs.unlink_notempty", fsops::case_fs_unlink_notempty),
        ("m4.fs.table_pressure", fsops::case_fs_table_pressure),
        ("m4.pc.hit_miss", pc::case_pc_hit_miss),
        ("m4.pc.share_handles", pc::case_pc_share_handles),
        ("m4.pc.fill_matches_readi", pc::case_pc_fill_matches_readi),
        ("m4.pc.evict_clean", pc::case_pc_evict_clean),
        ("m4.pc.gate_window", pc::case_pc_gate_window),
        ("m4.pc.readahead_seq", pc::case_pc_readahead_seq),
        ("m4.pc.probe_pressure", pc::case_pc_probe_pressure),
    ];
    let mut total = 0usize;
    let mut passed = 0usize;
    for (case_id, check) in checks {
        total += 1;
        let ok = check();
        console::write_str("CASE ");
        console::write_str(case_id);
        console::write_str(if ok { " PASS\n" } else { " FAIL\n" });
        if ok {
            passed += 1;
        }
    }
    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m4 run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m4 run=core-up total=");
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

/// free_slots after a zombie drain: the unlink family defers the
/// final (possibly truncating) iput to a later kern_* entry, so slot
/// accounting only settles once reclaim_zombies ran. The suite's
/// ledger pins go through this probe only.
pub(super) fn drained_slots() -> usize {
    crate::fs::vfs_glue::mutate::reclaim_zombies();
    crate::fs::xv6fs::inode::free_slots()
}

/// Resolve `path` and run `probe` on the inode with one reference
/// held; the reference always goes back (the harness itself never
/// pins a table slot). `probe` must be read-only: the fields it may
/// touch (inum/type_) are stable while the lookup holds a positive
/// reference.
pub(super) fn with_inode(path: &[u8], probe: impl FnOnce(&Inode) -> bool) -> bool {
    match crate::fs::vfs_glue::kern_namei(path) {
        Ok(ip) => {
            // SAFETY: kern_namei's idup'd reference; probe reads only.
            let ok = probe(unsafe { &*ip });
            // SAFETY: balancing put; a positive lookup keeps nlink>=1,
            // so this never takes the truncating path.
            unsafe { crate::fs::xv6fs::inode::iput(ip) };
            ok
        }
        Err(_) => false,
    }
}

fn write_decimal(mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &buf[i..] {
        crate::arch::x86_64::console::write_byte(d);
    }
}
