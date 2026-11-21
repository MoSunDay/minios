//! M3 kernel-mode suite: CFS policy (pick order, tick fairness), RT
//! FIFO/RR level semantics, scheduler wiring invariants, the s4 time
//! layer, and the s5 kthread/workqueue/softirq machinery. Cases live
//! in area submodules — `sched` (CFS/RT/waitqueue policy + wiring),
//! `time` (clocksource/clockevents/hrtimer/wheel), `bh` (kthread/
//! workqueue/softirq) — while this file owns the merged runner table
//! and the verdict/exit path. Runs in the boot context before the
//! scheduler starts (policy-level battery; process-level dynamics are
//! the s4 guest-time counters). Kernel-only (no root disk): boot
//! cmdline `m3-suite` -> CASE lines -> merged verdict, exit 0x10 ->
//! QEMU 33 (0x11 on any failure).

mod bh;
mod sched;
mod time;

use crate::arch::x86_64::io;
use crate::console;
use crate::kernel::softirq;

pub fn run() -> ! {
    ksymtab_placeholder();
    // Resident bottom-half pressure valve for the softirq cases.
    softirq::spawn_ksoftirqd();
    let checks: [(&str, fn() -> bool); 16] = [
        ("m3.cfs.pick_order", sched::case_cfs_pick_order),
        ("m3.cfs.tick_fairness", sched::case_cfs_tick_fairness),
        ("m3.rt.prio_order", sched::case_rt_prio_order),
        ("m3.rt.fifo_head", sched::case_rt_fifo_head),
        ("m3.rt.rr_rotate", sched::case_rt_rr_rotate),
        ("m3.sched.policy_wiring", sched::case_sched_policy_wiring),
        ("m3.wq.wake_policy", sched::case_wq_wake_policy),
        ("m3.wq.unlink_middle", sched::case_wq_unlink_middle),
        (
            "m3.time.clock_layer_wiring",
            time::case_time_clock_layer_wiring,
        ),
        (
            "m3.time.hrtimer_order_cancel",
            time::case_time_hrtimer_order_cancel,
        ),
        ("m3.time.wheel_cascade", time::case_time_wheel_cascade),
        ("m3.kthread.run_exit", bh::case_kthread_run_exit),
        ("m3.workq.dedup_run", bh::case_workq_dedup_run),
        ("m3.softirq.order_bh_mask", bh::case_softirq_order_bh_mask),
        ("m3.softirq.tasklet_sched", bh::case_softirq_tasklet_sched),
        (
            "m3.softirq.ksoftirqd_pressure",
            bh::case_softirq_ksoftirqd_pressure,
        ),
    ];
    let mut total = 0usize;
    let mut passed = 0usize;
    for (case_id, check) in checks {
        total += 1;
        let ok = check();
        console::write_str("CASE ");
        console::write_str(case_id);
        console::write_str(if ok { " PASS\n" } else { " FAIL\n" });
        if ok {
            passed += 1;
        }
    }
    let failed = total - passed;
    if failed == 0 {
        console::write_str("MINIOS_X86_64_TEST_PASS_V1 suite=m3 run=core-up total=");
    } else {
        console::write_str("MINIOS_X86_64_TEST_FAIL_V1 suite=m3 run=core-up total=");
    }
    write_decimal(total);
    console::write_str(" passed=");
    write_decimal(passed);
    console::write_str(" failed=");
    write_decimal(failed);
    console::write_str(" skipped=0\n");
    io::out32(0xf4, if failed == 0 { 0x10 } else { 0x11 });
    loop {
        io::hlt();
    }
}

/// m2suite names its symbol prefix via ksymtab; m3 cases are policy-only
/// (no injected faults), so just arm the suite name for the log.
fn ksymtab_placeholder() {
    super::ksymtab::set_active_suite("m3");
}

fn write_decimal(mut n: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &buf[i..] {
        crate::arch::x86_64::console::write_byte(d);
    }
}
