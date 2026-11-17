//! UserPtr accessors on top of the fixup-guarded asm primitives.
#![allow(dead_code)] // S2 syscall copy layer; fault fixup via .m1r.fixup

use core::arch::global_asm;

global_asm!(
    include_str!("../arch/x86_64/uaccess.S"),
    options(att_syntax)
);

pub const EFAULT: i64 = -14;
pub const ENAMETOOLONG: i64 = -36;
/// Kernel string limit (PATH_MAX-like, xv6 uses shorter names).
pub const MAX_STR: usize = 128;

unsafe extern "C" {
    fn _copyout_bytes(dst: *const u8, src: *const u8, len: usize) -> i64;
    fn _copyin_bytes(dst: *mut u8, src: *const u8, len: usize) -> i64;
    fn _copyinstr(dst: *mut u8, src: *const u8, max: usize) -> i64;
    fn _m1r_probe_read(addr: *const u8) -> i64;
    fn _m1r_probe_write(addr: *mut u8, val: u8) -> i64;
}

/// copyout: kernel `src` -> user `dst` (fault-fixup guarded).
pub fn copyout(dst: *mut u8, src: *const u8, len: usize) -> i64 {
    // SAFETY: dst/src ranges are caller-contracted; faults recover as EFAULT.
    unsafe { _copyout_bytes(dst as *const u8, src, len) }
}

/// copyin: user `src` -> kernel `dst` (fault-fixup guarded).
pub fn copyin(dst: *mut u8, src: *const u8, len: usize) -> i64 {
    // SAFETY: see copyout.
    unsafe { _copyin_bytes(dst, src, len) }
}

/// copyinstr: copy a NUL-terminated string from user memory; the kernel
/// buffer must hold at least `max` bytes.
pub fn copyinstr(dst: &mut [u8], src: *const u8) -> i64 {
    // SAFETY: dst is a valid kernel slice of len >= max.
    unsafe { _copyinstr(dst.as_mut_ptr(), src, dst.len()) }
}

/// Never-faulting one-byte user read (returns the byte or EFAULT).
pub fn probe_read(addr: *const u8) -> i64 {
    // SAFETY: single byte, fixup guarded.
    unsafe { _m1r_probe_read(addr) }
}

/// Never-faulting one-byte write (returns 0 or EFAULT).
pub fn probe_write(addr: *mut u8, val: u8) -> i64 {
    // SAFETY: single byte, fixup guarded.
    unsafe { _m1r_probe_write(addr as *mut u8, val) }
}
