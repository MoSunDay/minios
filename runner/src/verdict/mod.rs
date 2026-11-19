//! Pure verdict logic for the Core suites (plans/linux26 40 §2).
//! No I/O here: callers pass the serial log and process outcome; all
//! judgements are deterministic and unit-tested for failure modes.
//! Suite-specific specs live in the m0r/m1r siblings; this module holds
//! the shared suite-protocol judge.

mod m0r;
mod m1r;
mod m2;

pub use m0r::m0r_spec;
pub use m1r::{
    judge_fault, m1r_fault_spec, m1r_suite_spec, m1r_user_spec, EXPECTED_FAULT_EXIT_CODE,
};
pub use m2::m2_suite_spec;

/// Per-case `(case_id, status)` pairs from `CASE <id> <STATUS>` protocol
/// lines, for coverage evidence derivation. The suite judge remains the
/// verdict authority (duplicate/malformed handling lives there); coverage
/// runs only after a PASS verdict, on the same serial log.
pub fn case_results(log: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for raw in log.lines() {
        // Same CR trim as the judge: GRUB's banner leaves a leading CR.
        let line = raw.trim_matches(['\r']);
        let Some(rest) = line.strip_prefix("CASE ") else {
            continue;
        };
        let mut it = rest.split_whitespace();
        if let (Some(id), Some(status)) = (it.next(), it.next()) {
            if it.next().is_none() {
                out.insert(id.to_string(), status.to_string());
            }
        }
    }
    out
}

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

/// Shared outcome precheck for both judges: timeout, exit-code mismatch,
/// missing exit code, empty serial log (single source for the literals).
pub(super) fn base_outcome_errs(outcome: &RunOutcome, expected_exit: i32) -> Vec<String> {
    let mut errs = Vec::new();
    if outcome.timed_out {
        errs.push("timeout: suite deadline exceeded before QEMU exit".to_string());
    }
    match outcome.exit_code {
        Some(code) if code == expected_exit => {}
        Some(code) => errs.push(format!("exit code {code} != {expected_exit}")),
        None => errs.push("no QEMU exit code (killed or unreaped)".to_string()),
    }
    if outcome.serial_log.trim().is_empty() {
        errs.push("empty serial log".to_string());
    }
    errs
}

/// Shared red scan: any non-pass protocol marker, plus FAIL/FATAL/panic
/// content on non-protocol lines. CASE and M1RFault lines belong to their
/// own judges and are exempt here.
pub(super) fn scan_forbidden(log: &str) -> Vec<String> {
    let mut errs = Vec::new();
    for raw in log.lines() {
        // GRUB's banner ends LF+CR, leaving a leading CR on the next line.
        let line = raw.trim_matches(['\r']);
        if line.starts_with(MARKER_PREFIX) {
            if !line.starts_with(MARKER_PASS) {
                errs.push(format!("non-pass marker in log: {line}"));
            }
        } else if !line.starts_with("CASE ") && !line.starts_with("M1RFault ") {
            if line.contains(MARKER_FAIL)
                || line.contains(MARKER_FATAL)
                || line.to_ascii_lowercase().contains("panic")
            {
                errs.push(format!("fatal/fail/panic in log: {line}"));
            }
        }
    }
    errs
}

/// Judge one run. `Ok(())` is green; `Err(reasons)` lists every violation.
pub fn judge(spec: &SuiteSpec, outcome: &RunOutcome) -> Result<(), Vec<String>> {
    let mut errs = base_outcome_errs(outcome, EXPECTED_EXIT_CODE);
    let log = outcome.serial_log.as_str();
    if log.trim().is_empty() {
        return Err(errs);
    }

    let mut case_status: Vec<(String, String)> = Vec::new();
    let mut pass_markers = 0usize;
    let mut marker_fields: Option<Vec<(String, String)>> = None;
    let mut marker_line_no = 0usize;

    for (idx, raw) in log.lines().enumerate() {
        // Trim CR on both ends: GRUB's serial echo ends its banner with LF+CR,
        // leaving a leading CR on the next kernel line.
        let line = raw.trim_matches(['\r']);
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
        if line.starts_with(MARKER_PASS) {
            pass_markers += 1;
            marker_line_no = idx;
            marker_fields = Some(parse_kv(&line[MARKER_PASS.len()..]));
        }
    }
    // Shared red scan owns non-pass markers and fatal/fail/panic content.
    errs.extend(scan_forbidden(log));

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
