//! Device switch table: xv6 `file.h`'s `struct devsw devsw[NDEV]`, filled in
//! by console init (major 1 = CONSOLE) so readi/writei can dispatch T_DEV
//! inodes. xv6 keeps this as a plain unlocked array written once at boot.

use crate::fs::xv6fs::inode::Inode;
use crate::fs::xv6fs::layout::NDEV;

pub type DevReadFn = unsafe fn(ip: *mut Inode, dst: *mut u8, n: i32) -> i32;
pub type DevWriteFn = unsafe fn(ip: *mut Inode, src: *const u8, n: i32) -> i32;

#[derive(Clone, Copy)]
pub struct DevSwitch {
    pub read: Option<DevReadFn>,
    #[allow(dead_code)] // S4d: devsw_write()/writei() dispatch lands then
    pub write: Option<DevWriteFn>,
}

// SAFETY: written once at boot by register_dev() before user processes run,
// then only read — the same plain-array discipline as xv6's devsw[].
static mut DEVSW: [DevSwitch; NDEV] = [DevSwitch {
    read: None,
    write: None,
}; NDEV];

/// Register a device driver (console init writes major 1 = CONSOLE here).
///
/// # Safety
/// must only be called during boot, before any T_DEV inode can be read or
/// written (xv6 fills devsw[] in consoleinit without locking).
#[allow(dead_code)] // S4d: console init registers major 1 at boot
pub unsafe fn register_dev(major: i16, read: Option<DevReadFn>, write: Option<DevWriteFn>) {
    assert!((major as usize) < NDEV, "register_dev: bad major");
    DEVSW[major as usize] = DevSwitch { read, write };
}

/// devsw[major].read lookup for readi()'s T_DEV dispatch.
///
/// # Safety
/// caller must have bounds-checked `major` into 0..NDEV.
pub unsafe fn devsw_read(major: i16) -> Option<DevReadFn> {
    DEVSW[major as usize].read
}

/// devsw[major].write lookup for writei()'s T_DEV dispatch.
///
/// # Safety
/// caller must have bounds-checked `major` into 0..NDEV.
#[allow(dead_code)] // S4d: writei() dispatches T_DEV writes
pub unsafe fn devsw_write(major: i16) -> Option<DevWriteFn> {
    DEVSW[major as usize].write
}
