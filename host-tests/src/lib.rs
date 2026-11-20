//! Host-side unit tests for portable kernel code.
//!
//! The modules below are the real `kernel/src/lib` files, included by path so
//! there is no test-only copy to drift. `cfg(test)` is active for the whole
//! crate here, which also disables the kernel's `#[no_mangle]` string symbols
//! so they cannot collide with the host libc.

#[path = "../../kernel/src/lib/bitmap.rs"]
pub mod bitmap;
#[path = "../../kernel/src/mm/bootmem.rs"]
pub mod bootmem;
#[path = "../../kernel/src/mm/buddy.rs"]
pub mod buddy;
#[path = "../../kernel/src/proc/cfs.rs"]
pub mod cfs;
#[path = "../../kernel/src/lib/elf.rs"]
pub mod elf;
#[path = "../../kernel/src/kernel/fixup_table.rs"]
pub mod fixup_table;
#[path = "../../kernel/src/kernel/ksyms.rs"]
pub mod ksyms;
#[path = "../../kernel/src/proc/rbtree.rs"]
pub mod rbtree;
#[path = "../../kernel/src/proc/rt.rs"]
pub mod rt;
#[path = "../../kernel/src/mm/slub.rs"]
pub mod slub;
#[path = "../../kernel/src/lib/string.rs"]
pub mod string;
#[path = "../../kernel/src/kernel/sysarg.rs"]
pub mod sysarg;
#[path = "../../kernel/src/kernel/tf_layout.rs"]
pub mod tf_layout;
#[path = "../../kernel/src/mm/vma.rs"]
pub mod vma;
#[path = "../../kernel/src/proc/waitqueue.rs"]
pub mod waitqueue;

/// Host stand-in for the kernel's klib::sync::Spinlock: the kernel lock
/// discipline (interrupts-off acquire) is kernel-only; the included
/// kernel time modules just need mutual exclusion and a const `new`.
pub mod klib {
    pub mod sync {
        use std::sync::{Mutex, MutexGuard};

        pub struct Spinlock<T> {
            pub name: &'static str,
            data: Mutex<T>,
        }

        impl<T> Spinlock<T> {
            pub const fn new(name: &'static str, data: T) -> Self {
                Spinlock {
                    name,
                    data: Mutex::new(data),
                }
            }

            pub fn lock(&self) -> SpinlockGuard<'_, T> {
                SpinlockGuard(self.data.lock().unwrap())
            }
        }

        pub struct SpinlockGuard<'a, T>(
            /// Held for RAII drop (the release path); never read.
            #[allow(dead_code)]
            MutexGuard<'a, T>,
        );
    }
}

/// Host stand-in for the kernel's arch::x86_64::io: the included
/// clocksource needs `sti`/`cli` to type-check around calibrate_tsc's
/// busy wait — on the host that path is never called (no PIT, no
/// interrupt gate), so the pair records the depth without touching
/// real flags.
pub mod arch {
    pub mod x86_64 {
        pub mod io {
            use core::sync::atomic::{AtomicUsize, Ordering};

            static DEPTH: AtomicUsize = AtomicUsize::new(0);

            pub fn sti() {
                DEPTH.fetch_add(1, Ordering::SeqCst);
            }

            pub fn cli() {
                DEPTH.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

/// The kernel time modules live in src/time.rs (single source of
/// truth, no test copies): `super::TICKS` is a host stand-in for the
/// kernel's jiffies counter (no timer interrupt advances it here —
/// tests drive wheel_tick/run_expired with explicit `now` values
/// instead). It is a file module (not inline) because the included
/// kernel files resolve `#[path]` relative to src/, like every other
/// kernel include above.
pub mod time;

#[cfg(test)]
mod m2tests;
#[cfg(test)]
mod m3tests;
#[cfg(test)]
mod rbtreetests;
#[cfg(test)]
mod rttests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timetests;
#[cfg(test)]
mod vmatests;
#[cfg(test)]
mod waitqtests;
