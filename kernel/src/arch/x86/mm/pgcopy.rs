//! pgcopy: user-page access helpers split out of vm.c for the 400-line
//! file limit — clearpteu/uva2ka/copyout/copyin. Re-exported through
//! pgtable so integration code can keep using the vm.c names.
//!
//! SAFETY contract: every function takes a live page directory.

use core::ptr::null_mut;

use crate::arch::x86::mm::layout::p2v;
use crate::arch::x86::mm::pgtable::{pgrounddown, pte_addr, walkpgdir, Pde, PTE_P, PTE_U};
use crate::lib::string::memmove;
use crate::mm::frame::PGSIZE;

// ---- PTE utilities for exec/fork/sysfile (vm.c:325..) --------------------

/// clearpteu (vm.c:325): clear PTE_U on the PTE for `uva`.
///
/// SAFETY: pgdir must be a live page directory with `uva` mapped.
pub unsafe fn clearpteu(pgdir: *mut Pde, uva: u32) {
    unsafe {
        let pte = walkpgdir(pgdir, uva, 0);
        if pte.is_null() {
            panic!("clearpteu");
        }
        *pte &= !PTE_U;
    }
}

/// uva2ka (vm.c:309): kernel alias of a user page, or null when the page
/// is absent or not user-accessible.
///
/// SAFETY: pgdir must be a live page directory.
pub unsafe fn uva2ka(pgdir: *mut Pde, uva: u32) -> *mut u8 {
    unsafe {
        let pte = walkpgdir(pgdir, uva, 0);
        if pte.is_null() || *pte & PTE_P == 0 || *pte & PTE_U == 0 {
            return null_mut();
        }
        p2v(pte_addr(*pte)) as *mut u8
    }
}

/// copyout (vm.c:337): copy `len` bytes from kernel buffer `src` into
/// user virtual `dstva`. 0 on success, -1 on a bad user page.
pub unsafe fn copyout(pgdir: *mut Pde, mut va: u32, src: *const u8, mut len: u32) -> i32 {
    unsafe {
        let mut buf = src;
        while len > 0 {
            let va0 = pgrounddown(va as usize) as u32;
            let pa0 = uva2ka(pgdir, va0);
            if pa0.is_null() {
                return -1;
            }
            let mut n = (PGSIZE as u32) - (va - va0);
            if n > len {
                n = len;
            }
            memmove(pa0.add((va - va0) as usize), buf, n as usize);
            len -= n;
            buf = buf.add(n as usize);
            va = va0 + PGSIZE as u32;
        }
    }
    0
}

/// copyin: mirror of copyout (kernel reads from user memory). Not in
/// xv6-public's vm.c, but expected by the syscall layer contract.
pub unsafe fn copyin(pgdir: *mut Pde, dst: *mut u8, mut srcva: u32, mut len: u32) -> i32 {
    unsafe {
        let mut buf = dst;
        while len > 0 {
            let va0 = pgrounddown(srcva as usize) as u32;
            let pa0 = uva2ka(pgdir, va0);
            if pa0.is_null() {
                return -1;
            }
            let mut n = (PGSIZE as u32) - (srcva - va0);
            if n > len {
                n = len;
            }
            memmove(buf, pa0.add((srcva - va0) as usize), n as usize);
            len -= n;
            buf = buf.add(n as usize);
            srcva = va0 + PGSIZE as u32;
        }
    }
    0
}
