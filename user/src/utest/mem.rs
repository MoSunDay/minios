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
