use super::{EINTR, EINVAL};
use crate::mm::uaccess;

/// gettimeofday(2) (Linux x86-64 96): report the wall clock. There is
/// no RTC until M8+, so wall = boot_epoch(0) + monotonic (ADR-0019).
/// tv NULL is accepted (no copyout); tz is ignored.
pub(super) fn sys_gettimeofday(tv: Option<usize>) -> i64 {
    let ns = crate::kernel::time::clocksource::wall_ns();
    let sec = (ns / 1_000_000_000) as i64;
    let usec = ((ns % 1_000_000_000) / 1_000) as i64;
    if let Some(dst) = tv {
        let mut ts = [0u8; 16];
        ts[0..8].copy_from_slice(&sec.to_le_bytes());
        ts[8..16].copy_from_slice(&usec.to_le_bytes());
        let r = uaccess::copyout(dst as *mut u8, ts.as_ptr(), 16);
        if r != 0 {
            return r;
        }
    }
    0
}

/// alarm(2) (37): arm the process alarm wheel timer; returns the whole
/// seconds (rounded up) remaining on the previous alarm, 0 if none.
/// seconds == 0 cancels. Expiry (wheel callback alarm_fire, s6) marks
/// alarm_fired and generates SIGALRM for delivery at user return.
pub(super) fn sys_alarm(seconds: i64) -> i64 {
    let p = crate::proc::proc::myproc();
    if p.is_null() {
        return 0;
    }
    // SAFETY: myproc() is the running process; the alarm field is
    // exclusively ours and the wheel lock is a leaf (ptable not held).
    unsafe {
        let prev = if (*p).alarm.queued() {
            let rem = (*p)
                .alarm
                .expires()
                .saturating_sub(crate::kernel::time::wheel::jiffies());
            (rem + 99) / 100
        } else {
            0
        };
        if seconds < 0 {
            return EINVAL;
        }
        if seconds == 0 {
            if (*p).alarm.queued() {
                crate::kernel::time::wheel::del_timer(&mut (*p).alarm);
            }
        } else {
            // Linux retruncates oversize alarms; here the deadline clamps
            // to the wheel horizon (tv5 placement beyond it is imprecise).
            let offset = (seconds as u64)
                .saturating_mul(100)
                .min(crate::kernel::time::wheel::WHEEL_HORIZON);
            let expires = crate::kernel::time::wheel::jiffies().saturating_add(offset);
            crate::kernel::time::wheel::add_timer(
                &mut (*p).alarm,
                expires,
                crate::proc::proc::alarm_fire,
            );
        }
        prev as i64
    }
}

/// nanosleep(2) (35): park on an on-stack hrtimer until the monotonic
/// deadline. The callback must take TICK_LOCK before the wakeup so the
/// check-then-sleep window stays atomic against the expiry (a fire in
/// that window blocks on the sleeper's TICK_LOCK and its wakeup then
/// observes the sleeper — no missed wake). Cancelled on every return so
/// the on-stack timer never outlives the call; a killed sleeper aborts
/// with EINTR (xv6-visible).
pub(super) fn sys_nanosleep(req: usize) -> i64 {
    let mut ts = [0u8; 16];
    match uaccess::copyin(ts.as_mut_ptr(), req as *const u8, 16) {
        0 => {}
        e => return e,
    }
    let sec = i64::from_le_bytes(ts[0..8].try_into().unwrap());
    let nsec = i64::from_le_bytes(ts[8..16].try_into().unwrap());
    if nsec < 0 || nsec >= 1_000_000_000 || sec < 0 {
        return EINVAL;
    }
    let dur_ns = (sec as u64) * 1_000_000_000 + nsec as u64;
    if dur_ns == 0 {
        return 0;
    }
    let mut timer = crate::kernel::time::hrtimer::HrTimer::new();
    let deadline = crate::kernel::time::clocksource::monotonic_ns().saturating_add(dur_ns);
    // One channel for the sleep and the callback to agree on byte-for-byte.
    let chan = (&mut timer as *mut _) as usize;
    // SAFETY: timer outlives every armed window (cancelled on all returns).
    unsafe {
        crate::kernel::time::hrtimer::hrtimer_start(
            &mut timer,
            deadline,
            crate::kernel::time::hrtimer::HrMode::Abs,
            nanosleep_wake,
        );
        let p = crate::proc::proc::myproc();
        loop {
            // SAFETY: balanced TICK_LOCK around each sleep check (same
            // protocol as the previous ticks-channel implementation).
            crate::kernel::time::TICK_LOCK.raw_lock();
            if crate::kernel::time::clocksource::monotonic_ns() >= deadline {
                crate::kernel::time::TICK_LOCK.raw_release();
                break;
            }
            // xv6 sys_sleep: a killed sleeper aborts (EINTR); the next
            // trap from user mode exits it (traps_dispatch). An
            // unblocked pending signal aborts the same way (s6): its
            // handler/default runs at the syscall return path.
            if !p.is_null() && ((*p).killed != 0 || crate::kernel::signal::pending_unblocked(p)) {
                crate::kernel::time::TICK_LOCK.raw_release();
                crate::kernel::time::hrtimer::hrtimer_cancel(&mut timer);
                return EINTR;
            }
            crate::proc::sleep::sleep(chan, &crate::kernel::time::TICK_LOCK);
            // SAFETY: sleep returns with TICK_LOCK held.
            crate::kernel::time::TICK_LOCK.raw_release();
        }
        crate::kernel::time::hrtimer::hrtimer_cancel(&mut timer);
    }
    0
}

/// nanosleep wakeup (hrtimer callback): take TICK_LOCK around the
/// channel wakeup so the sleeper's check-then-sleep window stays atomic
/// against the expiry.
fn nanosleep_wake(t: *mut crate::kernel::time::hrtimer::HrTimer) {
    // Balanced TICK_LOCK acquire/release (timer-interrupt context; the
    // discipline matches time::tick's own wakeup).
    // SAFETY: same lock discipline as time::tick.
    unsafe {
        crate::kernel::time::TICK_LOCK.raw_lock();
        crate::proc::sleep::wakeup(t as usize);
        crate::kernel::time::TICK_LOCK.raw_release();
    }
}
