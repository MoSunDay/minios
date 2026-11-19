//! M2 suite spec (plans/linux26/40 §2.1 suite protocol): the kernel-mode
//! physical-page foundation cases (bootmem, buddy, direct map, SLUB,
//! vmalloc, ioremap skeleton). Kernel-only round: no root disk, PASS
//! exit 33 under the suite=m2 marker. Self-tests mirror the M1R ones:
//! one legal complete log is green, every failure mode is red.

use super::SuiteSpec;

pub fn m2_suite_spec() -> SuiteSpec {
    SuiteSpec {
        suite: "m2",
        run: "core-up",
        expected_cases: &[
            "m2.bootmem.table",
            "m2.buddy.reserved",
            "m2.buddy.conserve_merge",
            "m2.directmap.roundtrip",
            "m2.directmap.cover",
            "m2.slub.kmalloc",
            "m2.vmalloc.map_perm",
            "m2.vmalloc.vfree_once",
            "m2.ioremap.skeleton",
            "m2.vma.layout",
            "m2.mmap.eager",
            "m2.mprotect.perm",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{judge, RunOutcome, EXPECTED_EXIT_CODE};
    use super::*;

    fn good_m2_log() -> String {
        let mut log = String::from("loading minios x86_64 M2 suite\r\n");
        for c in m2_suite_spec().expected_cases {
            log.push_str(&format!("CASE {c} PASS\n"));
        }
        log.push_str(
            "MINIOS_X86_64_TEST_PASS_V1 suite=m2 run=core-up total=12 passed=12 failed=0 skipped=0\n",
        );
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
            &m2_suite_spec(),
            &outcome(&good_m2_log(), Some(EXPECTED_EXIT_CODE), false)
        )
        .is_ok());
    }

    #[test]
    fn missing_case_is_red() {
        let mut log = good_m2_log();
        log = log.replace("CASE m2.slub.kmalloc PASS\n", "");
        assert!(judge(&m2_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn fail_case_is_red() {
        let log = good_m2_log().replace("CASE m2.slub.kmalloc PASS", "CASE m2.slub.kmalloc FAIL");
        assert!(judge(&m2_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn wrong_exit_is_red() {
        assert!(judge(&m2_suite_spec(), &outcome(&good_m2_log(), Some(35), false)).is_err());
        assert!(judge(&m2_suite_spec(), &outcome(&good_m2_log(), None, true)).is_err());
        // Extra marker field mismatch (total) is red too.
        let log = good_m2_log().replace("total=12 passed=12", "total=12 passed=11");
        assert!(judge(
            &m2_suite_spec(),
            &outcome(&log, Some(EXPECTED_EXIT_CODE), false)
        )
        .is_err());
    }

    #[test]
    fn case_ids_unique() {
        let spec = m2_suite_spec();
        let mut seen = std::collections::BTreeSet::new();
        for c in spec.expected_cases {
            assert!(seen.insert(c), "duplicate case id {c}");
        }
        assert_eq!(spec.suite, "m2");
    }
}
