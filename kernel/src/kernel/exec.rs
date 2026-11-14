//! exec.rs: xv6 exec.c + elf.h port — replace the current process with an
//! ELF program loaded from a file.
//!
//! Uses pgtable contract fns (see kernel/sched/mod.rs EXPECT block) and the
//! inode layer from crate::fs::xv6fs::inode (Agent C):
//!   namei(path) -> *mut Inode; ilock/iput/iunlockput(ip);
//!   readi(ip, dst, off, n) -> i32 (n on success, <0/short on failure).

use core::ptr::null_mut;

use crate::arch::x86::mm::layout::p2v;
use crate::arch::x86::mm::pgtable as vm;
use crate::fs::xv6fs::inode::{ilock, iunlockput, readi};
use crate::fs::xv6fs::namei::namei;
use crate::kernel::sched::{myproc, MAXARG, PTE_ADDR};
use crate::lib::elf::{parse_ehdr, parse_phdr, valid_magic, EHDR_SIZE, ELF_PROG_LOAD, PHDR_SIZE};
use crate::lib::string::{safestrcpy, strlen};
use crate::mm::frame::PGSIZE;

/// vm.c loaduvm(): load a program segment into the page table at va.
/// Returns 0 on success, -1 on failure.
///
/// SAFETY: `va..va+sz` must already be mapped in `pgdir` (allocuvm).
unsafe fn loadseg(
    pgdir: *mut u32,
    va: u32,
    ip: *mut crate::fs::xv6fs::inode::Inode,
    offset: u32,
    sz: u32,
) -> i32 {
    if va % PGSIZE as u32 != 0 {
        panic!("loaduvm: addr must be page aligned");
    }
    // xv6 loaduvm: copy the segment page by page; allocating one page per
    // loop iteration keeps multi-page .text/.rodata/.data segments intact.
    let mut i = 0u32;
    while i < sz {
        let pte = unsafe { vm::walkpgdir(pgdir, va + i, 0) };
        if pte.is_null() {
            return -1;
        }
        // SAFETY: pte points at the PTE slot for va+i.
        let pa = unsafe { *pte } & PTE_ADDR;
        let n = if sz - i < PGSIZE as u32 {
            sz - i
        } else {
            PGSIZE as u32
        };
        if unsafe { readi(ip, p2v(pa as usize) as *mut u8, offset + i, n) } != n as i32 {
            return -1;
        }
        i += PGSIZE as u32;
    }
    0
}

/// exec(path, argv): replace the current process image. `argv` is a
/// NUL-terminated-by-null-pointer array of user string pointers (already
/// validated by sys_exec). Returns 0 on success, -1 on failure (faithful
/// xv6 return convention).
///
/// SAFETY: called in process context (myproc()); on success never returns
/// to the old image's stack usage point except via trap frame rewrite.
pub unsafe fn exec(path: *const u8, argv: &[*const u8]) -> i32 {
    let proc = myproc();
    let mut ustack = [0u32; 3 + MAXARG + 1];

    let mut ip = unsafe { namei(path as *mut u8) };
    if ip.is_null() {
        return -1;
    }
    unsafe { ilock(ip) };
    let mut pgdir: *mut u32 = null_mut();

    let ok = 'body: {
        // Read + parse the ELF header.
        let mut ehdr_buf = [0u8; EHDR_SIZE];
        // SAFETY: readi writes at most EHDR_SIZE bytes into ehdr_buf.
        if unsafe { readi(ip, ehdr_buf.as_mut_ptr(), 0, EHDR_SIZE as u32) } != EHDR_SIZE as i32 {
            break 'body false;
        }
        let elf = match parse_ehdr(&ehdr_buf) {
            Some(e) => e,
            None => break 'body false,
        };
        if !valid_magic(&elf) {
            break 'body false;
        }

        let kd = unsafe { vm::setupkvm() };
        pgdir = kd;
        if kd.is_null() {
            break 'body false;
        }

        // Load program into memory.
        let mut sz: u32 = 0;
        let mut off = elf.phoff;
        for _ in 0..elf.phnum {
            let mut phdr_buf = [0u8; PHDR_SIZE];
            // SAFETY: readi writes at most PHDR_SIZE bytes into phdr_buf.
            if unsafe { readi(ip, phdr_buf.as_mut_ptr(), off, PHDR_SIZE as u32) }
                != PHDR_SIZE as i32
            {
                break 'body false;
            }
            let ph = match parse_phdr(&phdr_buf) {
                Some(p) => p,
                None => break 'body false,
            };
            off += PHDR_SIZE as u32;
            if ph.type_ != ELF_PROG_LOAD {
                continue;
            }
            if ph.memsz < ph.filesz {
                break 'body false;
            }
            sz = unsafe { vm::allocuvm(kd, sz, ph.vaddr.wrapping_add(ph.memsz)) };
            if sz == 0 {
                break 'body false;
            }
            if unsafe { loadseg(kd, ph.vaddr, ip, ph.off, ph.filesz) } < 0 {
                break 'body false;
            }
        }
        unsafe { crate::fs::xv6fs::inode::iunlockput(ip) };
        ip = null_mut();

        // Allocate two pages at the next page boundary.
        // Make the first inaccessible. Use the second as the user stack.
        sz = (sz + PGSIZE as u32 - 1) & !(PGSIZE as u32 - 1); // PGROUNDUP
        sz = unsafe { vm::allocuvm(kd, sz, sz.wrapping_add(2 * PGSIZE as u32)) };
        if sz == 0 {
            break 'body false;
        }
        unsafe { vm::clearpteu(kd, sz - 2 * PGSIZE as u32) };
        let mut sp = sz;

        // Push argument strings, prepare rest of stack in ustack.
        let mut argc: u32 = 0;
        for a in argv.iter() {
            if *a as usize == 0 {
                break;
            }
            if argc as usize >= MAXARG {
                break 'body false;
            }
            // SAFETY: argv strings were validated by fetchstr.
            let len = strlen(*a) as u32;
            sp = (sp.wrapping_sub(len + 1)) & !3;
            if unsafe { vm::copyout(kd, sp, *a, len + 1) } < 0 {
                break 'body false;
            }
            ustack[3 + argc as usize] = sp;
            argc += 1;
        }
        ustack[3 + argc as usize] = 0; // argv sentinel

        ustack[0] = 0xffff_ffff; // fake return PC
        ustack[1] = argc;
        ustack[2] = sp.wrapping_sub((argc + 1) * 4); // argv pointer

        sp -= (3 + argc + 1) * 4;
        if unsafe { vm::copyout(kd, sp, ustack.as_ptr() as *const u8, (3 + argc + 1) * 4) } < 0 {
            break 'body false;
        }

        // Save program name for debugging.
        let mut last = path;
        let mut s = path;
        // SAFETY: path is a validated NUL-terminated user string.
        unsafe {
            while *s != 0 {
                if *s == b'/' {
                    last = s.add(1);
                }
                s = s.add(1);
            }
            safestrcpy((*proc).name.as_mut_ptr(), last, 16);
        }

        // Commit to the user image.
        let oldpgdir = unsafe { (*proc).pgdir };
        unsafe {
            (*proc).pgdir = kd;
            (*proc).sz = sz;
            (*(*proc).tf).eip = elf.entry; // main
            (*(*proc).tf).esp = sp;
            vm::switchuvm((*proc).pgdir, (*proc).tf);
            vm::freevm(oldpgdir);
        }
        return 0;
    };

    // bad:
    if !ok {
        if !pgdir.is_null() {
            unsafe { vm::freevm(pgdir) };
        }
        if !ip.is_null() {
            unsafe { iunlockput(ip) };
        }
    }
    -1
}
