//! M2 kernel-mode suite: the physical-page foundation (bootmem, buddy,
//! direct map) plus SLUB and vmalloc/ioremap. Kernel-only (no root
//! disk): boot cmdline `m2-suite` -> CASE lines -> merged verdict,
//! exit 0x10 -> QEMU 33 (0x11 on any failure).

use super::ksymtab;
use crate::arch::x86_64::io;
use crate::console;
use crate::mm::{bootmem, directmap, fault, frame, mmap, pgtable, vm, vma, vmalloc};
use frame::{free_frames, kalloc, kfree, kmalloc, kmfree, p2v, v2p, PGSIZE};

/// Sampled allocation scratch (1024 pointers, 8 KiB bss).
static mut PAGES: [*mut u8; 1024] = [core::ptr::null_mut(); 1024];

pub fn run() -> ! {
    ksymtab::set_active_suite("m2");
    let checks: [(&str, fn() -> bool); 15] = [
        ("m2.bootmem.table", case_bootmem_table),
        ("m2.buddy.reserved", case_buddy_reserved),
        ("m2.buddy.conserve_merge", case_buddy_conserve_merge),
        ("m2.directmap.roundtrip", case_directmap_roundtrip),
        ("m2.directmap.cover", case_directmap_cover),
        ("m2.slub.kmalloc", case_slub_kmalloc),
        ("m2.vmalloc.map_perm", case_vmalloc_map_perm),
        ("m2.vmalloc.vfree_once", case_vmalloc_vfree_once),
        ("m2.ioremap.skeleton", case_ioremap_skeleton),
        ("m2.vma.layout", case_vma_layout),
        ("m2.mmap.eager", case_mmap_eager),
        ("m2.mprotect.perm", case_mprotect_perm),
        ("m2.fault.stack", case_fault_stack),
        ("m2.fault.lazy", case_fault_lazy),
        ("m2.cow.pagecount", case_cow_pagecount),
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
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m2 run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m2 run=core-up total=");
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

/// Usable table sanity: non-empty, page-aligned, sorted/disjoint, all
/// inside [kernel end, PHYS_TOP), plus a sane probed top and volume.
fn case_bootmem_table() -> bool {
    let usable = bootmem::usable();
    let mut ok = usable.count >= 1;
    let floor = frame::kernel_phys_end();
    let mut prev_end = 0usize;
    for r in usable.iter() {
        let b = r.base as usize;
        let e = (r.base + r.len) as usize;
        ok = ok
            && b >= floor
            && e <= crate::arch::x86_64::PHYS_TOP
            && b % PGSIZE == 0
            && e % PGSIZE == 0
            && b >= prev_end; // sorted + disjoint (merged)
        prev_end = prev_end.max(e);
    }
    ok = ok && bootmem::usable().total_len(bootmem::Kind::Usable) >= (128 << 20) as u64;
    ok = ok && bootmem::probed_top_pa() >= prev_end;
    ok = ok && bootmem::raw().count >= usable.count;
    let z = frame::zone_summary();
    console::write_str("zones ");
    for (name, free) in z {
        console::write_str(name);
        console::write_str("=");
        console::write_hex(free);
        console::write_str(" ");
    }
    console::write_str("\n");
    ok
}

/// Reserved pages never enter the buddy: full mem_map invariant (a pfn
/// is inside the buddy iff its page lies in a usable region), plus every
/// sampled allocation stays above the kernel and inside usable RAM.
fn case_buddy_reserved() -> bool {
    let mut ok = frame::verify_seed();
    // SAFETY: boot-context scratch, single CPU.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(PAGES) };
    let floor = frame::kernel_phys_end();
    for slot in buf.iter_mut() {
        let p = kalloc();
        if p.is_null() {
            ok = false;
            break;
        }
        ok = ok && v2p(p as usize) >= floor && bootmem::in_usable(v2p(p as usize), PGSIZE);
        *slot = p;
    }
    for &p in buf.iter() {
        if !p.is_null() {
            kfree(p);
        }
    }
    ok
}

/// Conservation + merge: 1024 pages out and back (interleaved free
/// order) restore the free count and the largest free order exactly.
fn case_buddy_conserve_merge() -> bool {
    let base = free_frames();
    let largest0 = frame::largest_free_normal();
    // SAFETY: boot-context scratch, single CPU.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(PAGES) };
    for slot in buf.iter_mut() {
        *slot = kalloc();
        if slot.is_null() {
            return false;
        }
    }
    let drained = base == free_frames() + buf.len();
    // Free even indexes first, then odd: forces non-LIFO buddy merging.
    for i in (0..buf.len()).step_by(2) {
        kfree(buf[i]);
    }
    for i in (1..buf.len()).step_by(2) {
        kfree(buf[i]);
    }
    drained
        && free_frames() == base
        && frame::largest_free_normal().is_some()
        && frame::largest_free_normal() == largest0
}

/// Checked p2v/v2p: round trips inside the map, rejection at/beyond the
/// top, outside the window, and of non-canonical addresses.
fn case_directmap_roundtrip() -> bool {
    use directmap::{canonical, p2v_checked, v2p_checked};
    let top = directmap::dm_top();
    let mut ok = top >= (192 << 20); // -m 256M probes ~256 MiB
    for pa in [0usize, PGSIZE, 0x123_456, top - 1, top - PGSIZE] {
        ok = ok && p2v_checked(pa).is_some() && v2p_checked(p2v(pa)).map_or(false, |x| x == pa);
    }
    // Out of the map, out of the window, non-canonical: all rejected.
    ok = ok
        && v2p_checked(top).is_none()
        && v2p_checked(top + PGSIZE).is_none()
        && v2p_checked(0).is_none()
        && v2p_checked(crate::arch::x86_64::KERNEL_VMA).is_none()
        && v2p_checked(0x0000_5555_0000_0000).is_none()
        && v2p_checked(0x0000_8000_0000_0000).is_none()
        && v2p_checked(0xffff_8000_0000_0000).is_none();
    ok = ok && canonical(directmap::p2v_checked(0).unwrap());
    ok = ok && !canonical(0x0000_8000_0000_0000);
    ok = ok && canonical(0xffff_ffff_ffff_f000);
    ok
}

/// The direct map really spans every probed byte: each 2 MiB step has a
/// present 2 MiB supervisor RW NX PDE and the page itself is readable.
fn case_directmap_cover() -> bool {
    let top = directmap::dm_top();
    let mut pa = 0usize;
    let mut ok = true;
    while pa < top {
        let va = p2v(pa);
        // SAFETY: walk only touches kernel tables; the byte read lies in
        // mapped (probed) RAM.
        let pde = unsafe { directmap::dm_pde(va) };
        ok = ok
            && pde & pgtable::PTE_P != 0
            && pde & (1 << 7) != 0 // PS: 2 MiB page
            && pde & pgtable::PTE_W != 0
            && pde & pgtable::PTE_NX != 0
            && pde & pgtable::PTE_U == 0
            && (pde & 0x000f_ffff_ffe0_0000) as usize == pa;
        // SAFETY: probed RAM is mapped; volatile byte read only.
        let byte = unsafe { core::ptr::read_volatile(p2v(pa) as *const u8) };
        let _ = byte;
        pa += 2 << 20;
    }
    ok
}

/// kmalloc classes: write/read each size, LIFO reuse after kmfree, the
/// poison canary lands on freed bodies, and double/wrong frees fail.
fn case_slub_kmalloc() -> bool {
    let mut ok = true;
    for size in [8usize, 100, 512, 2048] {
        let p = kmalloc(size);
        ok = ok && !p.is_null();
        // SAFETY: kmalloc contract: size writable bytes.
        unsafe {
            *p = size as u8;
            *p.add(size - 1) = (size ^ 0xff) as u8;
            ok = ok && *p == size as u8 && *p.add(size - 1) == (size ^ 0xff) as u8;
        }
    }
    // Reuse + canary on one class.
    let p = kmalloc(500);
    ok = ok && !p.is_null();
    ok = ok && kmfree(p);
    // SAFETY: freed slab object; poison byte read for the canary check.
    let canary = unsafe { core::ptr::read_volatile(p.add(64)) };
    ok = ok && canary == crate::mm::slub::POISON;
    let q = kmalloc(500);
    ok = ok && q == p; // LIFO reuse
    ok = ok && kmfree(q);
    ok = ok && !kmfree(q); // double free rejected
    let page = kalloc();
    ok = ok && !page.is_null() && !kmfree(page); // not a slab object
    kfree(page);
    ok = ok && kmalloc(0).is_null();
    ok = ok && kmalloc(5000).is_null(); // above classes and page path
    let big = kmalloc(4096); // whole-page path
    ok = ok && !big.is_null();
    kfree(big);
    ok
}

/// vmalloc RW/RO mappings: data round trip through the VA window and the
/// promised PTE bits (RW: P|W|NX; RO: P|NX, W clear; never U).
fn case_vmalloc_map_perm() -> bool {
    let va = vmalloc::vmalloc(4, vmalloc::VM_RW);
    let mut ok = va >= vmalloc::VMALLOC_START && va < vmalloc::VMALLOC_END;
    for i in 0..4usize {
        // SAFETY: fresh vmalloc page; write+read one u64.
        let p = (va + i * PGSIZE) as *mut u64;
        unsafe {
            core::ptr::write_volatile(p, i as u64 + 0xfeed_0000);
            ok = ok && core::ptr::read_volatile(p) == i as u64 + 0xfeed_0000;
        }
        // SAFETY: kernel-table walk only.
        let pte = unsafe { *pgtable::walk(pgtable::boot_pml4(), va + i * PGSIZE, false) };
        ok = ok
            && pte & pgtable::PTE_P != 0
            && pte & pgtable::PTE_W != 0
            && pte & pgtable::PTE_NX != 0
            && pte & pgtable::PTE_U == 0;
    }
    let ro = vmalloc::vmalloc(2, vmalloc::VM_RO);
    ok = ok && ro != 0;
    // SAFETY: kernel-table walk only.
    let pte = unsafe { *pgtable::walk(pgtable::boot_pml4(), ro, false) };
    ok = ok && pte & pgtable::PTE_P != 0 && pte & pgtable::PTE_W == 0 && pte & pgtable::PTE_NX != 0;
    ok = ok && vmalloc::vfree(va);
    ok = ok && vmalloc::vfree(ro);
    ok
}

/// vfree invalidates the mapping and returns owned frames exactly once:
/// the frame count moves back by the owned page count (PT pages are
/// permanent), the PTE is clear, a second vfree is a no-op, and vmap'd
/// (not owned) pages are never freed by vfree.
fn case_vmalloc_vfree_once() -> bool {
    let base = free_frames();
    let va = vmalloc::vmalloc(8, vmalloc::VM_RW);
    // The window's page tables are permanent (installed by the earlier
    // map_perm case), so exactly the 8 owned frames leave the buddy.
    let mut ok = va != 0 && base - free_frames() == 8;
    ok = ok && vmalloc::vfree(va);
    // All 8 owned frames returned exactly once, nothing else moved
    // (permanent table pages stay allocated by design).
    ok = ok && free_frames() == base;
    // Mapping invalidated.
    // SAFETY: kernel-table walk only.
    let pte = unsafe { *pgtable::walk(pgtable::boot_pml4(), va, false) };
    ok = ok && pte & pgtable::PTE_P == 0;
    // Double vfree: rejected, frames untouched.
    let dv = vmalloc::vfree(va);
    ok = ok && !dv;
    ok = ok && free_frames() == base;
    // vmap of caller-owned pages: vfree never frees them.
    let mut pas = [0usize; 2];
    for pa in pas.iter_mut() {
        let p = kalloc();
        ok = ok && !p.is_null();
        *pa = v2p(p as usize);
    }
    let after = free_frames(); // both caller pages out of the buddy
    let vva = vmalloc::vmap(&pas, vmalloc::VM_RW);
    ok = ok && vva != 0;
    let vfr = vmalloc::vfree(vva);
    ok = ok && vfr;
    ok = ok && free_frames() == after; // the 2 caller pages are still ours
    for &pa in pas.iter() {
        // SAFETY: our own kalloc'd frames.
        kfree(p2v(pa) as *mut u8);
    }
    ok = ok && free_frames() == after + 2;
    ok
}

/// ioremap skeleton: MMIO frames map RW NX into the window with exact
/// PA bits, iounmap clears them, RAM requests are refused, and no buddy
/// frames move.
fn case_ioremap_skeleton() -> bool {
    let base = free_frames();
    let va = vmalloc::ioremap(0xfee0_0000, 0x1000);
    let mut ok = va >= vmalloc::VMALLOC_START && va < vmalloc::VMALLOC_END;
    // SAFETY: kernel-table walk only.
    let pte = unsafe { *pgtable::walk(pgtable::boot_pml4(), va, false) };
    ok = ok
        && pte & pgtable::PTE_P != 0
        && pte & pgtable::PTE_W != 0
        && pte & pgtable::PTE_NX != 0
        && (pte & pgtable::PTE_ADDR) as usize == 0xfee0_0000;
    ok = ok && vmalloc::iounmap(va);
    // SAFETY: kernel-table walk only.
    let pte = unsafe { *pgtable::walk(pgtable::boot_pml4(), va, false) };
    ok = ok && pte & pgtable::PTE_P == 0;
    ok = ok && vmalloc::ioremap(0x2000_000, 0x1000) == 0; // usable RAM refused
    ok = ok && free_frames() == base; // no frames owned by MMIO maps
    ok
}

// ---- m2r-s4: anonymous VMA / mmap / mprotect ---------------------------

/// Read a leaf PTE (0 when absent).
fn pte_of(pml4: pgtable::PtPtr, va: usize) -> u64 {
    // SAFETY: boot-context scratch mm; walk allocates nothing.
    let pte = unsafe { pgtable::walk(pml4, va, false) };
    if pte.is_null() {
        return 0;
    }
    // SAFETY: valid table page.
    unsafe { *pte }
}

/// exec-style VMA layout + brk cycle: segment VMAs carry their ELF
/// permissions into the PTEs (text W-off, data NX), the heap VMA tracks
/// a byte-exact break through grow/shrink/regrow, and the address space
/// frees without leaking frames.
fn case_vma_layout() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let text = 0x1_0000usize;
    let data = 0x1_1000usize;
    // Two segments: RX text, RW data (exec's insert order).
    let ok = mm.alloc(text, data, mmap::prot_to_pte(vma::PROT_R | vma::PROT_X)) == data
        && mm.vmas.insert(vma::Vma {
            start: text,
            end: data,
            prot: vma::PROT_R | vma::PROT_X,
            flags: 0,
        })
        && mm.alloc(data, data + PGSIZE, mmap::prot_to_pte(vma::PROT_RW)) == data + PGSIZE
        && mm.vmas.insert(vma::Vma {
            start: data,
            end: data + PGSIZE,
            prot: vma::PROT_RW,
            flags: 0,
        });
    if !ok {
        mm.free();
        return false;
    }
    let mut ok = true;
    // Text page: present+user, NOT writable, NOT NX (executable).
    let tpte = pte_of(mm.pml4, text);
    ok = ok
        && tpte & pgtable::PTE_P != 0
        && tpte & pgtable::PTE_U != 0
        && tpte & pgtable::PTE_W == 0
        && tpte & pgtable::PTE_NX == 0;
    // Data page: writable and NX.
    let dpte = pte_of(mm.pml4, data);
    ok = ok
        && dpte & pgtable::PTE_P != 0
        && dpte & pgtable::PTE_W != 0
        && dpte & pgtable::PTE_NX != 0;
    // Segments with different attrs never merge.
    ok = ok && mm.vmas.count == 2;
    // brk: page-aligned start, unaligned grow, exact shrink, regrow.
    mm.brk = data + PGSIZE;
    ok = ok && mm.set_brk(data + PGSIZE + 0x123) == data + PGSIZE + 0x123;
    ok = ok && mm.brk == data + PGSIZE + 0x123;
    ok = ok && mm.set_brk(data + PGSIZE) == data + PGSIZE;
    // Heap VMA vanishes on full shrink; regrow re-creates it byte-exact.
    ok = ok && mm.set_brk(data + 2 * PGSIZE + 0x45) == data + 2 * PGSIZE + 0x45;
    ok = ok && mm.vmas.count == 3;
    let heap = mm.vmas.vmas[2];
    ok = ok
        && heap.flags & vma::VMA_HEAP != 0
        && heap.start == data + PGSIZE
        && heap.end == data + 2 * PGSIZE + 0x45;
    // Sorted, non-overlapping table.
    ok = ok
        && mm.vmas.vmas[..mm.vmas.count]
            .windows(2)
            .all(|w| w[0].end <= w[1].start);
    let _fb = free_frames();
    if _fb != base {
        console::write_hex(_fb);
        console::write_str(" vs ");
        console::write_hex(base);
        console::write_str("\n");
    }
    mm.free();
    let _fa = free_frames();
    ok && _fa == base
}

/// Eager anonymous mmap: top-down disjoint placement, zero-filled RW|NX
/// pages, PROT_NONE reserves without mapping, munmap frees frames, and
/// non-anonymous ranges refuse to unmap.
fn case_mmap_eager() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, 3 * PGSIZE, vma::PROT_RW, false);
    let b = mm.mmap_anon(0, PGSIZE, vma::PROT_RW, false);
    let mut ok = a != 0 && b != 0 && b + PGSIZE <= a && a >= mmap::MMAP_FLOOR;
    // Second mapping packs directly below the first.
    ok = ok && b + PGSIZE == a;
    // Pages present, RW, NX, and zeroed.
    for off in 0..3 * PGSIZE {
        let e = pte_of(mm.pml4, a + off);
        ok = ok && e & pgtable::PTE_P != 0 && e & pgtable::PTE_W != 0 && e & pgtable::PTE_NX != 0;
        if e & pgtable::PTE_P != 0 {
            // SAFETY: direct-map word inside the mapped frame.
            let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
            ok = ok && w == 0;
        }
    }
    // Table anatomy under top-down placement: 1 pml4 + 1 pdpt + 1 pd +
    // 1 pt (the four pages share one 2 MiB span) plus 4 data frames.
    let frames_mapped = base - free_frames();
    ok = ok && frames_mapped == 8;
    // PROT_NONE reserves address space only.
    let c = mm.mmap_anon(0, PGSIZE, vma::PROT_NONE, false);
    ok = ok && c != 0 && c + PGSIZE == b && pte_of(mm.pml4, c) & pgtable::PTE_P == 0;
    ok = ok && free_frames() == base - 8;
    // munmap frees exactly the mapping's frames; a PROT_NONE range
    // unmaps fine (no frames to free).
    ok = ok && mm.munmap(a, 3 * PGSIZE);
    ok = ok && pte_of(mm.pml4, a) & pgtable::PTE_P == 0;
    // b stays mapped: 4 table pages + b's data page.
    ok = ok && free_frames() == base - 5;
    ok = ok && mm.munmap(c, PGSIZE);
    // Non-anonymous range (nothing there) refuses.
    ok = ok && !mm.munmap(text_floor(), PGSIZE);
    mm.free();
    ok && free_frames() == base
}

/// A canonical low address that is definitely not anonymous mmap memory.
fn text_floor() -> usize {
    0x2_0000
}

/// mprotect permission bits: RW -> R strips PTE_W; PROT_NONE strips
/// PTE_U; restoring RW keeps the data; holes are rejected and the VMA
/// table re-merges to one piece.
fn case_mprotect_perm() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, 2 * PGSIZE, vma::PROT_RW, false);
    if a == 0 {
        mm.free();
        return false;
    }
    // Stamp a marker through the direct map.
    let e = pte_of(mm.pml4, a + PGSIZE);
    // SAFETY: direct-map write into the mapped frame.
    unsafe {
        *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *mut u64) = 0xfeed_beef_cafe_1234;
    }
    let mut ok = true;
    // Read-only over page 2 only: table splits, PTE loses W.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_R);
    let e2 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok
        && e2 & pgtable::PTE_P != 0
        && e2 & pgtable::PTE_U != 0
        && e2 & pgtable::PTE_W == 0
        && e2 & pgtable::PTE_NX != 0;
    ok = ok && mm.vmas.count == 2;
    // Exec permission clears NX.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_R | vma::PROT_X);
    ok = ok && pte_of(mm.pml4, a + PGSIZE) & pgtable::PTE_NX == 0;
    // PROT_NONE keeps the page resident but user-inaccessible.
    ok = ok && mm.mprotect(a + PGSIZE, PGSIZE, vma::PROT_NONE);
    let e3 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok && e3 & pgtable::PTE_P != 0 && e3 & pgtable::PTE_U == 0;
    ok = ok && free_frames() == base - 6; // 4 table pages + 2 data, none dropped
                                          // Back to RW: data survives, table re-merges.
    ok = ok && mm.mprotect(a, 2 * PGSIZE, vma::PROT_RW);
    ok = ok && mm.vmas.count == 1;
    let e4 = pte_of(mm.pml4, a + PGSIZE);
    ok = ok && e4 & pgtable::PTE_W != 0 && e4 & pgtable::PTE_U != 0;
    // SAFETY: direct-map read of the same frame.
    let v = unsafe { *(frame::p2v((e4 & pgtable::PTE_ADDR) as usize) as *const u64) };
    ok = ok && v == 0xfeed_beef_cafe_1234;
    // Holes and bad prot are refused untouched.
    ok = ok && !mm.mprotect(a + 4 * PGSIZE, PGSIZE, vma::PROT_R);
    ok = ok && !mm.mprotect(a, PGSIZE, vma::PROT_R | 0x8);
    ok = ok && mm.vmas.count == 1;
    mm.free();
    ok && free_frames() == base
}

/// Stack VMA growth and guard (22 §4): a data fault below the stack VMA
/// but at/above the guard floor grows the VMA down with zeroed RW|NX
/// pages; below the floor, above the top, or an instruction fetch is
/// killed. Growth is repeatable and keeps the table sorted.
fn case_fault_stack() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let top = vm::USER_STACK_TOP;
    let s0 = top - vm::USER_STACK_PAGES * PGSIZE;
    if !mm.vmas.insert(vma::Vma {
        start: s0,
        end: top,
        prot: vma::PROT_RW,
        flags: vma::VMA_STACK,
    }) {
        mm.free();
        return false;
    }
    // One page below the stack: growth with the faulting page zeroed.
    let act = fault::classify(&mm, s0 - PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W);
    let mut ok =
        matches!(act, fault::FaultAction::GrowStack { new_start } if new_start == s0 - PGSIZE);
    ok = ok
        && fault::apply(
            &mut mm,
            &fault::FaultAction::GrowStack {
                new_start: s0 - PGSIZE,
            },
        );
    ok = ok
        && mm.vmas.count == 1
        && mm.vmas.vmas[0].start == s0 - PGSIZE
        && mm.vmas.vmas[0].end == top
        && mm.vmas.vmas[0].flags & vma::VMA_STACK != 0;
    let e = pte_of(mm.pml4, s0 - PGSIZE);
    ok = ok && e & pgtable::PTE_P != 0 && e & pgtable::PTE_W != 0 && e & pgtable::PTE_NX != 0;
    if e & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
        ok = ok && w == 0;
    }
    // Growth is repeatable (deep recursion pattern) and eager below the
    // new start only.
    let act2 = fault::classify(&mm, s0 - 3 * PGSIZE, fault::PF_ERR_U);
    ok = ok
        && matches!(act2, fault::FaultAction::GrowStack { new_start } if new_start == s0 - 3 * PGSIZE);
    ok = ok
        && fault::apply(
            &mut mm,
            &fault::FaultAction::GrowStack {
                new_start: s0 - 3 * PGSIZE,
            },
        );
    ok = ok && mm.vmas.count == 1 && mm.vmas.vmas[0].start == s0 - 3 * PGSIZE;
    ok = ok && pte_of(mm.pml4, s0 - 2 * PGSIZE) & pgtable::PTE_P != 0;
    // Guard band: below the floor is killed, instruction fetch never grows.
    ok = ok
        && matches!(
            fault::classify(
                &mm,
                fault::STACK_GUARD_FLOOR - PGSIZE,
                fault::PF_ERR_U | fault::PF_ERR_W
            ),
            fault::FaultAction::Kill
        );
    ok = ok
        && matches!(
            fault::classify(&mm, s0 - 4 * PGSIZE, fault::PF_ERR_U | fault::PF_ERR_I),
            fault::FaultAction::Kill
        );
    // Above the stack top: no VMA, killed.
    ok = ok
        && matches!(
            fault::classify(&mm, top + PGSIZE, fault::PF_ERR_U),
            fault::FaultAction::Kill
        );
    mm.free();
    ok && free_frames() == base
}

/// Lazy anonymous zero-fill and kill classification (22 §4): pages of a
/// PROT_NONE reservation re-permissioned to RW fault in one page at a
/// time; write-to-readonly, NX fetch, non-canonical and kernel-source
/// faults are all killed.
fn case_fault_lazy() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    // Reserve 2 pages, then make them RW: the PTEs stay absent (lazy).
    let c = mm.mmap_anon(0, 2 * PGSIZE, vma::PROT_NONE, false);
    if c == 0 || !mm.mprotect(c, 2 * PGSIZE, vma::PROT_RW) {
        mm.free();
        return false;
    }
    let mut ok = pte_of(mm.pml4, c + PGSIZE) & pgtable::PTE_P == 0;
    let act = fault::classify(&mm, c + PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W);
    ok = ok
        && matches!(act, fault::FaultAction::Fill { va, prot } if va == c + PGSIZE && prot == vma::PROT_RW);
    ok = ok && fault::apply(&mut mm, &act);
    let e = pte_of(mm.pml4, c + PGSIZE);
    ok = ok
        && e & pgtable::PTE_P != 0
        && e & pgtable::PTE_U != 0
        && e & pgtable::PTE_W != 0
        && e & pgtable::PTE_NX != 0;
    if e & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v((e & pgtable::PTE_ADDR) as usize) as *const u64) };
        ok = ok && w == 0;
    }
    // Demand paging: the neighbour is still absent, and its fill drops
    // exactly one frame (the region's page tables are already warm).
    ok = ok && pte_of(mm.pml4, c) & pgtable::PTE_P == 0;
    let before = free_frames();
    let act0 = fault::classify(&mm, c, fault::PF_ERR_U);
    ok = ok && matches!(act0, fault::FaultAction::Fill { va, .. } if va == c);
    ok = ok && fault::apply(&mut mm, &act0);
    ok = ok && before - free_frames() == 1;
    // Second fault on the now-present page (stale P): killed.
    ok = ok
        && matches!(
            fault::classify(
                &mm,
                c + PGSIZE,
                fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P
            ),
            fault::FaultAction::Kill
        );
    // Re-protect page 2 read-only: a write fault is killed by VMA prot.
    ok = ok && mm.mprotect(c + PGSIZE, PGSIZE, vma::PROT_R);
    ok = ok
        && matches!(
            fault::classify(&mm, c + PGSIZE, fault::PF_ERR_U | fault::PF_ERR_W),
            fault::FaultAction::Kill
        );
    // Instruction fetch on a non-exec VMA: killed.
    ok = ok
        && matches!(
            fault::classify(&mm, c, fault::PF_ERR_U | fault::PF_ERR_I),
            fault::FaultAction::Kill
        );
    // Non-canonical address and kernel-source fault: killed.
    ok = ok
        && matches!(
            fault::classify(&mm, 0x0000_8000_0000_0000, fault::PF_ERR_U),
            fault::FaultAction::Kill
        );
    ok = ok && matches!(fault::classify(&mm, c, 0), fault::FaultAction::Kill);
    // PROT_NONE access is killed regardless of error code bits.
    let d = mm.mmap_anon(0, PGSIZE, vma::PROT_NONE, false);
    ok = ok
        && d != 0
        && matches!(
            fault::classify(&mm, d, fault::PF_ERR_U | fault::PF_ERR_W),
            fault::FaultAction::Kill
        );
    mm.free();
    ok && free_frames() == base
}

/// COW fork page accounting (22 §4): fork shares every writable frame
/// read-only on both sides (refcount 2), a child write fault breaks into
/// a private copy (old frame back to 1, content preserved), and the sole
/// remaining owner re-owns in place without a copy. Teardown returns
/// every frame exactly once.
fn case_cow_pagecount() -> bool {
    let base = free_frames();
    let mut mm = match vm::Mm::new() {
        Some(m) => m,
        None => return false,
    };
    let a = mm.mmap_anon(0, PGSIZE, vma::PROT_RW, false);
    if a == 0 {
        mm.free();
        return false;
    }
    let e = pte_of(mm.pml4, a);
    let pa1 = e & pgtable::PTE_ADDR;
    // SAFETY: direct-map write into the mapped frame.
    unsafe {
        *(frame::p2v(pa1 as usize) as *mut u64) = 0x1234_5678_9abc_def0;
    }
    // fork: no data frame is copied — the child maps the same physical
    // page, write is broken on both sides.
    let before = free_frames();
    // SAFETY: scratch mm with present user mappings.
    let child = unsafe { pgtable::copy_user_half(mm.pml4) };
    // Tables only: the child needs pml4+pdpt+pd+pt for the region and
    // copies no data frame.
    let mut ok = !child.is_null() && before - free_frames() == 4;
    let ce = pte_of(child, a);
    ok = ok
        && ce & pgtable::PTE_P != 0
        && ce & pgtable::PTE_ADDR == pa1
        && ce & pgtable::PTE_W == 0
        && ce & pgtable::PTE_NX != 0;
    ok = ok && pte_of(mm.pml4, a) & pgtable::PTE_W == 0; // parent broken too
    ok = ok && frame::page_ref(pa1 as usize) == 2;
    // Child write fault: classify as COW, apply to a private copy.
    let child_mm = vm::Mm {
        pml4: child,
        vmas: mm.vmas,
        brk: mm.brk,
    };
    let mut child_mm = child_mm;
    let act = fault::classify(
        &child_mm,
        a,
        fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P,
    );
    ok = ok && matches!(act, fault::FaultAction::Cow { va } if va == a);
    ok = ok && fault::apply(&mut child_mm, &act);
    let ce2 = pte_of(child, a);
    let pa2 = ce2 & pgtable::PTE_ADDR;
    ok = ok
        && pa2 != pa1
        && ce2 & pgtable::PTE_W != 0
        && ce2 & pgtable::PTE_U != 0
        && ce2 & pgtable::PTE_NX != 0
        && frame::page_ref(pa1 as usize) == 1
        && frame::page_ref(pa2 as usize) == 1;
    // The copy carried the stamped marker.
    if ce2 & pgtable::PTE_P != 0 {
        // SAFETY: direct-map word inside the mapped frame.
        let w = unsafe { *(frame::p2v(pa2 as usize) as *const u64) };
        ok = ok && w == 0x1234_5678_9abc_def0;
    }
    // Parent write fault on the now-solely-owned frame: re-own in place,
    // no copy, same physical page.
    let act2 = fault::classify(&mm, a, fault::PF_ERR_U | fault::PF_ERR_W | fault::PF_ERR_P);
    ok = ok && matches!(act2, fault::FaultAction::Cow { va } if va == a);
    ok = ok && fault::apply(&mut mm, &act2);
    ok = ok
        && pte_of(mm.pml4, a) & pgtable::PTE_W != 0
        && pte_of(mm.pml4, a) & pgtable::PTE_ADDR == pa1;
    // Teardown: both spaces release every frame exactly once.
    // SAFETY: child tables no longer referenced.
    unsafe { pgtable::free_user_pml4(child) };
    mm.free();
    ok && free_frames() == base
}
