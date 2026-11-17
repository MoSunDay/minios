//! M0R spec + verdict self-tests (plans/linux26 40 §2.2; frozen with the
//! M0R evidence — the GOOD_LOG below pins the recorded serial shape).

use super::*;

pub fn m0r_spec() -> SuiteSpec {
    SuiteSpec {
        suite: "m0r",
        run: "core-up",
        expected_cases: &[
            "m0r.multiboot",
            "m0r.long_mode",
            "m0r.low_identity",
            "m0r.text_permissions",
            "m0r.rodata_permissions",
            "m0r.data_permissions",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_LOG: &str = "loading minios x86_64 M0R\r\n\
minios x86_64 M0R boot\n\
multiboot_info=0x2000\n\
CASE m0r.multiboot PASS\n\
CASE m0r.long_mode PASS\n\
CASE m0r.low_identity PASS\n\
CASE m0r.text_permissions PASS\n\
CASE m0r.rodata_permissions PASS\n\
CASE m0r.data_permissions PASS\n\
MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=6 failed=0 skipped=0\n";

    fn ok(log: &str) -> RunOutcome {
        RunOutcome {
            serial_log: log.to_string(),
            exit_code: Some(33),
            timed_out: false,
        }
    }

    fn judge_bad(log: &str, exit: Option<i32>, timeout: bool, needle: &str) {
        let spec = m0r_spec();
        let outcome = RunOutcome {
            serial_log: log.to_string(),
            exit_code: exit,
            timed_out: timeout,
        };
        let err = judge(&spec, &outcome).expect_err("must not be green");
        assert!(
            err.iter().any(|e| e.contains(needle)),
            "expected reason containing {needle:?}, got {err:?}"
        );
    }

    #[test]
    fn valid_complete_log_is_green() {
        assert!(judge(&m0r_spec(), &ok(GOOD_LOG)).is_ok());
    }

    #[test]
    fn empty_log_is_red() {
        judge_bad("", Some(33), false, "empty serial log");
    }

    #[test]
    fn missing_marker_is_red() {
        let log = "CASE m0r.multiboot PASS\n";
        judge_bad(log, Some(33), false, "missing");
    }

    #[test]
    fn incomplete_marker_is_red() {
        let log = "CASE m0r.multiboot PASS\n\
                   MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6\n";
        judge_bad(log, Some(33), false, "marker missing field");
    }

    #[test]
    fn forged_marker_counts_are_red() {
        let log = "CASE m0r.multiboot PASS\n\
                   CASE m0r.long_mode PASS\n\
                   CASE m0r.low_identity PASS\n\
                   CASE m0r.text_permissions PASS\n\
                   CASE m0r.rodata_permissions PASS\n\
                   CASE m0r.data_permissions PASS\n\
                   MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=5 failed=0 skipped=0\n";
        judge_bad(log, Some(33), false, "inconsistent");
    }

    #[test]
    fn duplicate_case_is_red() {
        let mut log = String::new();
        for id in m0r_spec().expected_cases {
            log.push_str(&format!("CASE {id} PASS\n"));
        }
        log.push_str("CASE m0r.multiboot PASS\n");
        log.push_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=6 failed=0 skipped=0\n");
        judge_bad(&log, Some(33), false, "duplicate case id");
    }

    #[test]
    fn duplicate_marker_is_red() {
        let mut log = GOOD_LOG.to_string();
        log.push_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=6 failed=0 skipped=0\n");
        judge_bad(&log, Some(33), false, "duplicate pass markers");
    }

    #[test]
    fn missing_case_is_red() {
        let log = "CASE m0r.multiboot PASS\n\
                   CASE m0r.long_mode PASS\n\
                   MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=2 passed=2 failed=0 skipped=0\n";
        judge_bad(log, Some(33), false, "missing expected case");
    }

    #[test]
    fn unknown_case_is_red() {
        let log = "CASE m0r.multiboot PASS\n\
                   CASE m0r.bogus PASS\n\
                   MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=2 passed=2 failed=0 skipped=0\n";
        judge_bad(log, Some(33), false, "unknown case id");
    }

    #[test]
    fn total_zero_is_red() {
        let log = "MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=0 passed=0 failed=0 skipped=0\n";
        judge_bad(log, Some(33), false, "total=0");
    }

    #[test]
    fn required_skip_is_red() {
        let mut log = String::new();
        log.push_str("CASE m0r.multiboot SKIP\n");
        for id in &m0r_spec().expected_cases[1..] {
            log.push_str(&format!("CASE {id} PASS\n"));
        }
        log.push_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=5 failed=0 skipped=0\n");
        judge_bad(&log, Some(33), false, "SKIP");
    }

    #[test]
    fn case_fail_is_red() {
        let mut log = String::new();
        log.push_str("CASE m0r.multiboot FAIL\n");
        for id in &m0r_spec().expected_cases[1..] {
            log.push_str(&format!("CASE {id} PASS\n"));
        }
        log.push_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=5 failed=0 skipped=0\n");
        judge_bad(&log, Some(33), false, "status FAIL");
    }

    #[test]
    fn tconf_is_red() {
        let mut log = String::new();
        log.push_str("CASE m0r.multiboot TCONF\n");
        for id in &m0r_spec().expected_cases[1..] {
            log.push_str(&format!("CASE {id} PASS\n"));
        }
        log.push_str("MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=5 failed=0 skipped=0\n");
        judge_bad(&log, Some(33), false, "TCONF");
    }

    #[test]
    fn panic_after_marker_is_red() {
        let mut log = GOOD_LOG.to_string();
        log.push_str("MINIOS_X86_64_TEST_FATAL_V1 suite=m0r run=core-up reason=panic\n");
        judge_bad(&log, Some(33), false, "FATAL_V1");
    }

    #[test]
    fn wrong_exit_code_is_red() {
        judge_bad(GOOD_LOG, Some(1), false, "exit code 1");
        judge_bad(GOOD_LOG, Some(35), false, "exit code 35");
    }

    #[test]
    fn timeout_is_red() {
        judge_bad(GOOD_LOG, None, true, "timeout");
    }

    #[test]
    fn wrong_suite_run_is_red() {
        let log = GOOD_LOG.replace("suite=m0r run=core-up", "suite=m9 run=parity");
        judge_bad(&log, Some(33), false, "suite=m9");
    }
}
