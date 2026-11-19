//! xv6 mem.c bsstest + the sbrktest subset (S4e-2): BSS zero-fill from
//! exec, and brk grow/touch/shrink/re-grow. The kernel-memory probe and
//! the OOM fork storm stay deferred (M2, coverage.toml ADR).

use crate::sys;

/// BSS beyond the ELF filesz must arrive zeroed (exec loadseg zero-fill).
static mut UNINIT: [u8; 10000] = [0; 10000];

pub fn case_bsstest() -> bool {
    // SAFETY: read-only scan of this process's own BSS.
    let uninit = unsafe { &*core::ptr::addr_of!(UNINIT) };
    uninit.iter().all(|&b| b == 0)
}

/// sbrk subset: 1 MiB grow with a per-page write/read pattern, a shrink
/// back to the original break, and a re-grow proving dealloc was real.
pub fn case_sbrk() -> bool {
    let old = match sys::sbrk(0) {
        Ok(a) => a,
        Err(_) => return false,
    };
    const GROW: usize = 1 << 20;
    const PAGE: usize = 4096;
    match sys::sbrk(GROW as i64) {
        Ok(a) => {
            if a != old {
                return false;
            }
        }
        Err(_) => return false,
    }
    match sys::sbrk(0) {
        Ok(a) => {
            if a as usize != old as usize + GROW {
                return false;
            }
        }
        Err(_) => return false,
    }
    let base = old as usize;
    let mut off = 0;
    while off < GROW {
        // SAFETY: this page is inside the grown break.
        let p = (base + off) as *mut u8;
        unsafe { *p = (off / PAGE) as u8 };
        off += PAGE;
    }
    let mut off = 0;
    while off < GROW {
        // SAFETY: read back the same page.
        let p = (base + off) as *const u8;
        let got = unsafe { *p };
        if got != (off / PAGE) as u8 {
            return false;
        }
        off += PAGE;
    }
    match sys::sbrk(-(GROW as i64)) {
        Ok(a) => {
            if a as usize != base + GROW {
                return false;
            }
        }
        Err(_) => return false,
    }
    match sys::sbrk(0) {
        Ok(a) => {
            if a != old {
                return false;
            }
        }
        Err(_) => return false,
    }
    match sys::sbrk(PAGE as i64) {
        Ok(a) => {
            if a != old {
                return false;
            }
        }
        Err(_) => return false,
    }
    // SAFETY: the first byte of the re-grown page.
    let p = old as usize as *mut u8;
    unsafe { *p = 0x5a };
    unsafe { *p == 0x5a }
}

// ---- M2 anonymous mmap/munmap/COW (coverage.toml xv6.mem.mmap/.munmap/.cow) ----

const PAGE: usize = 4096;
const ANON: u64 = abi::MAP_PRIVATE | abi::MAP_ANONYMOUS;

/// Anonymous mmap RW: two placements are disjoint and ordered (top-down),
/// pages arrive zeroed, writes read back, munmap frees both.
pub fn case_mmap() -> bool {
    let a = match sys::mmap(0, 2 * PAGE, abi::PROT_READ | abi::PROT_WRITE, ANON, -1, 0) {
        Ok(p) if p != 0 => p,
        _ => return false,
    };
    let b = match sys::mmap(0, PAGE, abi::PROT_READ | abi::PROT_WRITE, ANON, -1, 0) {
        Ok(p) if p != 0 => p,
        _ => return false,
    };
    // Top-down placement: the second mapping sits below the first.
    if b + PAGE > a {
        return false;
    }
    // Fresh pages read zero.
    // SAFETY: [a, a+2*PAGE) is a fresh private mapping.
    unsafe {
        for i in 0..(2 * PAGE) {
            if ((a + i) as *const u8).read_volatile() != 0 {
                return false;
            }
        }
        // Write pattern: page index in the first byte of each page.
        for pg in 0..2 {
            let p = (a + pg * PAGE) as *mut u8;
            p.write_volatile(0xa0 + pg as u8);
            if p.read_volatile() != 0xa0 + pg as u8 {
                return false;
            }
        }
    }
    sys::munmap(a, 2 * PAGE).is_ok() && sys::munmap(b, PAGE).is_ok()
}

/// munmap really removes the pages: a forked child touching the unmapped
/// range is killed by its fault; a live parent keeps running. munmap of
/// non-anonymous memory (the stack) is rejected with EINVAL.
pub fn case_munmap() -> bool {
    let a = match sys::mmap(0, PAGE, abi::PROT_READ | abi::PROT_WRITE, ANON, -1, 0) {
        Ok(p) if p != 0 => p,
        _ => return false,
    };
    // SAFETY: one fresh mapped page.
    unsafe { (a as *mut u8).write_volatile(1) };
    if sys::munmap(a, PAGE).is_err() {
        return false;
    }
    // The stack is not anonymous-mmap memory: the M2 minimal set refuses.
    let local = 0u8;
    let sp = &local as *const u8 as usize;
    if sys::munmap(sp & !(PAGE - 1), PAGE).is_ok() {
        return false;
    }
    let pid = match sys::fork() {
        Ok(pid) => pid as i32,
        Err(_) => return false,
    };
    if pid == 0 {
        // SAFETY: intentionally touch unmapped memory; the child dies on
        // its page fault and this write must never succeed.
        unsafe { (a as *mut u8).write_volatile(2) };
        sys::exit(0);
    }
    let mut status = 0i32;
    let reaped = match sys::wait4(pid, &mut status) {
        Ok(p) => p as i32,
        Err(_) => return false,
    };
    reaped == pid && status != 0
}

/// fork page isolation (COW in M2): a static page written by the child
/// never leaks into the parent, and both exit cleanly.
static mut COW_PAGE: [u8; PAGE] = [0; PAGE];

pub fn case_cow() -> bool {
    let pid = match sys::fork() {
        Ok(pid) => pid as i32,
        Err(_) => return false,
    };
    // SAFETY: the child owns a private copy of COW_PAGE (COW or eager).
    unsafe {
        if pid == 0 {
            COW_PAGE[0] = 0x5a;
            COW_PAGE[PAGE - 1] = 0xa5;
            if COW_PAGE[0] != 0x5a || COW_PAGE[PAGE - 1] != 0xa5 {
                sys::exit(1);
            }
            sys::exit(0);
        }
    }
    let mut status = 0i32;
    let reaped = match sys::wait4(pid, &mut status) {
        Ok(p) => p as i32,
        Err(_) => return false,
    };
    // SAFETY: parent's page untouched (isolation), child exited 0.
    unsafe { reaped == pid && status == 0 && COW_PAGE[0] == 0 && COW_PAGE[PAGE - 1] == 0 }
}
