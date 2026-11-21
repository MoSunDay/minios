//! M3 suite spec (plans/linux26/40 §2.1 suite protocol): the kernel-mode
//! CFS/RT/waitqueue/time/kthread/workqueue/softirq policy cases (pick order, tick fairness, RT level
//! order, FIFO head requeue, RR rotation, wiring invariants, wake
//! discipline, unlink). Kernel-only round: no
//! root disk, PASS exit 33 under the suite=m3 marker. Self-tests mirror
//! the M2 ones: one legal complete log is green, every failure mode is
//! red.

use super::SuiteSpec;

pub fn m3_suite_spec() -> SuiteSpec {
    SuiteSpec {
        suite: "m3",
        run: "core-up",
        expected_cases: &[
            "m3.cfs.pick_order",
            "m3.cfs.tick_fairness",
            "m3.rt.prio_order",
            "m3.rt.fifo_head",
            "m3.rt.rr_rotate",
            "m3.sched.policy_wiring",
            "m3.wq.wake_policy",
            "m3.wq.unlink_middle",
            "m3.time.clock_layer_wiring",
            "m3.time.hrtimer_order_cancel",
            "m3.time.wheel_cascade",
            "m3.kthread.run_exit",
            "m3.workq.dedup_run",
            "m3.softirq.order_bh_mask",
            "m3.softirq.tasklet_sched",
            "m3.softirq.ksoftirqd_pressure",
            "m3.sig.send_state_machine",
            "m3.sig.frame_layout",
            "m3.sig.alarm_sigalrm",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{judge, RunOutcome, EXPECTED_EXIT_CODE};
    use super::*;

    fn good_m3_log() -> String {
        let mut log = String::from("loading minios x86_64 M3 suite\r\n");
        for c in m3_suite_spec().expected_cases {
            log.push_str(&format!("CASE {c} PASS\n"));
        }
        let n = m3_suite_spec().expected_cases.len();
        log.push_str(&format!(
            "MINIOS_X86_64_TEST_PASS_V1 suite=m3 run=core-up total={n} passed={n} failed=0 skipped=0\n"
        ));
        log
    }

    fn outcome(log: &str, exit: Option<i32>, timeout: bool) -> RunOutcome {
        RunOutcome {
            serial_log: log.to_string(),
            exit_code: exit,
            timed_out: timeout,
        }
    }

    #[test]
    fn complete_log_is_green() {
        assert!(judge(
            &m3_suite_spec(),
            &outcome(&good_m3_log(), Some(EXPECTED_EXIT_CODE), false)
        )
        .is_ok());
    }

    #[test]
    fn missing_case_is_red() {
        let mut log = good_m3_log();
        log = log.replace("CASE m3.rt.prio_order PASS\n", "");
        assert!(judge(&m3_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn fail_case_is_red() {
        let log = good_m3_log().replace("CASE m3.rt.prio_order PASS", "CASE m3.rt.prio_order FAIL");
        assert!(judge(&m3_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn wrong_exit_is_red() {
        assert!(judge(&m3_suite_spec(), &outcome(&good_m3_log(), Some(35), false)).is_err());
        assert!(judge(&m3_suite_spec(), &outcome(&good_m3_log(), None, true)).is_err());
        // Extra marker field mismatch (total) is red too.
        let n = m3_suite_spec().expected_cases.len();
        let log = good_m3_log().replace(&format!("passed={n}"), &format!("passed={}", n - 1));
        assert!(judge(
            &m3_suite_spec(),
            &outcome(&log, Some(EXPECTED_EXIT_CODE), false)
        )
        .is_err());
    }

    #[test]
    fn case_ids_unique() {
        let spec = m3_suite_spec();
        let mut seen = std::collections::BTreeSet::new();
        for c in spec.expected_cases {
            assert!(seen.insert(c), "duplicate case id {c}");
        }
        assert_eq!(spec.suite, "m3");
    }
}
