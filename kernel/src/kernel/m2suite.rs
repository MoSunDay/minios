//! M2 kernel-mode suite: the physical-page foundation (bootmem, buddy,
//! direct map) plus SLUB and vmalloc/ioremap. Kernel-only (no root
//! disk): boot cmdline `m2-suite` -> CASE lines -> merged verdict,
//! exit 0x10 -> QEMU 33 (0x11 on any failure).

use super::ksymtab;
use super::{m2map, m2mem};
use crate::arch::x86_64::io;
use crate::console;
use crate::mm::{bootmem, directmap, frame, pgtable, vmalloc};
use frame::{free_frames, kalloc, kfree, kmalloc, kmfree, p2v, v2p, PGSIZE};

/// Sampled allocation scratch (1024 pointers, 8 KiB bss).
static mut PAGES: [*mut u8; 1024] = [core::ptr::null_mut(); 1024];

pub fn run() -> ! {
    ksymtab::set_active_suite("m2");
    let checks: [(&str, fn() -> bool); 16] = [
        ("m2.bootmem.table", case_bootmem_table),
        ("m2.buddy.reserved", case_buddy_reserved),
        ("m2.buddy.conserve_merge", case_buddy_conserve_merge),
        ("m2.directmap.roundtrip", case_directmap_roundtrip),
        ("m2.directmap.cover", case_directmap_cover),
        ("m2.slub.kmalloc", case_slub_kmalloc),
        ("m2.vmalloc.map_perm", case_vmalloc_map_perm),
        ("m2.vmalloc.vfree_once", case_vmalloc_vfree_once),
        ("m2.ioremap.skeleton", case_ioremap_skeleton),
        ("m2.vma.layout", m2map::case_vma_layout),
        ("m2.mmap.eager", m2map::case_mmap_eager),
        ("m2.mprotect.perm", m2map::case_mprotect_perm),
        ("m2.fault.stack", m2mem::case_fault_stack),
        ("m2.fault.lazy", m2mem::case_fault_lazy),
        ("m2.cow.pagecount", m2mem::case_cow_pagecount),
        ("m2.oom.select_stats", m2mem::case_oom_select_stats),
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
