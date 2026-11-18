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
    let checks: [(&str, fn() -> bool); 8] = [
        ("m1r.irq.pit", case_irq_pit),
        ("m1r.uaccess.copyin_efault", case_copyin_efault),
        ("m1r.uaccess.copyout_efault", case_copyout_efault),
        ("m1r.uaccess.kernelptr_efault", case_kernelptr_efault),
        ("m1r.ksym.self", case_ksym_self),
        ("m1r.fs.blockread", case_fs_blockread),
        ("m1r.fs.inode", case_fs_inode),
        ("m1r.fs.file", case_fs_file),
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
    // Snapshot semantics: earlier boot code (S4c loginit polls the IDE
    // disk with sti/hlt before the suite) may already have taken timer
    // ticks — that delivery proves the very path under test. Waiting for
    // a NEW edge keeps the case honest: a dead PIT never advances past t0,
    // hlt never returns, and the runner deadline produces the TIMEOUT
    // evidence (never a false PASS).
    let t0 = super::time::ticks();
    // Boot context on the boot stack; the IRQ frame nests below rsp.
    io::sti();
    while super::time::ticks() == t0 {
        io::hlt();
    }
    io::cli();
    super::time::ticks() > t0
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

/// S4c inode/namei layer: resolve /m1r-suite and /m1r-exec-child through
/// namei -> iget -> ilock -> readi and compare the file contents with the
/// embedded userimg ELFs (the very bytes minios-mkfs packed into the root
/// image). The suite ELF spans 36 blocks > NDIRECT(12), so readi walks the
/// indirect-block path through bmap. /enoent must fail like fs.c's miss
/// path. Read-only by design (40 §2.2 per-round image accounting).
fn case_fs_inode() -> bool {
    use crate::fs::xv6fs::inode::{ilock, iunlockput, readi};
    use crate::fs::xv6fs::layout::T_FILE;
    use crate::fs::xv6fs::namei::namei;

    let check = |path: &str| -> bool {
        let Some(img) = super::userimg::lookup(path.as_bytes()) else {
            return false;
        };
        let mut cpath = [0u8; 20];
        cpath[..path.len()].copy_from_slice(path.as_bytes());
        // SAFETY: cpath is NUL-terminated; the inode protocol is balanced
        // (namei -> ilock -> readi -> iunlockput), all under icache locks.
        let ip = unsafe { namei(cpath.as_ptr()) };
        if ip.is_null() {
            return false;
        }
        unsafe { ilock(ip) };
        // SAFETY: ip is I_BUSY; header fields read under the inode lock.
        let ok = unsafe { (*ip).type_ == T_FILE && (*ip).size as usize == img.len() };
        let mut buf = [0u8; 64];
        // Head 64 bytes straight off block 0 of the file.
        let head = ok && unsafe { readi(ip, buf.as_mut_ptr(), 0, 64) == 64 } && buf == img[..64];
        // Tail 64 bytes: starts mid-block (size % BSIZE != 0) and crosses
        // the direct/indirect boundary for a 36-block file.
        let off = (img.len() - 64) as u32;
        let tail = head
            && unsafe { readi(ip, buf.as_mut_ptr(), off, 64) == 64 }
            && buf == img[off as usize..];
        unsafe { iunlockput(ip) };
        tail
    };

    // SAFETY: NUL-terminated literal.
    let miss = unsafe { namei(b"/enoent\0".as_ptr()) }.is_null();
    check("/m1r-suite") && check("/m1r-exec-child") && miss
}

/// S4d file.c: the descriptor-table layer in boot context (no myproc/fd
/// path): filealloc + FileState::Inode following sys_open's exact
/// ownership transfer (namei -> ilock -> file state -> iunlock), then
/// fileread's shared-offset reads against the embedded ELF bytes,
/// filestat/stati metadata, and fileclose's ref drop to Closed.
fn case_fs_file() -> bool {
    use crate::fs::file_table::{filealloc, fileclose, fileread, filestat, FileState};
    use crate::fs::xv6fs::inode::{ilock, iunlock, iunlockput};
    use crate::fs::xv6fs::layout::{Stat, T_FILE};
    use crate::fs::xv6fs::namei::namei;

    let Some(img) = super::userimg::lookup(b"/m1r-suite") else {
        return false;
    };
    let mut cpath = [0u8; 11];
    cpath[..10].copy_from_slice(b"/m1r-suite");
    // SAFETY: mirrors sys_open: namei hands an iget'd reference, ilock
    // makes it valid, the reference moves into the file slot, and
    // fileclose later drops it through begin_op/iput/end_op.
    let ip = unsafe { namei(cpath.as_ptr()) };
    if ip.is_null() {
        return false;
    }
    unsafe { ilock(ip) };
    let f = unsafe { filealloc() };
    if f.is_null() {
        unsafe { iunlockput(ip) };
        return false;
    }
    // SAFETY: f is ref 1 from filealloc; state handoff per sys_open.
    unsafe {
        (*f).state = FileState::Inode { ip, off: 0 };
        (*f).readable = 1;
        (*f).writable = 0;
        iunlock(ip);
    }

    // Two 64-byte reads: bytes land in the kernel buffer and the file
    // offset advances (second read returns the next block's bytes).
    let mut buf = [0u8; 64];
    let n1 = unsafe { fileread(f, buf.as_mut_ptr(), 64) };
    let head = n1 == 64 && buf[..] == img[..64];
    let mut b2 = [0u8; 64];
    let n2 = unsafe { fileread(f, b2.as_mut_ptr(), 64) };
    let cont = head && n2 == 64 && b2[..] == img[64..128];

    // filestat: stati snapshot taken under ilock must name the same file.
    let mut st = Stat {
        type_: 0,
        dev: 0,
        ino: 0,
        nlink: 0,
        size: 0,
    };
    let stat_ok = cont
        && unsafe { filestat(f, &mut st) == 0 }
        && st.type_ == T_FILE
        && st.size == img.len() as u32;

    // Drop the last reference: the slot must return to Closed for reuse,
    // and the second allocation must find the same slot free again.
    unsafe { fileclose(f) };
    let slot_ok = unsafe { (*f).ref_ == 0 && matches!((*f).state, FileState::Closed) };
    let f2 = unsafe { filealloc() };
    let realloc = !f2.is_null();
    if realloc {
        // Return the probe slot (Closed state -> ref drop is a no-op).
        unsafe { fileclose(f2) };
    }
    stat_ok && slot_ok && realloc
}

fn write_decimal(value: u8) {
    const DIGITS: &[u8; 10] = b"0123456789";
    if value >= 10 {
        console::write_byte(DIGITS[usize::from(value / 10)]);
    }
    console::write_byte(DIGITS[usize::from(value % 10)]);
}
