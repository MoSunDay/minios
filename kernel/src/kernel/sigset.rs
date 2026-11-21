//! Signal numbering/sigset core (21 §6, M3 UP): Linux 2.6.39 x86-64
//! signal numbers, u64 sigsets, the default-action table, SigAction
//! validation, and the rt_sigframe layout contract shared with the
//! Rust user runtime (SA_RESTORER stub + magic cookie). Pure data and
//! functions only — no kernel-crate dependencies — so host tests
//! include this file by #[path] like cfs/rt/tf_layout.

/// Signals are 1..=NSIG; bit (sig-1) of a u64 sigset.
pub const NSIG: usize = 64;

// The full number surface mirrors abi's syscall numbers: the kernel glues
// only what M3 uses, the rest name the ABI for the user runtime/tests and
// the M8/M9 signal work (kept dead-code-exempt by contract, not accident).
#[allow(dead_code)]
mod numbers {

    pub const SIGHUP: i32 = 1;
    pub const SIGINT: i32 = 2;
    pub const SIGQUIT: i32 = 3;
    pub const SIGILL: i32 = 4;
    pub const SIGTRAP: i32 = 5;
    pub const SIGABRT: i32 = 6;
    pub const SIGBUS: i32 = 7;
    pub const SIGFPE: i32 = 8;
    pub const SIGKILL: i32 = 9;
    pub const SIGUSR1: i32 = 10;
    pub const SIGSEGV: i32 = 11;
    pub const SIGUSR2: i32 = 12;
    pub const SIGPIPE: i32 = 13;
    pub const SIGALRM: i32 = 14;
    pub const SIGTERM: i32 = 15;
    pub const SIGCHLD: i32 = 17;
    pub const SIGCONT: i32 = 18;
    pub const SIGSTOP: i32 = 19;
    pub const SIGTSTP: i32 = 20;
    pub const SIGTTIN: i32 = 21;
    pub const SIGTTOU: i32 = 22;
    pub const SIGXCPU: i32 = 24;
    pub const SIGXFSZ: i32 = 25;
    pub const SIGWINCH: i32 = 28;
}

pub use numbers::*;

/// sigset bit for `sig` (1-based; sig 0 yields 0 by convention — callers
/// validate the range first).
pub const fn sigmask(sig: i32) -> u64 {
    if sig <= 0 || sig > NSIG as i32 {
        0
    } else {
        1u64 << (sig as u32 - 1)
    }
}

/// KILL/STOP can be neither blocked nor caught.
pub const MASK_SIGKILL: u64 = 1 << 8;
pub const MASK_SIGSTOP: u64 = 1 << 18;
pub const UNBLOCKABLE: u64 = MASK_SIGKILL | MASK_SIGSTOP;

/// `kill(2)`-visible handler constants.
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

/// Linux x86-64 SA_* bits. M3 accepts RESTORER (mandatory on x86-64),
/// NODEFER, and RESETHAND; the rest answer EINVAL at rt_sigaction.
pub const SA_RESTORER: u64 = 0x0400_0000;
pub const SA_NODEFER: u64 = 0x4000_0000;
pub const SA_RESETHAND: u64 = 0x8000_0000;
pub const SA_SUPPORTED: u64 = SA_RESTORER | SA_NODEFER | SA_RESETHAND;

// Bits outside the M3 window (rejected at rt_sigaction until their
// milestone): named for the ABI surface, like the unused signal numbers.
#[allow(dead_code)]
mod sa_bits {
    pub const SA_NOCLDSTOP: u64 = 0x0000_0001;
    pub const SA_NOCLDWAIT: u64 = 0x0000_0002;
    pub const SA_SIGINFO: u64 = 0x0000_0004;
    pub const SA_ONSTACK: u64 = 0x0800_0000;
    pub const SA_RESTART: u64 = 0x1000_0000;
}

/// Disposition table entry (Linux default actions).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefaultAct {
    Ignore,
    Terminate,
    Core,
    /// Stop has no job control to land on until M8 (ADR-0021): M3
    /// treats it as a termination with the signal's wait status.
    Stop,
}

impl DefaultAct {
    /// True when the default action ends the process.
    pub const fn is_fatal(&self) -> bool {
        matches!(
            self,
            DefaultAct::Terminate | DefaultAct::Core | DefaultAct::Stop
        )
    }
}

/// Default action for `sig` (Term for every unmapped/RT number).
pub const fn default_act(sig: i32) -> DefaultAct {
    match sig {
        3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31 => DefaultAct::Core,
        17 | 18 | 23 | 28 => DefaultAct::Ignore,
        19 | 20 | 21 | 22 => DefaultAct::Stop,
        _ => DefaultAct::Terminate,
    }
}

/// One disposition slot (kernel copy of struct sigaction).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    /// Extra mask blocked while the handler runs.
    pub mask: u64,
}

impl SigAction {
    pub const DFL: SigAction = SigAction {
        handler: SIG_DFL,
        flags: 0,
        restorer: 0,
        mask: 0,
    };

    pub const fn is_dfl(&self) -> bool {
        self.handler == SIG_DFL
    }

    pub const fn is_ign(&self) -> bool {
        self.handler == SIG_IGN
    }

    /// rt_sigaction validation: catchable signal, known flag bits,
    /// SA_RESTORER with a live restorer stub (x86-64 contract).
    pub fn valid(&self, sig: i32) -> bool {
        valid_signal(sig as i64)
            && catchable(sig)
            && self.flags & !SA_SUPPORTED == 0
            && self.flags & SA_RESTORER != 0
            && self.restorer != 0
    }
}

/// 1..=NSIG.
pub const fn valid_signal(sig: i64) -> bool {
    sig >= 1 && sig <= NSIG as i64
}

/// KILL and STOP are uncatchable (and unblockable).
pub const fn catchable(sig: i32) -> bool {
    sigmask(sig) & UNBLOCKABLE == 0
}

/// rt_sigprocmask(2) `how` application. Returns None on an unknown how;
/// KILL/STOP bits never enter a block mask.
pub fn apply_procmask(how: i64, old: u64, set: u64) -> Option<u64> {
    let next = match how {
        0 => old | set,
        1 => old & !set,
        2 => set,
        _ => return None,
    };
    Some(next & !UNBLOCKABLE)
}

/// Lowest-numbered pending signal not blocked; Linux delivery order.
pub fn first_pending(pending: u64, blocked: u64) -> Option<i32> {
    let m = pending & !blocked;
    if m == 0 {
        None
    } else {
        Some(m.trailing_zeros() as i32 + 1)
    }
}

/// Block mask while a handler for `sig` runs: the old mask plus the
/// sa_mask, plus the signal itself unless SA_NODEFER (Linux default).
pub fn handler_blocked(old: u64, act: &SigAction, sig: i32) -> u64 {
    let mut m = old | act.mask;
    if act.flags & SA_NODEFER == 0 {
        m |= sigmask(sig);
    }
    m & !UNBLOCKABLE
}

/// The rt_sigframe pushed onto the user stack. `tf` is the trap-frame
/// image (22 u64s; the kernel Trapframe is all-u64 #[repr(C)] and
/// byte-moves into it). Offsets are contract-frozen and host-asserted.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Sigframe {
    /// Handler `ret` target: the user SA_RESTORER stub.
    pub retaddr: u64,
    /// Redundant signum copy (diagnostics; sigreturn does not trust it).
    pub signum: u64,
    /// Anti-forgery cookie checked by rt_sigreturn.
    pub magic: u64,
    /// Full saved user context (176 bytes, Trapframe layout).
    pub tf: [u64; 22],
    /// Blocked mask to restore on sigreturn.
    pub mask: u64,
}

pub const SIGFRAME_MAGIC: u64 = u64::from_le_bytes(*b"sigframe");
pub const SIGFRAME_TF_OFF: usize = 24;
pub const SIGFRAME_MASK_OFF: usize = 200;

/// Placement of the frame for a user rsp: entirely below the 128-byte
/// red zone, handler entry rsp (frame base) congruent to 8 mod 16 —
/// the SysV callee-entry convention a `ret`-delivered handler expects.
pub const fn frame_base_for(rsp: u64) -> u64 {
    (rsp.saturating_sub(128 + 208 + 8) & !0xf) + 8
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<Sigframe>() == 208);
    assert!(offset_of!(Sigframe, retaddr) == 0);
    assert!(offset_of!(Sigframe, signum) == 8);
    assert!(offset_of!(Sigframe, magic) == 16);
    assert!(offset_of!(Sigframe, tf) == SIGFRAME_TF_OFF);
    assert!(offset_of!(Sigframe, mask) == SIGFRAME_MASK_OFF);
    assert!(SIGFRAME_TF_OFF + core::mem::size_of::<[u64; 22]>() == SIGFRAME_MASK_OFF);
};
