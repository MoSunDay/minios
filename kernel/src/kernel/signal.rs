//! Kernel signal glue (21 §6, M3 UP): per-proc pending/blocked/disposition
//! state lives in `Proc`; this module owns generation (`sig_send`), the
//! synchronous-fault force path, user-return delivery (rt_sigframe push),
//! the rt_sig* syscall bodies, and the SIGKILL fallback to the xv6 kill
//! contract. Divergences (single pending bit per signal — no RT queues,
//! Stop-without-job-control terminates, exec keeps blocked+SIG_IGN) are
//! recorded in ADR-0021.

use super::sigset::{self, SigAction, Sigframe};
use super::traps::Trapframe;
use crate::mm::uaccess;
use crate::proc::proc::{exit, myproc};
use crate::proc::sched;
use crate::proc::{Proc, ProcState, PTABLE_LOCK};
use abi::{EINVAL, ESRCH};

/// Internal syscall-dispatch sentinel: rt_sigreturn restored the whole
/// user context, so syscall_dispatch must not touch tf.rax afterwards.
/// Linux reserves -512.. for exactly this kind of in-kernel errno.
pub const SIGRETURN_RET: i64 = -513;

/// Encode a signal death for the wait4 status path: negative xstate far
/// below the legacy kill sentinel (-1 keeps the xv6-visible 0xff exit
/// code, ADR-0021); -256-sig decodes back to sig in sys_wait4.
pub const fn exit_status_for_signal(sig: i32) -> i32 {
    -256 - sig
}

/// Any unblocked pending signal (or the fatal kill mark): the
/// interruptible-sleep predicate (sleep.rs::interrupted).
pub fn pending_unblocked(p: *const Proc) -> bool {
    if p.is_null() {
        return false;
    }
    // SAFETY: stable u64 field reads of a live slot.
    unsafe { (*p).sig_pending & !(*p).sig_blocked != 0 }
}

/// Generate `sig` in `p`: ignored signals drop at generation; blocked
/// ones pend (delivery defers); SIGKILL keeps the xv6 fatal contract.
/// Wake a sleeper whose wait the signal interrupts. Safe from interrupt
/// context (ptable-lock holders run with interrupts disabled — the
/// kill_proc discipline).
pub fn sig_send(p: *mut Proc, sig: i32) {
    if sig == sigset::SIGKILL {
        crate::proc::proc::kill_proc(p);
        return;
    }
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; the slot stays pinned by the caller.
    unsafe {
        if (*p).state == ProcState::UNUSED {
            return;
        }
        if (*p).sig_actions[(sig - 1) as usize].is_ign() {
            return; // discarded at generation (unblocked ignores only; M3)
        }
        (*p).sig_pending |= sigset::sigmask(sig);
        wake_if_sleeping(p, sig);
    }
}

/// Synchronous exceptions (SIGSEGV/SIGFPE/SIGILL/SIGTRAP) must not be
/// deferrable: pending AND unblocked regardless of disposition, so a
/// blocked/ignored faulting signal still terminates at delivery instead
/// of faulting forever (Linux force_sig semantics).
pub fn force_send(p: *mut Proc, sig: i32) {
    if sig == sigset::SIGKILL {
        crate::proc::proc::kill_proc(p);
        return;
    }
    let _guard = PTABLE_LOCK.lock();
    // SAFETY: ptable lock held; the slot stays pinned by the caller.
    unsafe {
        if (*p).state == ProcState::UNUSED {
            return;
        }
        (*p).sig_pending |= sigset::sigmask(sig);
        (*p).sig_blocked &= !sigset::sigmask(sig);
        wake_if_sleeping(p, sig);
    }
}

/// Wake `p` when `sig` interrupts its sleep (ptable lock held).
///
/// # Safety
/// PTABLE_LOCK must be held by this context.
unsafe fn wake_if_sleeping(p: *mut Proc, sig: i32) {
    // SAFETY: ptable lock held; sleeping means the wait_link owns the
    // slot's sleep (the sleeper self-dequeues after swtch).
    unsafe {
        if (*p).sig_blocked & sigset::sigmask(sig) == 0 && (*p).state == ProcState::SLEEPING {
            sched::runnable_wake(p);
        }
    }
}

/// Synchronous user-mode exception (traps.rs): force `sig` in the
/// current process; the trap-tail delivery point then either runs a
/// handler or takes the fatal default. A user frame always has a live
/// proc — a null myproc (boot context) leaves the frame untouched.
pub fn fault_cur(_tf: &mut Trapframe, sig: i32) {
    let p = myproc();
    if !p.is_null() {
        force_send(p, sig);
    }
}

/// exec(2) reset: handlers fall back to default except kept SIG_IGN
/// dispositions; the blocked mask survives (Linux semantics).
pub fn exec_reset(p: *mut Proc) {
    // SAFETY: the caller owns the slot through the exec commit window.
    unsafe {
        (*p).sig_pending = 0;
        for a in (*p).sig_actions.iter_mut() {
            if !a.is_ign() {
                *a = SigAction::DFL;
            }
        }
    }
}

/// Fill the sigframe image (pure half of delivery, m3-suite-tested):
/// retaddr/magic/signum, the full trap-frame copy, and the mask to
/// restore at sigreturn.
pub fn fill_frame(fr: &mut Sigframe, tf: &Trapframe, sig: i32, blocked: u64, act: &SigAction) {
    fr.retaddr = act.restorer;
    fr.signum = sig as u64;
    fr.magic = sigset::SIGFRAME_MAGIC;
    fr.mask = blocked;
    const _: () = assert!(core::mem::size_of::<Trapframe>() == core::mem::size_of::<[u64; 22]>());
    // SAFETY: both sides are 176-byte all-u64 plain images.
    unsafe {
        core::ptr::copy_nonoverlapping(
            tf as *const Trapframe as *const u64,
            fr.tf.as_mut_ptr(),
            22,
        );
    }
}

/// Apply the frame to the trap frame and the proc mask: handler entry
/// registers (rdi=signum, rip=handler, rsp=frame base, rax/rcx/r11
/// clobber-parity with a syscall entry), blocked |= sa_mask (+sig
/// unless SA_NODEFER), SA_RESETHAND one-shot disposition.
pub fn enter_handler(
    tf: &mut Trapframe,
    blocked: &mut u64,
    actions: &mut [SigAction; sigset::NSIG],
    fr_base: u64,
    act: &SigAction,
    sig: i32,
) {
    *blocked = sigset::handler_blocked(*blocked, act, sig);
    if act.flags & sigset::SA_RESETHAND != 0 {
        actions[(sig - 1) as usize] = SigAction::DFL;
    }
    tf.rip = act.handler;
    tf.rsp = fr_base;
    tf.rdi = sig as u64;
    tf.rax = 0;
    tf.rcx = 0;
    tf.r11 = 0;
}

/// The user-return delivery point (xv6 usertrap-tail placement): find
/// the lowest unblocked pending signal and either run its default
/// action or push an rt_sigframe and redirect to the handler. One
/// signal per pass; nesting re-arms on later returns.
pub fn userret_deliver(tf: &mut Trapframe) {
    let p = myproc();
    if p.is_null() {
        return;
    }
    loop {
        // SAFETY: the running process; kernel context, single CPU.
        unsafe {
            if (*p).killed != 0 {
                exit(-1);
            }
            let Some(sig) = sigset::first_pending((*p).sig_pending, (*p).sig_blocked) else {
                return;
            };
            let act = (*p).sig_actions[(sig - 1) as usize];
            (*p).sig_pending &= !sigset::sigmask(sig);
            if act.is_ign() {
                continue;
            }
            if act.is_dfl() {
                let act_def = sigset::default_act(sig);
                if !act_def.is_fatal() {
                    continue; // ignored by default: dropped
                }
                // Term/Core, and Stop until M8 job control: die with the
                // signal's wait status (ADR-0021).
                exit(exit_status_for_signal(sig));
            }
            if act.flags & sigset::SA_RESTORER == 0 || act.restorer == 0 {
                exit(exit_status_for_signal(sig));
            }
            let mut fr = Sigframe {
                retaddr: 0,
                signum: 0,
                magic: 0,
                tf: [0; 22],
                mask: 0,
            };
            fill_frame(&mut fr, tf, sig, (*p).sig_blocked, &act);
            let base = sigset::frame_base_for(tf.rsp);
            // SAFETY: &fr is a 208-byte kernel-stack image.
            if uaccess::copyout(base as *mut u8, &fr as *const Sigframe as *const u8, 208) != 0 {
                // No usable user stack: the signal stays fatal.
                exit(exit_status_for_signal(sig));
            }
            // SAFETY: the running process's own signal fields.
            // SAFETY: place expressions of the running process's own
            // signal fields (outer unsafe block).
            enter_handler(
                tf,
                &mut *core::ptr::addr_of_mut!((*p).sig_blocked),
                &mut *core::ptr::addr_of_mut!((*p).sig_actions),
                base,
                &act,
                sig,
            );
            return;
        }
    }
}

/// rt_sigreturn(2): the restorer stub's syscall with rsp pointing just
/// past the frame's retaddr slot. Validate the cookie, restore the
/// whole context and mask; a forged frame is EINVAL (the stub's ud2
/// then faults terminally).
pub fn sys_rt_sigreturn(tf: &mut Trapframe) -> i64 {
    let base = tf.rsp.wrapping_sub(8);
    let mut fr = Sigframe {
        retaddr: 0,
        signum: 0,
        magic: 0,
        tf: [0; 22],
        mask: 0,
    };
    // SAFETY: plain kernel-buffer write of user bytes (fixup-guarded).
    let r = uaccess::copyin(
        &mut fr as *mut Sigframe as *mut u8,
        base as *const u8,
        core::mem::size_of::<Sigframe>(),
    );
    if r != 0 {
        return r;
    }
    if fr.magic != sigset::SIGFRAME_MAGIC {
        return EINVAL;
    }
    // SAFETY: same-size plain images (asserted in fill_frame).
    unsafe {
        core::ptr::copy_nonoverlapping(fr.tf.as_ptr(), tf as *mut Trapframe as *mut u64, 22);
    }
    let p = myproc();
    if !p.is_null() {
        // SAFETY: the running process.
        unsafe {
            (*p).sig_blocked = fr.mask & !sigset::UNBLOCKABLE;
        }
    }
    SIGRETURN_RET
}

/// rt_sigaction(2): install/query one disposition. `act`/`old_act` are
/// optional user pointers to the 32-byte {handler, flags, restorer,
/// mask} image (the Rust runtime's struct sigaction).
pub fn sys_rt_sigaction(sig: i64, act: Option<usize>, old_act: Option<usize>) -> i64 {
    if !sigset::valid_signal(sig) {
        return EINVAL;
    }
    let sig = sig as i32;
    if !sigset::catchable(sig) {
        return EINVAL;
    }
    let p = myproc();
    if p.is_null() {
        return EINVAL;
    }
    if let Some(src) = act {
        let mut buf = [0u8; 32];
        let r = uaccess::copyin(buf.as_mut_ptr(), src as *const u8, 32);
        if r != 0 {
            return r;
        }
        let new_act = action_from_le(&buf);
        if !new_act.valid(sig) {
            return EINVAL;
        }
        // SAFETY: the running process.
        unsafe {
            (*p).sig_actions[(sig - 1) as usize] = new_act;
        }
    }
    if let Some(dst) = old_act {
        // SAFETY: the running process.
        let cur = unsafe { (*p).sig_actions[(sig - 1) as usize] };
        let buf = action_to_le(&cur);
        let r = uaccess::copyout(dst as *mut u8, buf.as_ptr(), 32);
        if r != 0 {
            return r;
        }
    }
    0
}

fn action_from_le(b: &[u8; 32]) -> SigAction {
    let w = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    SigAction {
        handler: w(0),
        flags: w(8),
        restorer: w(16),
        mask: w(24),
    }
}

fn action_to_le(a: &SigAction) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0..8].copy_from_slice(&a.handler.to_le_bytes());
    b[8..16].copy_from_slice(&a.flags.to_le_bytes());
    b[16..24].copy_from_slice(&a.restorer.to_le_bytes());
    b[24..32].copy_from_slice(&a.mask.to_le_bytes());
    b
}

/// rt_sigprocmask(2): BLOCK/UNBLOCK/SETMASK over the 8-byte set image;
/// old mask copied out first. sigsetsize must be 8 (x86-64).
pub fn sys_rt_sigprocmask(how: i64, set: Option<usize>, old_set: Option<usize>, size: u64) -> i64 {
    if size != 8 {
        return EINVAL;
    }
    let p = myproc();
    if p.is_null() {
        return EINVAL;
    }
    if let Some(dst) = old_set {
        // SAFETY: the running process.
        let cur = unsafe { (*p).sig_blocked };
        let r = uaccess::copyout(dst as *mut u8, cur.to_le_bytes().as_ptr(), 8);
        if r != 0 {
            return r;
        }
    }
    if let Some(src) = set {
        let mut buf = [0u8; 8];
        let r = uaccess::copyin(buf.as_mut_ptr(), src as *const u8, 8);
        if r != 0 {
            return r;
        }
        let new_set = u64::from_le_bytes(buf);
        // SAFETY: the running process.
        let old = unsafe { (*p).sig_blocked };
        match sigset::apply_procmask(how, old, new_set) {
            Some(m) => unsafe {
                (*p).sig_blocked = m;
            },
            None => return EINVAL,
        }
    }
    0
}

/// rt_sigpending(2): copy the 8-byte pending set out.
pub fn sys_rt_sigpending(set: usize, size: u64) -> i64 {
    if size != 8 {
        return EINVAL;
    }
    let p = myproc();
    if p.is_null() {
        return EINVAL;
    }
    // SAFETY: the running process.
    let cur = unsafe { (*p).sig_pending };
    let r = uaccess::copyout(set as *mut u8, cur.to_le_bytes().as_ptr(), 8);
    if r != 0 {
        return r;
    }
    0
}

/// kill(2): send `sig` to `pid` (>0; groups are M8 — pid <= 0 answers
/// EINVAL). sig 0 is the existence probe.
pub fn sys_kill(pid: i32, sig: i64) -> i64 {
    if pid <= 0 {
        return EINVAL;
    }
    if sig != 0 && !sigset::valid_signal(sig) {
        return EINVAL;
    }
    let target = {
        let _guard = PTABLE_LOCK.lock();
        // SAFETY: ptable lock held.
        unsafe {
            let table = crate::proc::ptable_mut();
            let mut found = core::ptr::null_mut();
            for slot in table.iter_mut() {
                if slot.pid == pid && slot.state != ProcState::UNUSED {
                    found = slot as *mut Proc;
                    break;
                }
            }
            found
        }
    };
    if target.is_null() {
        return ESRCH;
    }
    if sig != 0 {
        sig_send(target, sig as i32);
    }
    0
}
