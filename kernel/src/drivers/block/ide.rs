//! Simple PIO-based (non-DMA) IDE driver: port of `ide.c` (xv6-public).
//!
//! Landed integrations used here:
//! * `crate::kernel::sched::{sleep, wakeup, RawLock}` — proc.c-style
//!   sleep/wakeup; idelock is a RawLock and waiters sleep on the buf
//!   pointer exactly like ide.c.
//! * `crate::arch::x86::kernel::pic::picirq_enable(irq: u8)` — xv6
//!   picenable(IRQ_IDE); xv6 additionally calls ioapicenable(IRQ_IDE,
//!   ncpu-1), omitted because minios is PIC-only.

use crate::arch::x86::kernel::IRQ_IDE;
use crate::fs::buffer::{Buf, B_BUSY, B_DIRTY, B_VALID};
use crate::fs::xv6fs::layout::{has_flag, BSIZE};
use crate::io::{inb, insl, outb, outsl};
use crate::kernel::sched::{acquire, release, sleep, wakeup, RawLock};

// ide.c status/command bits.
const IDE_BSY: i32 = 0x80;
const IDE_DRDY: i32 = 0x40;
const IDE_DF: i32 = 0x20;
const IDE_ERR: i32 = 0x01;

const IDE_CMD_READ: u8 = 0x20;
const IDE_CMD_WRITE: u8 = 0x30;

// Primary ATA bus PIO ports (0x1f0..0x1f7) + control port 0x3f6.
const IDE_PORT_DATA: u16 = 0x1f0;
const IDE_PORT_SECCOUNT: u16 = 0x1f2;
const IDE_PORT_LBA_LO: u16 = 0x1f3;
const IDE_PORT_LBA_MID: u16 = 0x1f4;
const IDE_PORT_LBA_HI: u16 = 0x1f5;
const IDE_PORT_DRVHD: u16 = 0x1f6; // drive/cylinder-head select register
const IDE_PORT_STATUS: u16 = 0x1f7;
const IDE_PORT_CTRL: u16 = 0x3f6;

// idequeue points to the buf now being read/written to the disk.
// idequeue->qnext points to the next buf to be processed.
// You must hold idelock while manipulating queue.
static IDELOCK: RawLock = RawLock::new("ide");

// SAFETY: both statics are only touched while holding IDELOCK (and once at
// boot from ideinit before interrupts are enabled for the device).
static mut IDEQUEUE: *mut Buf = core::ptr::null_mut();
static mut HAVEDISK1: i32 = 0;

// ---- queue helpers (idelock held) ------------------------------------------

/// Read the queue head. Caller must hold IDELOCK.
unsafe fn idequeue() -> *mut Buf {
    // SAFETY: protected by IDELOCK.
    unsafe { core::ptr::addr_of!(IDEQUEUE).read() }
}

/// Set the queue head. Caller must hold IDELOCK.
unsafe fn set_idequeue(b: *mut Buf) {
    // SAFETY: protected by IDELOCK.
    unsafe { core::ptr::addr_of_mut!(IDEQUEUE).write(b) };
}

/// Wait for IDE disk to become ready. (ide.c idewait)
///
/// xv6 BUG, kept faithful: there is no timeout — if the disk never comes
/// ready this spins forever ("wait for disk").
fn idewait(checkerr: bool) -> i32 {
    let mut r: i32;
    loop {
        r = inb(IDE_PORT_STATUS) as i32;
        if r & (IDE_BSY | IDE_DRDY) == IDE_DRDY {
            break;
        }
    }
    if checkerr && (r & (IDE_DF | IDE_ERR)) != 0 {
        return -1;
    }
    0
}

/// ide.c ideinit(): enable the IRQ, probe for a second disk.
pub fn ideinit() {
    // xv6: initlock(&idelock, "ide") — const-initialized here.
    crate::arch::x86::kernel::pic::picirq_enable(IRQ_IDE as u8);
    // xv6: ioapicenable(IRQ_IDE, ncpu-1); omitted (PIC-only minios).
    //
    // minios deviation: there is no first drive (xv6 booted from xv6.img on
    // hda; minios boots via multiboot -kernel), so select the fs drive
    // (drive 1, hdb) BEFORE waiting for DRDY — idewait polls whichever
    // drive DRVHD currently selects.
    outb(IDE_PORT_DRVHD, 0xe0 | (1 << 4));
    for _ in 0..1000 {
        if inb(IDE_PORT_STATUS) != 0 {
            // SAFETY: boot-time init, device IRQ not yet enabled.
            unsafe { core::ptr::addr_of_mut!(HAVEDISK1).write(1) };
            break;
        }
    }
    idewait(false);
}

/// Start the request for b. Caller must hold idelock. (ide.c idestart)
///
/// # Safety
/// `b` must be non-null and its `data` valid; caller must hold IDELOCK.
unsafe fn idestart(b: *mut Buf) {
    if b.is_null() {
        panic!("idestart");
    }

    idewait(false);
    outb(IDE_PORT_CTRL, 0); // generate interrupt
    outb(IDE_PORT_SECCOUNT, 1); // number of sectors
    outb(IDE_PORT_LBA_LO, ((*b).sector & 0xff) as u8);
    outb(IDE_PORT_LBA_MID, (((*b).sector >> 8) & 0xff) as u8);
    outb(IDE_PORT_LBA_HI, (((*b).sector >> 16) & 0xff) as u8);
    // Drive/cylinder-head select: 0xe0 (LBA mode) | drive | high LBA bits —
    // exactly the drv/head math in ide.c idestart().
    outb(
        IDE_PORT_DRVHD,
        (0xe0 | (((*b).dev & 1) << 4) | (((*b).sector >> 24) & 0x0f)) as u8,
    );
    if has_flag((*b).flags, B_DIRTY) {
        outb(IDE_PORT_STATUS, IDE_CMD_WRITE);
        // SAFETY: writes BSIZE/4 dwords from b->data to the data port.
        outsl(IDE_PORT_DATA, (*b).data.as_ptr() as *const u32, BSIZE / 4);
    } else {
        outb(IDE_PORT_STATUS, IDE_CMD_READ);
    }
}

/// Interrupt handler. (ide.c ideintr)
pub fn ideintr() {
    // First queued buffer is the active request.
    // SAFETY: queue manipulation and flag updates under IDELOCK, mirroring
    // ideintr()'s acquire/release of idelock.
    unsafe {
        acquire(&IDELOCK);

        let b = idequeue();
        if b.is_null() {
            // cprintf("spurious IDE interrupt\n");
            release(&IDELOCK);
            return;
        }
        set_idequeue((*b).qnext);

        // b is the active request; queue head already advanced above.
        if !has_flag((*b).flags, B_DIRTY) && idewait(true) >= 0 {
            insl(IDE_PORT_DATA, (*b).data.as_mut_ptr() as *mut u32, BSIZE / 4);
        }

        // Wake process waiting for this buf.
        (*b).flags |= B_VALID;
        (*b).flags &= !B_DIRTY;
        wakeup(b as usize);

        // Start disk on next buf in queue.
        if !idequeue().is_null() {
            idestart(idequeue());
        }

        release(&IDELOCK);
    }
}

/// Sync buf with disk. (ide.c iderw)
/// If B_DIRTY is set, write buf to disk, clear B_DIRTY, set B_VALID.
/// Else if B_VALID is not set, read buf from disk, set B_VALID.
///
/// # Safety
/// `b` must be B_BUSY (from bget/bread) and have something to do.
pub unsafe fn iderw(b: *mut Buf) {
    unsafe {
        if !has_flag((*b).flags, B_BUSY) {
            panic!("iderw: buf not busy");
        }
        if (*b).flags & (B_VALID | B_DIRTY) == B_VALID {
            panic!("iderw: nothing to do");
        }
        if (*b).dev != 0 && core::ptr::addr_of!(HAVEDISK1).read() == 0 {
            panic!("iderw: ide disk 1 not present");
        }

        acquire(&IDELOCK); // DOC: acquire-lock

        // Append b to idequeue, walking the raw-pointer qnext chain exactly
        // like ide.c's `for(pp=&idequeue; *pp; pp=&(*pp)->qnext)`.
        (*b).qnext = core::ptr::null_mut();
        let mut pp: *mut *mut Buf = core::ptr::addr_of_mut!(IDEQUEUE); // DOC: insert-queue
        while !(*pp).is_null() {
            pp = core::ptr::addr_of_mut!((*(*pp)).qnext);
        }
        *pp = b;

        // Start disk if necessary.
        if idequeue() == b as *mut Buf {
            idestart(b);
        }

        // Wait for request to finish. sleep() releases IDELOCK while
        // waiting and re-acquires it before returning (ide.c iderw).
        while (*b).flags & (B_VALID | B_DIRTY) != B_VALID {
            sleep(b as usize, &IDELOCK);
        }

        release(&IDELOCK);
    }
}
