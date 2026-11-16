//! Pure verdict logic for the M0R Core smoke suite (plans/linux26 40 §2).
//! No I/O here: callers pass the serial log and process outcome; all
//! judgements are deterministic and unit-tested for failure modes.

pub const MARKER_PREFIX: &str = "MINIOS_X86_64_TEST_";
pub const MARKER_PASS: &str = "MINIOS_X86_64_TEST_PASS_V1";
pub const MARKER_FAIL: &str = "MINIOS_X86_64_TEST_FAIL_V1";
pub const MARKER_FATAL: &str = "MINIOS_X86_64_TEST_FATAL_V1";
pub const EXPECTED_EXIT_CODE: i32 = 33;

/// Frozen suite/run identity and the required case list for one round.
pub struct SuiteSpec {
    pub suite: &'static str,
    pub run: &'static str,
    pub expected_cases: &'static [&'static str],
}

/// Observed outcome of one QEMU run.
pub struct RunOutcome {
    pub serial_log: String,
    /// Process exit code; `None` when the runner killed or could not reap it.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Judge one run. `Ok(())` is green; `Err(reasons)` lists every violation.
pub fn judge(spec: &SuiteSpec, outcome: &RunOutcome) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();

    if outcome.timed_out {
        errs.push("timeout: suite deadline exceeded before QEMU exit".to_string());
    }
    match outcome.exit_code {
        Some(EXPECTED_EXIT_CODE) => {}
        Some(code) => errs.push(format!("exit code {code} != {EXPECTED_EXIT_CODE}")),
        None => errs.push("no QEMU exit code (killed or unreaped)".to_string()),
    }

    let log = outcome.serial_log.as_str();
    if log.trim().is_empty() {
        errs.push("empty serial log".to_string());
        return Err(errs);
    }

    let mut case_status: Vec<(String, String)> = Vec::new();
    let mut pass_markers = 0usize;
    let mut marker_fields: Option<Vec<(String, String)>> = None;
    let mut marker_line_no = 0usize;

    for (idx, raw) in log.lines().enumerate() {
        let line = raw.trim_end_matches(['\r']);
        if line.starts_with("CASE ") {
            if pass_markers > 0 {
                errs.push(format!("protocol line after pass marker: {line}"));
            }
            let rest = &line["CASE ".len()..];
            match rest.split_once(' ') {
                Some((id, status)) if !id.is_empty() => {
                    if case_status.iter().any(|(seen, _)| seen == id) {
                        errs.push(format!("duplicate case id {id}"));
                    }
                    case_status.push((id.to_string(), status.to_string()));
                }
                _ => errs.push(format!("malformed CASE line: {line}")),
            }
            continue;
        }
        if !line.starts_with(MARKER_PREFIX) {
            if line.contains(MARKER_FAIL)
                || line.contains(MARKER_FATAL)
                || line.to_ascii_lowercase().contains("panic")
            {
                errs.push(format!("fatal/fail/panic in log: {line}"));
            }
            continue;
        }
        if line.starts_with(MARKER_PASS) {
            pass_markers += 1;
            marker_line_no = idx;
            marker_fields = Some(parse_kv(&line[MARKER_PASS.len()..]));
        } else if line.starts_with(MARKER_FAIL) || line.starts_with(MARKER_FATAL) {
            errs.push(format!("non-pass marker in log: {line}"));
        }
    }

    if pass_markers == 0 {
        errs.push(format!("missing {MARKER_PASS} marker"));
    } else if pass_markers > 1 {
        errs.push(format!("duplicate pass markers: {pass_markers}"));
    }

    // Case identity checks against the frozen expected list.
    for (id, status) in &case_status {
        if !spec.expected_cases.contains(&id.as_str()) {
            errs.push(format!("unknown case id {id}"));
            continue;
        }
        match status.as_str() {
            "PASS" => {}
            "SKIP" => errs.push(format!("required case {id} SKIP")),
            other => errs.push(format!("case {id} status {other}")),
        }
    }
    for expected in spec.expected_cases {
        if !case_status.iter().any(|(id, _)| id == expected) {
            errs.push(format!("missing expected case {expected}"));
        }
    }

    // Marker field self-consistency.
    if let Some(fields) = marker_fields {
        let get = |k: &str| {
            fields
                .iter()
                .find(|(fk, _)| fk == k)
                .map(|(_, v)| v.clone())
        };
        let expect = |k: &str, want: &str, errs: &mut Vec<String>| match get(k) {
            Some(v) if v == want => {}
            Some(v) => errs.push(format!("marker {k}={v}, expected {want}")),
            None => errs.push(format!("marker missing field {k}")),
        };
        expect("suite", spec.suite, &mut errs);
        expect("run", spec.run, &mut errs);
        let number = |k: &str, errs: &mut Vec<String>| -> Option<u64> {
            match get(k) {
                Some(v) => match v.parse::<u64>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        errs.push(format!("marker {k}={v} is not a number"));
                        None
                    }
                },
                None => {
                    errs.push(format!("marker missing field {k}"));
                    None
                }
            }
        };
        let total = number("total", &mut errs);
        let passed = number("passed", &mut errs);
        expect("failed", "0", &mut errs);
        expect("skipped", "0", &mut errs);
        if let Some(total) = total {
            if total == 0 {
                errs.push("marker total=0".to_string());
            }
            let want = spec.expected_cases.len() as u64;
            if total != want {
                errs.push(format!("marker total={total} != expected {want} cases"));
            }
        }
        if let (Some(total), Some(passed)) = (total, passed) {
            let passes = case_status.iter().filter(|(_, s)| s == "PASS").count() as u64;
            if passed != passes || passed > total {
                errs.push(format!(
                    "marker passed={passed} inconsistent with {passes} PASS cases (total {total})"
                ));
            }
        }
        // The pass marker must be the final protocol line of the suite.
        let protocol_after = log.lines().enumerate().any(|(idx, l)| {
            idx > marker_line_no && (l.starts_with("CASE ") || l.starts_with(MARKER_PREFIX))
        });
        if protocol_after {
            errs.push("pass marker is not the last protocol line".to_string());
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn parse_kv(rest: &str) -> Vec<(String, String)> {
    rest.split_whitespace()
        .filter_map(|tok| tok.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

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
