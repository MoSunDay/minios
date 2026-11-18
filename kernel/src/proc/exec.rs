//! exec: replace the current process image with a static ELF64 loaded from
//! a memory blob (xv6 exec.c semantics; the blob is an embedded init or an
//! S4 file read). Stack layout follows the Linux x86-64 process start ABI
//! (argc / argv[] / NULL / envp[] / NULL / AT_NULL auxv, strings above).

use super::proc;
use super::Proc;
use crate::arch::x86_64::io;
use crate::kernel::traps::Trapframe;
use crate::mm::frame::{kalloc, kfree, p2v, v2p, PGSIZE};
use crate::mm::pgtable::{self, PTE_NX, PTE_P};
use crate::mm::vm::{Mm, USER_STACK_PAGES, USER_STACK_TOP};

const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;

pub const ENOEXEC: i64 = -8;
pub const ENOMEM: i64 = -12;

#[repr(C)]
#[derive(Clone, Copy)]
struct Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
}

fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn le64(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Validate the ELF64 header and hand back (entry, phoff, phnum).
fn parse_ehdr(b: &[u8]) -> Result<(u64, usize, usize), i64> {
    if b.len() < EHDR_SIZE || le32(b, 0) != 0x464C_457F {
        return Err(ENOEXEC);
    }
    if b[4] != 2 || b[5] != 1 {
        return Err(ENOEXEC); // ELF64 little-endian only
    }
    if le16(b, 16) != ET_EXEC || le16(b, 18) != EM_X86_64 {
        return Err(ENOEXEC); // static x86-64 executables only
    }
    let phoff = le64(b, 32) as usize;
    let phentsize = le16(b, 54) as usize;
    let phnum = le16(b, 56) as usize;
    if phentsize != PHDR_SIZE || phoff.saturating_add(phnum * PHDR_SIZE) > b.len() {
        return Err(ENOEXEC);
    }
    Ok((le64(b, 24), phoff, phnum))
}

fn phdr_at(b: &[u8], phoff: usize, i: usize) -> Phdr {
    let o = phoff + i * PHDR_SIZE;
    Phdr {
        p_type: le32(b, o),
        p_flags: le32(b, o + 4),
        p_offset: le64(b, o + 8),
        p_vaddr: le64(b, o + 16),
        p_filesz: le64(b, o + 32),
        p_memsz: le64(b, o + 40),
    }
}

/// Copy `src` to user va `va..va+len` through the direct map.
///
/// # Safety
/// va..va+len is mapped in `pml4`; pml4 is not the active CR3.
unsafe fn write_user(pml4: pgtable::PtPtr, va: usize, src: &[u8]) {
    let mut off = 0;
    while off < src.len() {
        let page_va = pgtable::pgrounddown(va + off);
        let pte = pgtable::walk(pml4, page_va, false);
        assert!(
            !pte.is_null() && (*pte & PTE_P) != 0,
            "exec: unmapped segment page"
        );
        let base = p2v((*pte & pgtable::PTE_ADDR) as usize);
        let in_page = (va + off) - page_va;
        let n = (PGSIZE - in_page).min(src.len() - off);
        // SAFETY: direct-map destination inside one owned page.
        unsafe {
            core::ptr::copy_nonoverlapping(src[off..].as_ptr(), (base + in_page) as *mut u8, n);
        }
        off += n;
    }
}

/// Set/clear NX across [va, va+size) leaf PTEs.
///
/// # Safety
/// range mapped in pml4.
unsafe fn set_nx(pml4: pgtable::PtPtr, va: usize, size: usize, on: bool) {
    let mut a = pgtable::pgrounddown(va);
    let end = pgtable::pgroundup(va + size);
    while a < end {
        let pte = pgtable::walk(pml4, a, false);
        if !pte.is_null() && (*pte & PTE_P) != 0 {
            if on {
                *pte |= PTE_NX;
            } else {
                *pte &= !PTE_NX;
            }
        }
        a += PGSIZE;
    }
}

/// Build the initial user stack in kernel-owned pages, map it below
/// USER_STACK_TOP, and return the initial rsp.
///
/// # Safety
/// stack pages are freshly kalloc'd; pml4 not active.
unsafe fn build_stack(pml4: pgtable::PtPtr, argv: &[&[u8]]) -> Result<usize, i64> {
    let total = USER_STACK_PAGES * PGSIZE;
    let stack_top_va = USER_STACK_TOP;
    let stack_base_va = stack_top_va - total;

    // String area grows down from the very top of the stack region.
    let mut sp = stack_top_va;
    let mut addrs = [0usize; 16];
    if argv.len() > 15 {
        return Err(ENOEXEC);
    }
    for (i, arg) in argv.iter().enumerate().rev() {
        sp -= arg.len() + 1;
        addrs[i] = sp;
    }
    // Pointer area: argc + argv[] + NULL + envp NULL + AT_NULL pair.
    let words = 1 + argv.len() + 1 + 1 + 2;
    sp &= !0xF;
    sp -= words * 8;
    if sp < stack_base_va + 512 {
        return Err(ENOMEM); // not enough room for argv
    }

    // Materialize both areas into the kernel pages, then map them.
    let mut pages = [core::ptr::null_mut(); USER_STACK_PAGES];
    for page in pages.iter_mut() {
        *page = kalloc();
        if page.is_null() {
            for q in pages.iter() {
                if !q.is_null() {
                    kfree(*q);
                }
            }
            return Err(ENOMEM);
        }
        // SAFETY: fresh page.
        crate::klib::string::memset(*page, 0, PGSIZE);
    }
    let mut put = |va: usize, bytes: &[u8]| -> bool {
        let off = va - stack_base_va;
        let page = &mut pages[off / PGSIZE];
        let in_page = off % PGSIZE;
        let n = bytes.len().min(PGSIZE - in_page);
        // SAFETY: destination inside the owned scratch page.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), page.add(in_page), n) };
        n == bytes.len()
    };
    for (i, arg) in argv.iter().enumerate() {
        put(addrs[i], arg);
        put(addrs[i] + arg.len(), &[0u8]);
    }
    let mut word_area = [0u64; 16 + 6];
    let mut w = 0;
    word_area[w] = argv.len() as u64;
    w += 1;
    for a in addrs.iter().take(argv.len()) {
        word_area[w] = *a as u64;
        w += 1;
    }
    word_area[w] = 0; // argv NULL
    word_area[w + 1] = 0; // envp NULL
    word_area[w + 2] = 0; // AT_NULL
    word_area[w + 3] = 0;
    w += 4;
    let mut bytes = [0u8; (16 + 6) * 8];
    for (i, word) in word_area.iter().take(w).enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    if !put(sp, &bytes[..w * 8]) {
        for q in pages.iter() {
            if !q.is_null() {
                kfree(*q);
            }
        }
        return Err(ENOMEM);
    }
    for (i, page) in pages.iter().enumerate() {
        // SAFETY: fresh page, fresh mapping slot.
        if unsafe {
            pgtable::mappages(
                pml4,
                stack_base_va + i * PGSIZE,
                PGSIZE,
                v2p(*page as usize),
                pgtable::PTE_W | pgtable::PTE_U | PTE_NX,
            )
        } < 0
        {
            return Err(ENOMEM);
        }
    }
    Ok(sp)
}

/// Load `image` into a fresh address space for `p` and reseed `tf`.
///
/// # Safety
/// `p` is owned or current; when current, CR3 must not be `p`'s new pml4.
pub unsafe fn exec_elf(
    p: *mut Proc,
    tf: &mut Trapframe,
    image: &[u8],
    argv: &[&[u8]],
) -> Result<(), i64> {
    let (entry, phoff, phnum) = parse_ehdr(image)?;
    const MAX_SEGS: usize = 16;
    if phnum > MAX_SEGS {
        return Err(ENOEXEC);
    }

    // Collect PT_LOAD segments in ascending vaddr order (no heap: fixed
    // arrays + insertion sort) so mm.sz grows monotonically.
    let mut segs = [[0usize; 5]; MAX_SEGS]; // [vaddr, offset, filesz, memsz, flags]
    let mut nsegs = 0usize;
    for i in 0..phnum {
        let ph = phdr_at(image, phoff, i);
        if ph.p_type != PT_LOAD {
            continue;
        }
        let rec = [
            ph.p_vaddr as usize,
            ph.p_offset as usize,
            ph.p_filesz as usize,
            ph.p_memsz as usize,
            ph.p_flags as usize,
        ];
        let mut j = nsegs;
        while j > 0 && segs[j - 1][0] > rec[0] {
            segs[j] = segs[j - 1];
            j -= 1;
        }
        segs[j] = rec;
        nsegs += 1;
    }

    let mut new_mm = match Mm::new() {
        Some(m) => m,
        None => return Err(ENOMEM),
    };
    // Map from the first segment's page (never VA 0: the NULL page stays
    // unmapped) and keep a running cursor so each alloc covers only the
    // newly spanned range (xv6 exec's growing sz).
    let mut cursor = if nsegs > 0 {
        pgtable::pgrounddown(segs[0][0])
    } else {
        0
    };
    let mut max_end = 0usize;
    for seg in &segs[..nsegs] {
        let (vaddr, from, filesz, memsz, flags) = (seg[0], seg[1], seg[2], seg[3], seg[4]);
        let end = pgtable::pgroundup(vaddr + memsz);
        if vaddr >= USER_STACK_TOP || end > USER_STACK_TOP - 0x1000_0000 {
            new_mm.free();
            return Err(ENOEXEC);
        }
        if from.saturating_add(filesz) > image.len() {
            new_mm.free();
            return Err(ENOEXEC);
        }
        if new_mm.alloc(cursor, end) == 0 {
            new_mm.free();
            return Err(ENOMEM);
        }
        cursor = end;
        // SAFETY: range just mapped in new_mm.pml4.
        unsafe { write_user(new_mm.pml4, vaddr, &image[from..from + filesz]) };
        // Data segments non-executable; code segments executable.
        unsafe { set_nx(new_mm.pml4, vaddr, memsz, (flags & PF_X as usize) == 0) };
        if end > max_end {
            max_end = end;
        }
    }
    new_mm.sz = max_end;

    let rsp = match unsafe { build_stack(new_mm.pml4, argv) } {
        Ok(sp) => sp,
        Err(e) => {
            new_mm.free();
            return Err(e);
        }
    };

    // Commit: swap the address space, then free the old one through the
    // direct map (safe even if it was the active CR3 a moment ago).
    let new_cr3 = new_mm.cr3_pa();
    // SAFETY: owned slot.
    let old = unsafe { core::mem::replace(&mut (*p).mm, Some(new_mm)) };
    let running = proc::myproc() == p;
    if running {
        io::set_cr3(new_cr3 as u64);
    }
    if let Some(mut old_mm) = old {
        old_mm.free();
    }

    proc::seed_user_tf(tf, entry as usize, rsp);
    if let Some(first) = argv.first() {
        let n = first.len().min(15);
        // SAFETY: name array is 16 bytes; copy bounded by n <= 15.
        unsafe {
            (*p).name = [0; 16];
            let dst = core::ptr::addr_of_mut!((*p).name) as *mut u8;
            core::ptr::copy_nonoverlapping(first.as_ptr(), dst, n);
        }
    }
    Ok(())
}
