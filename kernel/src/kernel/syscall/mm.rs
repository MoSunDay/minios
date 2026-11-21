use super::{EINVAL, ENOMEM};
use abi::{MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE};

/// brk(2): addr 0 queries the current break; otherwise move it to the
/// byte-exact addr, returning the new break or the old one on failure.
pub(super) fn sys_brk(addr: u64) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: myproc() is the running (or seeded) process.
    let mm = match unsafe { (*p).mm.as_mut() } {
        Some(m) => m,
        None => return 0,
    };
    if addr == 0 {
        return mm.brk as i64;
    }
    mm.set_brk(addr as usize) as i64
}

/// mmap(2) M2 anonymous subset: MAP_ANONYMOUS|MAP_PRIVATE only (fd
/// ignored, offset must be 0); addr 0 places top-down, MAP_FIXED pins.
/// Returns the mapping start or -errno.
pub(super) fn sys_mmap(addr: usize, len: usize, prot: i64, flags: i64, _fd: i64, off: i64) -> i64 {
    const FLAG_BITS: i64 = (MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED) as i64;
    const PROT_BITS: i64 = (abi::PROT_READ | abi::PROT_WRITE | abi::PROT_EXEC) as i64;
    if flags & (MAP_PRIVATE as i64) == 0 || flags & (MAP_ANONYMOUS as i64) == 0 {
        return EINVAL;
    }
    if (flags & !FLAG_BITS) != 0 {
        return EINVAL;
    }
    if off != 0 {
        return EINVAL;
    }
    if (prot & !PROT_BITS) != 0 {
        return EINVAL;
    }
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return ENOMEM;
    }
    let kprot = (prot & PROT_BITS) as u32;
    // vma.rs PROT_* bit values match the abi's (1, 2, 4).
    let fixed = flags & (MAP_FIXED as i64) != 0;
    // SAFETY: myproc() is the running process.
    let r = match unsafe { (*p).mm.as_mut() } {
        Some(mm) => mm.mmap_anon(addr, len, kprot, fixed),
        None => 0,
    };
    if r == 0 {
        ENOMEM
    } else {
        r as i64
    }
}

/// munmap(2): anonymous mappings only (M2 minimal set); 0 or -errno.
pub(super) fn sys_munmap(addr: usize, len: usize) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return EINVAL;
    }
    // SAFETY: myproc() is the running process.
    match unsafe { (*p).mm.as_mut() } {
        Some(mm) => {
            if mm.munmap(addr, len) {
                0
            } else {
                EINVAL
            }
        }
        None => EINVAL,
    }
}

/// mprotect(2): 0 or -errno (EINVAL on bad prot, ENOMEM on holes).
pub(super) fn sys_mprotect(addr: usize, len: usize, prot: i64) -> i64 {
    const PROT_BITS: i64 = (abi::PROT_READ | abi::PROT_WRITE | abi::PROT_EXEC) as i64;
    if (prot & !PROT_BITS) != 0 {
        return EINVAL;
    }
    let kprot = (prot & PROT_BITS) as u32;
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return EINVAL;
    }
    // SAFETY: myproc() is the running process.
    match unsafe { (*p).mm.as_mut() } {
        Some(mm) => {
            if mm.mprotect(addr, len, kprot) {
                0
            } else {
                ENOMEM
            }
        }
        None => ENOMEM,
    }
}
