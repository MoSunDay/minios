//! Pure syscall argument decode (host-testable): the x86-64 kernel ABI
//! order (rdi rsi rdx r10 r8 r9 — r10 replaces the hardware-clobbered rcx)
//! and Linux-style widening/narrowing rules (04 §3: return values in
//! -4095..-1 are errnos, everything else is a value).

/// One decoded syscall invocation: number + six argument slots.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SyscallArgs {
    pub nr: u64,
    pub a: [u64; 6],
}

impl SyscallArgs {
    /// Arguments in kernel-ABI order (rdi rsi rdx r10 r8 r9).
    pub const fn new(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> Self {
        SyscallArgs {
            nr,
            a: [a0, a1, a2, a3, a4, a5],
        }
    }

    /// Raw u64 argument `i` (0-based).
    pub fn arg64(&self, i: usize) -> u64 {
        self.a[i]
    }

    /// Argument `i` as an `isize`-like signed value (sign-extended from the
    /// 64-bit register; e.g. brk deltas and pid arguments).
    #[allow(dead_code)] // host-testable API; kernel callers use the casts inline
    pub fn arg_i64(&self, i: usize) -> i64 {
        self.a[i] as i64
    }

    /// Argument `i` as a pointer-sized unsigned value (canonical addresses).
    pub fn arg_usize(&self, i: usize) -> usize {
        self.a[i] as usize
    }

    /// Argument `i` narrowed to a C `int` (Linux ABI: callers pass 32-bit
    /// values zero/sign-extended; the kernel narrows back).
    pub fn arg_i32(&self, i: usize) -> i32 {
        self.a[i] as u32 as i32
    }

    /// A nullable out-pointer argument (0 = NULL, Linux convention).
    pub fn arg_opt_ptr(&self, i: usize) -> Option<usize> {
        match self.a[i] {
            0 => None,
            p => Some(p as usize),
        }
    }

    /// wait4-style pid decode: > 0 targets that pid, <= 0 means "any child"
    /// (process groups collapse to "any" in the single-group M1R kernel).
    pub fn wait_pid(&self, i: usize) -> i32 {
        let pid = self.arg_i32(i);
        if pid > 0 {
            pid
        } else {
            -1
        }
    }

    /// Linux syscall return contract: values in -4095..=-1 are errnos.
    #[allow(dead_code)] // host-testable API (04 §3)
    pub const fn is_neg_errno(v: i64) -> bool {
        v >= -4095 && v < 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SyscallArgs {
        SyscallArgs::new(61, 0xffff_ffff, 0x2000, 0, 0, 0, 0) // wait4(-1, status, ...)
    }

    #[test]
    fn arg4_comes_from_r10_slot() {
        let a = SyscallArgs::new(0, 1, 2, 3, 4, 5, 6);
        assert_eq!(a.arg64(3), 4, "4th arg lives in r10, not rcx");
        assert_eq!(a.arg64(4), 5);
        assert_eq!(a.arg64(5), 6);
    }

    #[test]
    fn i32_narrowing_truncates() {
        // exit(0x123456789) narrows to 0x56789 then to status low bits.
        let a = SyscallArgs::new(60, 0x1_0000_0002, 0, 0, 0, 0, 0);
        assert_eq!(a.arg_i32(0), 2);
    }

    #[test]
    fn sign_extension_for_i64() {
        let a = SyscallArgs::new(12, 0xffff_ffff_ffff_ffff, 0, 0, 0, 0, 0);
        assert_eq!(a.arg_i64(0), -1);
    }

    #[test]
    fn wait_pid_decode() {
        let any = SyscallArgs::new(61, 0xffff_ffff, 0, 0, 0, 0, 0); // -1
        assert_eq!(any.wait_pid(0), -1);
        let zero = SyscallArgs::new(61, 0, 0, 0, 0, 0, 0); // 0 = group
        assert_eq!(zero.wait_pid(0), -1);
        let pid = SyscallArgs::new(61, 17, 0, 0, 0, 0, 0);
        assert_eq!(pid.wait_pid(0), 17);
        let _ = sample();
    }

    #[test]
    fn optional_pointers() {
        assert_eq!(sample().arg_opt_ptr(1), Some(0x2000));
        assert_eq!(sample().arg_opt_ptr(2), None);
    }

    #[test]
    fn errno_window() {
        assert!(SyscallArgs::is_neg_errno(-14));
        assert!(SyscallArgs::is_neg_errno(-4095));
        assert!(!SyscallArgs::is_neg_errno(-4096));
        assert!(!SyscallArgs::is_neg_errno(0));
        assert!(!SyscallArgs::is_neg_errno(33));
    }
}
