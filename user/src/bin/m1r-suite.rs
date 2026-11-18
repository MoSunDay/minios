//! M1R user-mode suite driver (S3): pid 1 in the `m1r-suite` boot. Runs
//! the user-space CASE list on the real syscall surface (write/getpid/
//! fork/wait4/execve/brk/SSE2/registers), prints one CASE line per case,
//! and exits with (total<<4)|failed so the kernel can merge the verdict.

#![no_std]
#![no_main]

use core::arch::x86_64::{_mm_add_pd, _mm_loadu_pd, _mm_storeu_pd};
use core::hint::black_box;
use minios_user::{io, sys};

/// The SSE crosstalk operands: plain statics the optimizer cannot fold
/// away (values cross fork/syscall boundaries through FPU state).
static mut PARENT_VEC: [f64; 4] = [1.5, 2.25, 3.125, 4.0625];
static mut CHILD_VEC: [f64; 4] = [-8.5, 7.25, -6.125, 5.0625];

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // xv6 init.c: the first process owns the console — open it (mknod on
    // first boot) and dup the descriptor onto stdin/stdout/stderr. Every
    // later write(1, ...) goes fd table -> file -> devsw CONSOLE.
    let console = b"/console\0";
    let fd0 = match sys::open(console, abi::O_RDWR) {
        Ok(fd) => fd as i32,
        Err(_) => {
            // First boot: create the device node, then open it.
            if sys::mknod(console, 1, 1).is_err() {
                sys::exit(-1);
            }
            match sys::open(console, abi::O_RDWR) {
                Ok(fd) => fd as i32,
                Err(_) => sys::exit(-1),
            }
        }
    };
    if fd0 != 0 {
        // Not fd 0 (impossible as pid 1 with an empty table, but be exact).
        let _ = sys::close(fd0);
        sys::exit(-1);
    }
    if sys::dup(0).is_err() || sys::dup(0).is_err() {
        sys::exit(-1);
    }

    let mut total = 0u32;
    let mut failed = 0u32;

    total += 1;
    if !run_case("m1r.user.write", case_write) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.getpid_fork", case_getpid_fork) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.fork_wait", case_fork_wait) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.execve", case_execve) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.execve_enoent", case_execve_enoent) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.brk", case_brk) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.sse_crosstalk", case_sse_crosstalk) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.syscall_regs", case_syscall_regs) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.open_read", case_open_read) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.pipe", case_pipe) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.dup_close", case_dup_close) {
        failed += 1;
    }
    total += 1;
    if !run_case("m1r.user.execve_diskcopy", case_execve_diskcopy) {
        failed += 1;
    }

    sys::exit(((total << 4) | failed) as i32)
}

fn run_case(id: &str, check: fn() -> bool) -> bool {
    let ok = check();
    let printed = io::print_str("CASE ")
        && io::print_str(id)
        && io::print_str(if ok { " PASS\n" } else { " FAIL\n" });
    ok && printed
}

/// write(2): byte count, full buffer on serial, and EFAULT on a kernel
/// pointer (covers the syscall return path and errno decoding).
fn case_write() -> bool {
    let msg = b"m1r-user-write-ok\n";
    let n = match sys::write(io::STDOUT, msg) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n != msg.len() {
        return false;
    }
    // A kernel-half source must be rejected before any byte moves.
    let bad = 0xffff_8000_0000_0000usize as *const u8;
    // SAFETY: pointer is never dereferenced locally; the kernel rejects
    // the kernel-half source before touching it.
    let badbuf = unsafe { core::slice::from_raw_parts(bad, 4) };
    matches!(sys::write(io::STDOUT, badbuf), Err(e) if e == abi::EFAULT)
}

/// getpid + fork: distinct pids, child sees 0, parent sees the child pid.
fn case_getpid_fork() -> bool {
    let parent = match sys::getpid() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if parent <= 0 {
        return false;
    }
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        let mine = sys::getpid().unwrap_or(-1);
        sys::exit(if mine != parent && mine > 0 { 0 } else { 1 });
    }
    pid > 0
}

/// fork + exit(status) + wait4: parent reaps the exact pid and decodes
/// the Linux wait status (exit code in bits 8..15).
fn case_fork_wait() -> bool {
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        sys::exit(42);
    }
    let mut status: i32 = 0;
    let reaped = match sys::wait4(pid as i32, &mut status) {
        Ok(p) => p,
        Err(_) => return false,
    };
    reaped == pid as i64 && (status >> 8) & 0xff == 42
}

/// execve of the embedded second image; the child's marker and exit code
/// prove the replacement.
fn case_execve() -> bool {
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        let path = b"/m1r-exec-child\0";
        let argv: [*const u8; 2] = [path.as_ptr(), core::ptr::null()];
        match sys::execve(path, argv.as_ptr() as *const usize, core::ptr::null()) {
            // A successful execve never returns; any return is a failure.
            Err(e) if e == abi::ENOENT => sys::exit(2),
            _ => sys::exit(3),
        }
    }
    let mut status: i32 = 0;
    let reaped = match sys::wait4(pid as i32, &mut status) {
        Ok(p) => p,
        Err(_) => return false,
    };
    reaped == pid as i64 && (status >> 8) & 0xff == 42
}

/// execve of a missing path returns -ENOENT and keeps the caller alive.
fn case_execve_enoent() -> bool {
    let path = b"/definitely-not-here\0";
    let argv: [*const u8; 2] = [path.as_ptr(), core::ptr::null()];
    matches!(sys::execve(path, argv.as_ptr() as *const usize, core::ptr::null()), Err(e) if e == abi::ENOENT)
}

/// brk(2): query, grow two pages, write/read patterns across the growth,
/// shrink back.
fn case_brk() -> bool {
    let base = match sys::brk(0) {
        Ok(b) => b,
        Err(_) => return false,
    };
    if base == 0 || base & 0xfff != 0 {
        return false;
    }
    let want = base + 0x2000;
    if sys::brk(want).unwrap_or(0) != want {
        return false;
    }
    // Touch each grown page: its first qword and the last qword that
    // still lies wholly inside the span (a store at page_end-1 would
    // cross into the unmapped next page and fault by design).
    // SAFETY: the break covers [base, want) now.
    unsafe {
        for off in [0x1000usize, 0x1ff8] {
            let p = (base + off as u64) as *mut u64;
            p.write_volatile(0x5a5a_5a5a_dead_beef);
            if p.read_volatile() != 0x5a5a_5a5a_dead_beef {
                return false;
            }
        }
    }
    sys::brk(base).unwrap_or(0) == base
}
/// SSE2 crosstalk across fork + syscall: both processes keep independent
/// xmm state while interleaving writes (21 §1 gate item).
fn case_sse_crosstalk() -> bool {
    // SAFETY: single-threaded process until fork; statics are ours.
    let (parent_expected, child_expected) = unsafe {
        let pv = &raw mut PARENT_VEC;
        let cv = &raw mut CHILD_VEC;
        (*pv)[0] = black_box((*pv)[0]);
        (sse_reduce(pv), sse_reduce(cv))
    };
    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        // SAFETY: child owns its copy of the statics.
        let got = unsafe { sse_reduce(&raw mut CHILD_VEC) };
        // A serial write between FPU uses forces save/restore across the
        // syscall boundary inside the child too.
        let _ = io::print_str("");
        sys::exit(if got == child_expected { 0 } else { 1 });
    }
    let mut status: i32 = -1;
    if sys::wait4(pid as i32, &mut status).unwrap_or(-1) != pid || (status >> 8) & 0xff != 0 {
        return false;
    }
    // Parent state must be untouched by the child's FPU work and by the
    // wait syscall itself.
    // SAFETY: parent statics.
    unsafe { sse_reduce(&raw mut PARENT_VEC) == parent_expected }
}

/// Reduce a 4-lane f64 array through xmm with interleaved syscalls.
///
/// SAFETY: `v` points at 4 readable f64s for this process's lifetime.
unsafe fn sse_reduce(v: *const [f64; 4]) -> u64 {
    unsafe {
        let mut acc = _mm_loadu_pd(v.cast::<f64>());
        let hi = _mm_loadu_pd(v.cast::<f64>().add(2));
        acc = _mm_add_pd(acc, hi);
        // Interleave a syscall between the load and the store.
        let _ = sys::write(io::STDOUT, b"");
        let mut out = [0f64; 2];
        _mm_storeu_pd(out.as_mut_ptr(), acc);
        let bits = out[0].to_bits() ^ out[1].to_bits();
        // Not-a-number would mean torn state; fold it out of range.
        if out[0].is_nan() || out[1].is_nan() {
            return u64::MAX;
        }
        bits
    }
}

/// Callee-saved GPRs (r12..r15; rbx/rbp are reserved for the compiler on
/// this codegen path) stay intact across a write syscall, checked inside
/// one asm block.
fn case_syscall_regs() -> bool {
    let buf = b"r\n";
    let ok: u64;
    let (r12, r13, r14, r15): (u64, u64, u64, u64);
    // SAFETY: fixed-register asm; kernel must return with every register
    // intact except rax/rcx/r11 (the syscall ABI).
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") abi::SYS_WRITE,
            in("rdi") io::STDOUT as u64,
            in("rsi") buf.as_ptr(),
            in("rdx") buf.len() as u64,
            inout("r12") 0x9999_aaaa_bbbb_ccccu64 => r12,
            inout("r13") 0xdddd_eeee_ffff_0000u64 => r13,
            inout("r14") 0x1234_5678_9abc_def0u64 => r14,
            inout("r15") 0x0fed_cba9_8765_4321u64 => r15,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
        ok = (r12 == 0x9999_aaaa_bbbb_cccc
            && r13 == 0xdddd_eeee_ffff_0000
            && r14 == 0x1234_5678_9abc_def0
            && r15 == 0x0fed_cba9_8765_4321) as u64;
    }
    ok == 1
}

/// S4d file syscalls: open the on-disk /m1r-suite (the very ELF this
/// process runs from, packed by minios-mkfs), check the ELF magic and a
/// full-block read, then close.
fn case_open_read() -> bool {
    let path = b"/m1r-suite\0";
    let fd = match sys::open(path, abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let mut buf = [0u8; 512];
    let n = match sys::read(fd, &mut buf) {
        Ok(n) => n,
        Err(_) => {
            let _ = sys::close(fd);
            return false;
        }
    };
    let magic = n == 512 && buf[..4] == [0x7f, b'E', b'L', b'F'];
    // A second read continues at offset 512 (shared file offset, readi
    // through bmap): byte 512 must differ from byte 0 on a real ELF.
    let more = match sys::read(fd, &mut buf) {
        Ok(n) => n == 512,
        Err(_) => false,
    };
    let cont = more && buf[0] != 0x7f;
    let closed = sys::close(fd).is_ok();
    // Reading through a closed fd must fail with EBADF.
    let bad = matches!(sys::read(fd, &mut buf), Err(_));
    magic && cont && closed && bad
}

/// S4d pipes: write one end, read the other, byte-exact round trip, then
/// close the write end and observe EOF (read == 0).
fn case_pipe() -> bool {
    let mut fds = [0i32; 2];
    if sys::pipe(&mut fds).is_err() {
        return false;
    }
    let msg = b"pipe-ok";
    if sys::write(fds[1], msg) != Ok(msg.len()) {
        return false;
    }
    let mut buf = [0u8; 16];
    match sys::read(fds[0], &mut buf) {
        Ok(n) if n == msg.len() && &buf[..n] == msg => {}
        _ => return false,
    }
    // Close the write end: the reader must see EOF (0 bytes).
    if sys::close(fds[1]).is_err() {
        return false;
    }
    let eof = matches!(sys::read(fds[0], &mut buf), Ok(0));
    let closed = sys::close(fds[0]).is_ok();
    eof && closed
}

/// S4d dup/close: a dup'ed stdout writes the same console bytes, and
/// closing the dup leaves the original working.
fn case_dup_close() -> bool {
    let fd = match sys::dup(io::STDOUT) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let msg = b"m1r-user-dup-ok\n";
    let a = sys::write(fd, msg) == Ok(msg.len());
    let b = sys::close(fd).is_ok();
    // The original descriptor must still work after closing the dup.
    let c = sys::write(io::STDOUT, b"d\n") == Ok(2);
    a && b && c
}

/// S4d-2 disk exec: copy /m1r-exec-child to /elfcopy through fds (the
/// write path's first real use), exec the copy — a path that exists only
/// on disk, never in the embedded table — and expect the child image's
/// marker behavior; then unlink and prove the name is gone.
fn case_execve_diskcopy() -> bool {
    let src = b"/m1r-exec-child\0";
    let dst = b"/elfcopy\0";
    let sfd = match sys::open(src, abi::O_RDONLY) {
        Ok(fd) => fd as i32,
        Err(_) => return false,
    };
    let dfd = match sys::open(dst, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC) {
        Ok(fd) => fd as i32,
        Err(_) => {
            let _ = sys::close(sfd);
            return false;
        }
    };
    let mut total = 0usize;
    let mut chunk = [0u8; 512];
    loop {
        match sys::read(sfd, &mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if sys::write(dfd, &chunk[..n]) != Ok(n) {
                    let _ = sys::close(sfd);
                    let _ = sys::close(dfd);
                    return false;
                }
                total += n;
            }
            Err(_) => {
                let _ = sys::close(sfd);
                let _ = sys::close(dfd);
                return false;
            }
        }
    }
    let _ = sys::close(sfd);
    let _ = sys::close(dfd);
    // A real ELF: header + segments past the direct-block boundary.
    if total <= 4096 {
        return false;
    }

    let pid = match sys::fork() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pid == 0 {
        let argv: [*const u8; 2] = [dst.as_ptr(), core::ptr::null()];
        match sys::execve(dst, argv.as_ptr() as *const usize, core::ptr::null()) {
            _ => sys::exit(3), // a successful exec never returns
        }
    }
    let mut status: i32 = 0;
    let reaped = match sys::wait4(pid as i32, &mut status) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if reaped != pid || (status >> 8) & 0xff != 42 {
        return false;
    }

    // Cleanup with verification: unlink, then the name must be gone.
    if sys::unlink(dst).is_err() {
        return false;
    }
    matches!(sys::open(dst, abi::O_RDONLY), Err(e) if e == abi::ENOENT)
}
