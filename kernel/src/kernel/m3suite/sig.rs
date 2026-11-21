//! m3-suite signal cases (21 §6): the generation state machine
//! (pend/block/ignore/force, SIGKILL fallback), the rt_sigframe pure
//! halves (fill/placement/handler entry), and the alarm expiry switch
//! from xv6-kill to real SIGALRM delivery.

use super::super::signal;
use super::super::sigset::{self, SigAction};
use crate::kernel::traps::Trapframe;
use crate::proc::ProcState;
use core::sync::atomic::{AtomicBool, Ordering};

static SIG_DONE: AtomicBool = AtomicBool::new(false);

extern "C" fn sigprobe_body(_arg: usize) -> i32 {
    SIG_DONE.store(true, Ordering::SeqCst);
    0
}

fn sig_done() -> bool {
    SIG_DONE.load(Ordering::SeqCst)
}

/// m3.sig.send_state_machine: default dispositions pend (blocked ones
/// defer without loss), ignored signals drop at generation, force_send
/// unblocks, procmask hows compose (KILL/STOP never blockable), and
/// SIGKILL keeps the xv6 fatal contract.
pub(super) fn case_sig_send_state_machine() -> bool {
    // SAFETY: boot context owns the fresh slot until run/reap.
    let p = unsafe { crate::proc::kthread::kthread_create(sigprobe_body, 0, b"sigprobe") };
    if p.is_null() {
        return false;
    }
    // SAFETY: EMBRYO slot pinned by this context throughout.
    let checks = unsafe {
        // (a) default action pends even while blocked (deferral).
        (*p).sig_blocked = sigset::sigmask(sigset::SIGUSR1);
        signal::sig_send(p, sigset::SIGUSR1);
        let a = (*p).sig_pending == sigset::sigmask(sigset::SIGUSR1);
        // (b) ignored signals drop at generation.
        (*p).sig_actions[sigset::SIGPIPE as usize - 1].handler = sigset::SIG_IGN;
        signal::sig_send(p, sigset::SIGPIPE);
        let b = (*p).sig_pending & sigset::sigmask(sigset::SIGPIPE) == 0;
        // (c) force semantics: pending AND unblocked (first_pending sees it).
        signal::force_send(p, sigset::SIGUSR1);
        let c = sigset::first_pending((*p).sig_pending, (*p).sig_blocked) == Some(sigset::SIGUSR1);
        // (d) procmask hows; KILL/STOP stripped everywhere.
        let d = sigset::apply_procmask(0, 0, sigset::sigmask(sigset::SIGTERM))
            == Some(sigset::sigmask(sigset::SIGTERM))
            && sigset::apply_procmask(
                0,
                0,
                sigset::sigmask(sigset::SIGKILL) | sigset::sigmask(sigset::SIGSTOP),
            ) == Some(0)
            && sigset::apply_procmask(
                1,
                sigset::sigmask(sigset::SIGTERM) | sigset::sigmask(sigset::SIGSEGV),
                sigset::sigmask(sigset::SIGTERM),
            ) == Some(sigset::sigmask(sigset::SIGSEGV))
            && sigset::apply_procmask(2, 0xf, sigset::sigmask(sigset::SIGHUP))
                == Some(sigset::sigmask(sigset::SIGHUP))
            && sigset::apply_procmask(3, 0, 0).is_none();
        // (e) SIGKILL: the xv6 fatal mark, not a pending bit.
        signal::sig_send(p, sigset::SIGKILL);
        let e = (*p).killed == 1 && (*p).sig_pending & sigset::sigmask(sigset::SIGKILL) == 0;
        // Let the probe run to a clean exit below.
        (*p).killed = 0;
        a && b && c && d && e
    };
    if !checks {
        return false;
    }
    // Drive the probe to ZOMBIE and reap it (the kthread harness
    // discipline of m3.kthread.run_exit).
    // SAFETY: boot context; the slot is ours.
    unsafe {
        crate::proc::kthread::kthread_run(p);
        if !crate::proc::sched::sched_drive(sig_done, 64) {
            return false;
        };
        {
            let _guard = crate::proc::PTABLE_LOCK.lock();
            if (*p).state != ProcState::ZOMBIE {
                return false;
            }
        }
        let rp = crate::proc::kthread::kthread_reap(p);
        rp == 0
    }
}

/// A trap frame with distinguishable register images.
fn probe_tf() -> Trapframe {
    Trapframe {
        rdi: 0x1001,
        rsi: 0x1002,
        rdx: 0x1003,
        rcx: 0x1004,
        rax: 0x1005,
        r8: 0x1006,
        r9: 0x1007,
        r10: 0x1008,
        r11: 0x1009,
        rbx: 0x100a,
        rbp: 0x100b,
        r12: 0x100c,
        r13: 0x100d,
        r14: 0x100e,
        r15: 0x100f,
        vector: 14,
        err: 4,
        rip: 0x2000,
        cs: 0x33,
        rflags: 0x202,
        rsp: 0x7fff_1234_5678,
        ss: 0x2b,
    }
}

/// m3.sig.frame_layout: the pure delivery halves — fill_frame image
/// (retaddr/signum/magic/mask + full register copy), placement math
/// (red-zone clear, handler-entry rsp = 8 mod 16), and enter_handler
/// (register redirect, sa_mask+sig blocking, SA_NODEFER/SA_RESETHAND).
pub(super) fn case_sig_frame_layout() -> bool {
    let mut tf = probe_tf();
    let act = SigAction {
        handler: 0x1234_0000,
        flags: sigset::SA_RESTORER | sigset::SA_RESETHAND,
        restorer: 0x2100_0000,
        mask: sigset::sigmask(sigset::SIGSEGV),
    };
    let mut fr = sigset::Sigframe {
        retaddr: 0,
        signum: 0,
        magic: 0,
        tf: [0; 22],
        mask: 0,
    };
    signal::fill_frame(&mut fr, &tf, sigset::SIGUSR1, 0xf0f0, &act);
    let fill_ok = fr.retaddr == act.restorer
        && fr.signum == sigset::SIGUSR1 as u64
        && fr.magic == sigset::SIGFRAME_MAGIC
        && fr.mask == 0xf0f0
        && fr.tf[0] == tf.rdi
        && fr.tf[7] == tf.r10
        && fr.tf[14] == tf.r15
        && fr.tf[15] == tf.vector
        && fr.tf[16] == tf.err
        && fr.tf[17] == tf.rip
        && fr.tf[21] == tf.ss;
    // Placement: below the red zone, handler-entry alignment.
    let rsp = 0x7fff_1234_5678u64;
    let base = sigset::frame_base_for(rsp);
    let place_ok = base % 16 == 8 && base + 208 <= rsp - 128;
    // Handler entry: redirect + mask compositing + RESETHAND one-shot.
    let mut blocked = 0u64;
    let mut actions = [SigAction::DFL; sigset::NSIG];
    signal::enter_handler(
        &mut tf,
        &mut blocked,
        &mut actions,
        base,
        &act,
        sigset::SIGUSR1,
    );
    let enter_ok = tf.rip == 0x1234_0000
        && tf.rsp == base
        && tf.rdi == sigset::SIGUSR1 as u64
        && tf.rax == 0
        && tf.rcx == 0
        && tf.r11 == 0
        && blocked == sigset::sigmask(sigset::SIGUSR1) | sigset::sigmask(sigset::SIGSEGV)
        && actions[sigset::SIGUSR1 as usize - 1] == SigAction::DFL;
    // SA_NODEFER: the handler's own signal stays deliverable.
    let nd_act = SigAction {
        handler: 1,
        flags: sigset::SA_RESTORER | sigset::SA_NODEFER,
        restorer: 1,
        mask: 0,
    };
    let mut nd_blocked = 0u64;
    let mut nd_actions = [SigAction::DFL; sigset::NSIG];
    nd_actions[sigset::SIGUSR2 as usize - 1] = nd_act;
    signal::enter_handler(
        &mut tf,
        &mut nd_blocked,
        &mut nd_actions,
        base,
        &nd_act,
        sigset::SIGUSR2,
    );
    let nodefer_ok = nd_blocked == 0 && nd_actions[sigset::SIGUSR2 as usize - 1] == nd_act;
    fill_ok && place_ok && enter_ok && nodefer_ok
}

/// m3.sig.alarm_sigalrm: wheel expiry (alarm_fire, called exactly as
/// the wheel would) marks alarm_fired and generates SIGALRM instead of
/// the pre-s6 xv6 kill; an ignored SIGALRM drops at generation.
pub(super) fn case_sig_alarm_sigalrm() -> bool {
    // SAFETY: boot context owns the fresh slot.
    let p = unsafe { crate::proc::kthread::kthread_create(sigprobe_body, 0, b"alrmprobe") };
    if p.is_null() {
        return false;
    }
    // SAFETY: EMBRYO slot pinned by this context; the alarm field is
    // exclusively ours (the timer is never queued — alarm_fire only
    // touches its container).
    unsafe {
        crate::proc::proc::alarm_fire(core::ptr::addr_of_mut!((*p).alarm));
        let live = (*p).alarm_fired == 1
            && (*p).killed == 0
            && (*p).sig_pending == sigset::sigmask(sigset::SIGALRM);
        (*p).sig_pending = 0;
        (*p).sig_actions[sigset::SIGALRM as usize - 1].handler = sigset::SIG_IGN;
        crate::proc::proc::alarm_fire(core::ptr::addr_of_mut!((*p).alarm));
        let ignored = (*p).sig_pending == 0;
        {
            let _guard = crate::proc::PTABLE_LOCK.lock();
            crate::proc::proc::freeproc(p);
        }
        let gone = {
            let _guard = crate::proc::PTABLE_LOCK.lock();
            (*p).state == ProcState::UNUSED
        };
        live && ignored && gone
    }
}
