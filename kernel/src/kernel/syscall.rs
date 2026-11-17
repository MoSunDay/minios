//! Syscall dispatch hub. Numbers follow Linux 2.6.39.4 x86-64; the shared
//! abi crate and the real handlers arrive with the S3 userspace chain.
//! Until then every unclaimed number returns -ENOSYS.

use super::traps::Trapframe;

pub const ENOSYS: i64 = -38;

/// Linux x86-64 syscall numbers wired in M1R.
pub const SYS_EXIT: u64 = 60;

/// C entry from the syscall stub (tf.rax = number, arg registers follow
/// the x86-64 kernel ABI: rdi rsi rdx r10 r8 r9).
pub fn dispatch(tf: &mut Trapframe) -> i64 {
    match tf.rax {
        SYS_EXIT => crate::proc::proc::exit(tf.rdi as i32),
        _ => ENOSYS,
    }
}

/// Timer hook (S3 nanosleep bookkeeping); no-op until the suite lands.
pub fn on_tick(_now: u64) {}
