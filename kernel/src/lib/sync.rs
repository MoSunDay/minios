//! Spinlock with xv6 interrupt-disable semantics (spinlock.c).
//! unsafe is confined to intr state tracking.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

pub struct Spinlock<T> {
    pub name: &'static str,
    locked: AtomicU32,
    #[allow(dead_code)] // read through SpinlockGuard::get
    data: UnsafeCell<T>,
}

pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<T> Spinlock<T> {
    pub const fn new(name: &'static str, data: T) -> Self {
        Spinlock {
            name,
            locked: AtomicU32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        self.push_off();
        // SAFETY: xv6 semantics: acquire with interrupts disabled.
        while self.locked.swap(1, Ordering::Acquire) == 1 {
            core::hint::spin_loop();
        }
        SpinlockGuard { lock: self }
    }

    /// Acquire without a guard: ownership may be released from another
    /// context after a scheduler swtch (xv6 forkret/sleep discipline).
    /// SAFETY: caller must balance with raw_release on the same lock.
    #[allow(dead_code)] // paired with raw_release; sleep(chan, other_lk) lands in S2
    pub unsafe fn raw_lock(&self) {
        self.push_off();
        while self.locked.swap(1, Ordering::Acquire) == 1 {
            core::hint::spin_loop();
        }
    }

    /// Release without a guard. SAFETY: the lock must be held (the current
    /// context logically owns it, possibly inherited through a swtch).
    pub unsafe fn raw_release(&self) {
        self.locked.store(0, Ordering::Release);
        self.pop_off();
    }

    fn push_off(&self) {
        pushcli();
    }

    fn pop_off(&self) {
        popcli();
    }
}

#[allow(dead_code)] // guard accessors for Spinlock<T> users (only T=() today)
impl<'a, T> SpinlockGuard<'a, T> {
    pub fn get(&self) -> &T {
        // SAFETY: guard owns exclusive access.
        unsafe { &*self.lock.data.get() }
    }
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: guard owns exclusive access.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        let held = self.lock.locked.load(Ordering::Acquire) == 1;
        if !held {
            panic!("drop of unlocked {}", self.lock.name);
        }
        self.lock.locked.store(0, Ordering::Release);
        self.lock.pop_off();
    }
}

/// Are interrupts currently enabled?
pub fn intr_state() -> u32 {
    let mut flags: u64;
    // SAFETY: raw pushfq/popq read (x86_64 RFLAGS).
    unsafe { core::arch::asm!("pushfq; popq %rax", out("rax") flags, options(att_syntax)) };
    ((flags >> 9) & 1) as u32
}

pub fn intr_disable() {
    // SAFETY: cli instruction.
    unsafe { core::arch::asm!("cli", options(nomem, preserves_flags)) };
}

pub fn intr_enable() {
    // SAFETY: sti instruction.
    unsafe { core::arch::asm!("sti", options(nomem, preserves_flags)) };
}

// xv6 spinlock.c pushcli/popcli: ONE per-CPU nesting counter shared by every
// lock flavor (RawLock, Spinlock) and by switchuvm. Tracking the counter per
// lock (as an earlier version did) lets an inner unlock re-enable interrupts
// while an outer lock is still held -- on a single CPU that turns a timer IRQ
// inside a critical section into a nested acquire of the same lock.
static NCLI: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static INTENA: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// xv6 pushcli(): disable interrupts, remember whether they were enabled at
/// the outermost nesting level.
pub fn pushcli() {
    let old = intr_state();
    intr_disable();
    // SAFETY: single CPU; SeqCst orders the disable before the count update.
    if NCLI.fetch_add(1, core::sync::atomic::Ordering::SeqCst) == 0 {
        INTENA.store(old, core::sync::atomic::Ordering::SeqCst);
    }
}

/// xv6 popcli(): drop one nesting level; re-enable only at the outermost
/// level and only if interrupts were on when the outermost pushcli ran.
pub fn popcli() {
    let n = NCLI.fetch_sub(1, core::sync::atomic::Ordering::SeqCst) - 1;
    if n == 0 && INTENA.load(core::sync::atomic::Ordering::SeqCst) != 0 {
        intr_enable();
    }
}

/// Current pushcli nesting depth (xv6 mycpu()->ncli).
#[allow(dead_code)] // xv6 spinlock.c accessor, useful for diagnostics
pub fn ncli() -> u32 {
    NCLI.load(core::sync::atomic::Ordering::SeqCst)
}

/// Interrupt-enable snapshot taken at the outermost pushcli.
#[allow(dead_code)] // xv6 spinlock.c accessor, useful for diagnostics
pub fn intena() -> u32 {
    INTENA.load(core::sync::atomic::Ordering::SeqCst)
}


// SAFETY: Spinlock hands out &T only while held (single CPU M1); raw
// interior mutability is mediated by the lock itself.
unsafe impl<T: Send> Sync for Spinlock<T> {}
