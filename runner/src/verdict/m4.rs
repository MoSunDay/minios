//! M4 suite spec (plans/linux26 40 §2.1 suite protocol): the VFS
//! glue's namespace semantics on the real root disk — namei (root
//! paths, DIRSIZ truncation, negative-dentry flip, file-as-parent)
//! and fs mutations (EEXIST balance, dir dots, link semantics,
//! ENOTEMPTY balance, inode-table pressure). Root-disk round: PASS
//! exit 33 under the suite=m4 marker. Self-tests mirror the M3 ones:
//! one legal complete log is green, every failure mode is red.

use super::SuiteSpec;

pub fn m4_suite_spec() -> SuiteSpec {
    SuiteSpec {
        suite: "m4",
        run: "core-up",
        expected_cases: &[
            "m4.namei.root_paths",
            "m4.namei.dirsiz_trunc",
            "m4.namei.negative_flip",
            "m4.namei.file_as_parent",
            "m4.fs.create_eexist",
            "m4.fs.dir_dots",
            "m4.fs.link_two_names",
            "m4.fs.unlink_notempty",
            "m4.fs.table_pressure",
            "m4.pc.hit_miss",
            "m4.pc.share_handles",
            "m4.pc.fill_matches_readi",
            "m4.pc.evict_clean",
            "m4.pc.gate_window",
            "m4.pc.readahead_seq",
            "m4.pc.probe_pressure",
            "m4.pc.fd_read",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{judge, RunOutcome, EXPECTED_EXIT_CODE};
    use super::*;

    fn good_m4_log() -> String {
        let mut log = String::from("loading minios x86_64 M4 suite\r\n");
        for c in m4_suite_spec().expected_cases {
            log.push_str(&format!("CASE {c} PASS\n"));
        }
        let n = m4_suite_spec().expected_cases.len();
        log.push_str(&format!(
            "MINIOS_X86_64_TEST_PASS_V1 suite=m4 run=core-up total={n} passed={n} failed=0 skipped=0\n"
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
            &m4_suite_spec(),
            &outcome(&good_m4_log(), Some(EXPECTED_EXIT_CODE), false)
        )
        .is_ok());
    }

    #[test]
    fn missing_case_is_red() {
        let mut log = good_m4_log();
        log = log.replace("CASE m4.fs.dir_dots PASS\n", "");
        assert!(judge(&m4_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn fail_case_is_red() {
        let log = good_m4_log().replace("CASE m4.fs.dir_dots PASS", "CASE m4.fs.dir_dots FAIL");
        assert!(judge(&m4_suite_spec(), &outcome(&log, Some(33), false)).is_err());
    }

    #[test]
    fn wrong_exit_is_red() {
        assert!(judge(&m4_suite_spec(), &outcome(&good_m4_log(), Some(35), false)).is_err());
        assert!(judge(&m4_suite_spec(), &outcome(&good_m4_log(), None, true)).is_err());
        // Extra marker field mismatch (total) is red too.
        let n = m4_suite_spec().expected_cases.len();
        let log = good_m4_log().replace(&format!("passed={n}"), &format!("passed={}", n - 1));
        assert!(judge(
            &m4_suite_spec(),
            &outcome(&log, Some(EXPECTED_EXIT_CODE), false)
        )
        .is_err());
    }

    #[test]
    fn case_ids_unique() {
        let spec = m4_suite_spec();
        let mut seen = std::collections::BTreeSet::new();
        for c in spec.expected_cases {
            assert!(seen.insert(c), "duplicate case id {c}");
        }
        assert_eq!(spec.suite, "m4");
    }
}
