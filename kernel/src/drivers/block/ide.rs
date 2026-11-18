//! PIO (non-DMA) IDE driver: port of ide.c (xv6-public) for the x86_64
//! Core. One raw disk on the primary controller, drive 0 — the mkfs
//! xv6 root attached by the runner at ide index 0 (plans/linux26/04
//! §5). Deviation from xv6, documented: xv6's boot medium was disk 0 and
//! the fs lived on disk 1; minios boots via GRUB/El Torito, so the fs
//! root IS drive 0 and there is no second disk to probe.

use crate::arch::x86_64::io::{cli, hlt, in8, insl, out8, outsl, sti};
use crate::fs::buffer::{Buf, B_DIRTY, B_VALID};
use crate::klib::sync::Spinlock;
use crate::proc::proc::myproc;
use crate::proc::sleep::{sleep, wakeup};
use core::sync::atomic::Ordering;

/// The one supported device number (QEMU ide index 0).
pub const DEV_ROOT: u32 = 0;

// Status/command bits (ide.c).
const IDE_BSY: u8 = 0x80;
const IDE_DRDY: u8 = 0x40;
const IDE_DF: u8 = 0x20;
const IDE_ERR: u8 = 0x01;
const IDE_CMD_READ: u8 = 0x20;
const IDE_CMD_WRITE: u8 = 0x30;

// Primary ATA bus PIO ports.
const P_DATA: u16 = 0x1f0;
const P_SECCOUNT: u16 = 0x1f2;
const P_LBA_LO: u16 = 0x1f3;
const P_LBA_MID: u16 = 0x1f4;
const P_LBA_HI: u16 = 0x1f5;
const P_DRVHD: u16 = 0x1f6;
const P_STATUS: u16 = 0x1f7;
const P_CTRL: u16 = 0x3f6;

static IDELOCK: Spinlock<()> = Spinlock::new("ide", ());
// SAFETY: queue head only touched under IDELOCK.
static mut IDEQUEUE: *mut Buf = core::ptr::null_mut();

/// Drive-ready wait (idewait). Bounded spin: xv6 loops forever, an
/// emulated stuck disk should fail the suite loudly, not hang it.
fn idewait(checkerr: bool) -> i32 {
    let mut r: u8;
    let mut spins: u32 = 0;
    loop {
        r = in8(P_STATUS);
        if r & (IDE_BSY | IDE_DRDY) == IDE_DRDY {
            break;
        }
        spins += 1;
        if spins > 10_000_000 {
            panic!("idewait: disk never ready");
        }
    }
    if checkerr && (r & (IDE_DF | IDE_ERR)) != 0 {
        return -1;
    }
    0
}

/// Enable the IRQ and confirm the fs drive answers (ideinit, with the
/// single-disk deviation above: probe drive 0, not drive 1).
pub fn ideinit() {
    crate::drivers::pic::unmask(
        (crate::kernel::traps::IRQ_IDE - crate::kernel::traps::IRQ_BASE) as u8,
    );
    out8(P_DRVHD, 0xe0 | (0 << 4)); // select drive 0
    let mut present = false;
    for _ in 0..1000 {
        if in8(P_STATUS) != 0 {
            present = true;
            break;
        }
    }
    if !present {
        panic!("ideinit: fs disk (drive 0) not present");
    }
    idewait(false);
}

/// Start the request for `b` (idestart). Caller holds IDELOCK.
///
/// # Safety
/// `b` valid and non-null; IDELOCK held.
unsafe fn idestart(b: *mut Buf) {
    unsafe {
        if b.is_null() {
            panic!("idestart");
        }
        idewait(false);
        out8(P_CTRL, 0); // interrupt on completion
        out8(P_SECCOUNT, 1);
        out8(P_LBA_LO, (*b).sector as u8);
        out8(P_LBA_MID, ((*b).sector >> 8) as u8);
        out8(P_LBA_HI, ((*b).sector >> 16) as u8);
        out8(
            P_DRVHD,
            (0xe0 | (((*b).dev & 1) << 4) | (((*b).sector >> 24) & 0x0f)) as u8,
        );
        if (*b).flags.load(Ordering::Relaxed) & B_DIRTY != 0 {
            out8(P_STATUS, IDE_CMD_WRITE);
            outsl(P_DATA, (*b).data.as_ptr() as *const u32, 512 / 4);
        } else {
            out8(P_STATUS, IDE_CMD_READ);
        }
    }
}

/// Interrupt handler (ideintr): finish the active request, wake the
/// waiter, start the next queued buffer.
pub fn ideintr() {
    crate::drivers::pic::eoi(crate::kernel::traps::IRQ_IDE as u8);
    // SAFETY: the queue and buffer flags are guarded by IDELOCK.
    unsafe {
        IDELOCK.raw_lock();
        let active = core::ptr::addr_of!(IDEQUEUE).read();
        if active.is_null() {
            IDELOCK.raw_release();
            return;
        }
        // Dequeue the active request.
        core::ptr::addr_of_mut!(IDEQUEUE).write((*active).qnext);

        // Read data if needed.
        if (*active).flags.load(Ordering::Relaxed) & B_DIRTY == 0 && idewait(true) >= 0 {
            insl(P_DATA, (*active).data.as_mut_ptr() as *mut u32, 512 / 4);
        }

        (*active).flags.fetch_or(B_VALID, Ordering::Relaxed);
        (*active).flags.fetch_and(!B_DIRTY, Ordering::Relaxed);
        wakeup(active as usize);

        let head = core::ptr::addr_of!(IDEQUEUE).read();
        if !head.is_null() {
            idestart(head);
        }
        IDELOCK.raw_release();
    }
}

/// Queue a busy buffer for I/O and wait for completion (iderw).
///
/// Deviation, documented: xv6 sleeps on the buffer under IDELOCK; the
/// kernel suite driver runs before any process exists, so in that
/// context (myproc() == 0) the wait polls with interrupts enabled
/// instead — bounded by iterations so a dead disk panics rather than
/// hanging the suite deadline.
///
/// # Safety
/// `b` must be B_BUSY; on return it is B_VALID (and clean).
pub unsafe fn bdev_strategy(b: *mut Buf) {
    unsafe {
        if (*b).flags.load(Ordering::Relaxed) & crate::fs::buffer::B_BUSY == 0 {
            panic!("iderw: buf not busy");
        }
        if (*b).flags.load(Ordering::Relaxed) & (B_VALID | B_DIRTY) == B_VALID {
            panic!("iderw: nothing to do");
        }
        if (*b).dev != DEV_ROOT {
            panic!("iderw: dev {} is not the root disk", (*b).dev);
        }

        IDELOCK.raw_lock();
        // Append to the queue tail.
        (*b).qnext = core::ptr::null_mut();
        let mut pp: *mut *mut Buf = core::ptr::addr_of_mut!(IDEQUEUE);
        while !(*pp).is_null() {
            pp = core::ptr::addr_of_mut!((**pp).qnext);
        }
        *pp = b;

        let head = core::ptr::addr_of!(IDEQUEUE).read();
        if head == b as *mut Buf {
            idestart(b);
        }

        if !myproc().is_null() {
            while (*b).flags.load(Ordering::Relaxed) & (B_VALID | B_DIRTY) != B_VALID {
                sleep(b as usize, &IDELOCK);
            }
            IDELOCK.raw_release();
        } else {
            // Boot context: no process to sleep in. Enable interrupts and
            // park; the IDE (and PIT) interrupts drive completion.
            IDELOCK.raw_release();
            let mut waited: u32 = 0;
            while (*b).flags.load(Ordering::Relaxed) & (B_VALID | B_DIRTY) != B_VALID {
                sti();
                hlt();
                cli();
                waited += 1;
                if waited > 5000 {
                    panic!("iderw: completion timeout (sector {})", (*b).sector);
                }
            }
        }
    }
}
