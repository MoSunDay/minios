//! m3r-s6 signal-core tests (21 §6): sigset bit packing, delivery
//! order, the default-action table, procmask semantics, SigAction
//! validation, and the rt_sigframe/stack-placement contract the user
//! runtime's SA_RESTORER stub pairs with.

use crate::sigset::{
    apply_procmask, catchable, default_act, first_pending, frame_base_for, handler_blocked,
    sigmask, valid_signal, DefaultAct, SigAction, MASK_SIGKILL, MASK_SIGSTOP, NSIG, SA_NODEFER,
    SA_RESETHAND, SA_RESTORER, SIGALRM, SIGCHLD, SIGCONT, SIGFPE, SIGFRAME_MAGIC,
    SIGFRAME_MASK_OFF, SIGFRAME_TF_OFF, SIGILL, SIGINT, SIGKILL, SIGPIPE, SIGQUIT, SIGSEGV,
    SIGSTOP, SIGTERM, SIGTSTP, SIGTTIN, SIGTTOU, SIGUSR1, SIGWINCH, SIGXCPU, SIGXFSZ,
};

#[test]
fn sigset_bits_and_bounds() {
    assert_eq!(sigmask(0), 0, "sig 0 is a probe, never a bit");
    assert_eq!(sigmask(-3), 0);
    assert_eq!(sigmask(1), 1);
    assert_eq!(sigmask(9), MASK_SIGKILL);
    assert_eq!(sigmask(19), MASK_SIGSTOP);
    assert_eq!(sigmask(NSIG as i32), 1u64 << 63);
    assert_eq!(sigmask(NSIG as i32 + 1), 0, "out of range clamps to 0");
    assert!(valid_signal(1) && valid_signal(NSIG as i64));
    assert!(!valid_signal(0) && !valid_signal(NSIG as i64 + 1));
    assert!(catchable(SIGTERM) && !catchable(SIGKILL) && !catchable(SIGSTOP));
}

#[test]
fn first_pending_delivers_lowest_unblocked() {
    // Lowest number wins among unblocked, blocked ones wait.
    let pending = sigmask(SIGUSR1) | sigmask(SIGSEGV) | sigmask(SIGINT);
    assert_eq!(first_pending(pending, 0), Some(SIGINT));
    assert_eq!(first_pending(pending, sigmask(SIGINT)), Some(SIGUSR1));
    let all_blocked = pending & !(sigmask(SIGINT) | sigmask(SIGSEGV));
    assert_eq!(first_pending(all_blocked, pending), None);
    assert_eq!(first_pending(0, 0), None);
    // KILL bypasses blocking because legal masks never contain it
    // (apply_procmask strips the bit), not by special-casing here.
    assert_eq!(first_pending(MASK_SIGKILL, 0), Some(SIGKILL));
    assert_eq!(first_pending(MASK_SIGKILL, MASK_SIGKILL), None);
}

#[test]
fn default_act_table_matches_linux() {
    assert_eq!(default_act(SIGQUIT), DefaultAct::Core);
    assert_eq!(default_act(SIGILL), DefaultAct::Core);
    assert_eq!(default_act(SIGFPE), DefaultAct::Core);
    assert_eq!(default_act(SIGSEGV), DefaultAct::Core);
    assert_eq!(default_act(SIGXCPU), DefaultAct::Core);
    assert_eq!(default_act(SIGXFSZ), DefaultAct::Core);
    assert_eq!(default_act(SIGCHLD), DefaultAct::Ignore);
    assert_eq!(default_act(SIGCONT), DefaultAct::Ignore);
    assert_eq!(default_act(SIGWINCH), DefaultAct::Ignore);
    assert_eq!(default_act(SIGSTOP), DefaultAct::Stop);
    assert_eq!(default_act(SIGTSTP), DefaultAct::Stop);
    assert_eq!(default_act(SIGTTIN), DefaultAct::Stop);
    assert_eq!(default_act(SIGTTOU), DefaultAct::Stop);
    assert_eq!(default_act(SIGTERM), DefaultAct::Terminate);
    assert_eq!(default_act(SIGPIPE), DefaultAct::Terminate);
    assert_eq!(default_act(SIGALRM), DefaultAct::Terminate);
    assert!(DefaultAct::Core.is_fatal());
    assert!(
        DefaultAct::Stop.is_fatal(),
        "M3: Stop terminates (ADR-0021)"
    );
    assert!(!DefaultAct::Ignore.is_fatal());
}

#[test]
fn apply_procmask_hows_and_unblockable_strip() {
    let old = sigmask(SIGTERM);
    let set = sigmask(SIGINT) | sigmask(SIGUSR1) | MASK_SIGSTOP;
    // SIG_BLOCK / SIG_UNBLOCK / SIG_SETMASK.
    assert_eq!(apply_procmask(0, old, set), Some(old | set & !MASK_SIGSTOP));
    assert_eq!(
        apply_procmask(1, old | set, sigmask(SIGINT)),
        Some(sigmask(SIGTERM) | sigmask(SIGUSR1))
    );
    assert_eq!(apply_procmask(2, old, set), Some(set & !MASK_SIGSTOP));
    assert_eq!(apply_procmask(3, old, set), None, "unknown how is EINVAL");
    // KILL/STOP never enter a mask under any how.
    assert_eq!(apply_procmask(0, 0, MASK_SIGKILL | MASK_SIGSTOP), Some(0));
    assert_eq!(apply_procmask(2, 0, MASK_SIGKILL), Some(0));
}

#[test]
fn sigaction_validation_contract() {
    let ok = SigAction {
        handler: 0x4000_1000,
        flags: SA_RESTORER | SA_NODEFER,
        restorer: 0x4000_2000,
        mask: 0,
    };
    assert!(ok.valid(SIGUSR1));
    assert!(SigAction::DFL.is_dfl());
    assert!(!SigAction::DFL.valid(SIGUSR1), "DFL is not a catch action");
    // x86-64 contract: SA_RESTORER set, restorer non-null.
    let no_restorer_flag = SigAction {
        flags: SA_NODEFER,
        ..ok
    };
    assert!(!no_restorer_flag.valid(SIGUSR1));
    let null_stub = SigAction { restorer: 0, ..ok };
    assert!(!null_stub.valid(SIGUSR1));
    // Unsupported bits rejected, RESETHAND accepted.
    let bad = SigAction {
        flags: SA_RESTORER | 0x4,
        ..ok
    };
    assert!(!bad.valid(SIGUSR1));
    let rh = SigAction {
        flags: SA_RESTORER | SA_RESETHAND,
        ..ok
    };
    assert!(rh.valid(SIGUSR1));
    // KILL/STOP uncatchable even with a perfect action.
    assert!(!ok.valid(SIGKILL));
    assert!(!ok.valid(SIGSTOP));
    assert!(!ok.valid(0) && !ok.valid(NSIG as i32 + 1));
    // SIG_IGN disposition parses as a valid action for catchable sigs.
    let ign = SigAction { handler: 1, ..ok };
    assert!(ign.valid(SIGPIPE) && ign.is_ign());
}

#[test]
fn handler_blocked_masks_and_nodefer() {
    let act = SigAction {
        handler: 1,
        flags: SA_RESTORER,
        restorer: 2,
        mask: sigmask(SIGINT),
    };
    // Default: sa_mask + the signal itself on top of the old mask.
    let b = handler_blocked(sigmask(SIGTERM), &act, SIGUSR1);
    assert_eq!(b, sigmask(SIGTERM) | sigmask(SIGINT) | sigmask(SIGUSR1));
    // SA_NODEFER leaves the running signal deliverable again.
    let nd = SigAction {
        flags: SA_RESTORER | SA_NODEFER,
        ..act
    };
    assert_eq!(
        handler_blocked(sigmask(SIGTERM), &nd, SIGUSR1),
        sigmask(SIGTERM) | sigmask(SIGINT)
    );
    // KILL/STOP never become blocked inside a handler either.
    let evil = SigAction {
        mask: MASK_SIGKILL | MASK_SIGSTOP,
        ..act
    };
    assert_eq!(
        handler_blocked(0, &evil, SIGUSR1) & (MASK_SIGKILL | MASK_SIGSTOP),
        0
    );
}

#[test]
fn sigframe_layout_is_frozen() {
    // repr(C) contract with the user runtime: offsets/size/magic.
    assert_eq!(SIGFRAME_TF_OFF, 24);
    assert_eq!(SIGFRAME_MASK_OFF, 200);
    assert_eq!(core::mem::size_of::<crate::sigset::Sigframe>(), 208);
    assert_eq!(SIGFRAME_MAGIC, u64::from_le_bytes(*b"sigframe"));
}

#[test]
fn frame_base_honors_red_zone_and_alignment() {
    for rsp in [0x7fff_ffff_0000u64, 0x4000_0f13, 0x4000_0f08, 0x1000] {
        let base = frame_base_for(rsp);
        // Below the caller frame with 128B red zone clearance: the
        // lowest frame byte is at least 128 below the old rsp.
        assert!(base + 208 + 128 <= rsp, "red zone clobber at rsp {rsp:#x}");
        // Handler entry rsp congruent 8 mod 16 (SysV callee entry).
        assert_eq!(base % 16, 8, "misaligned base for rsp {rsp:#x}");
    }
    // Saturating path: tiny rsp still aligns without underflow.
    let base = frame_base_for(4);
    assert_eq!(base, 8);
    // Deterministic map: rsp and rsp-8 land on the same base slot.
    assert_eq!(frame_base_for(0x4000_1000), frame_base_for(0x4000_0ff8));
}
