//! The dedicated expected-fault judge (plans/linux26 40 §2.1.6): matches
//! the pre-declared fault_id sequence, fault kinds/error codes, RIP symbol
//! prefixes, magic registers and the backtrace head exactly; any extra or
//! unexpected fault stays red. Its PASS never substitutes for an ordinary
//! suite verdict.

use super::super::{judge, RunOutcome, SuiteSpec, MARKER_FAIL, MARKER_FATAL, MARKER_PASS};
use super::{
    FaultCaseSpec, FaultSpec, EXPECTED_FAULT_EXIT_CODE, KERNEL_CS, MAGIC_R10, MAGIC_RAX, MAGIC_RBX,
    MAGIC_RDI, MAGIC_RDX, MAGIC_RSI, USER_UNMAPPED_VA,
};

/// Field map of one `M1RFault ...` line.
struct FaultLine {
    fields: Vec<(String, String)>,
}

impl FaultLine {
    fn get(&self, k: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(fk, _)| fk == k)
            .map(|(_, v)| v.as_str())
    }
    fn req<'a>(&'a self, k: &str, errs: &mut Vec<String>) -> Option<&'a str> {
        match self.get(k) {
            Some(v) => Some(v),
            None => {
                errs.push(format!("M1RFault line missing field {k}"));
                None
            }
        }
    }
}

fn parse_fields(line: &str) -> FaultLine {
    FaultLine {
        fields: line
            .split_whitespace()
            .filter_map(|tok| tok.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn parse_hex(v: &str, what: &str, i: usize, errs: &mut Vec<String>) -> Option<u64> {
    match u64::from_str_radix(v.trim_start_matches("0x"), 16) {
        Ok(x) => Some(x),
        Err(_) => {
            errs.push(format!("M1RFault #{i} {what}={v} is not hex"));
            None
        }
    }
}

/// Judge one expected-fault run. `Ok(())` is green; `Err(reasons)` lists
/// every violation (never early-returns on the first).
pub fn judge_fault(spec: &FaultSpec, outcome: &RunOutcome) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();

    if outcome.timed_out {
        errs.push("timeout: suite deadline exceeded before QEMU exit".to_string());
    }
    match outcome.exit_code {
        Some(EXPECTED_FAULT_EXIT_CODE) => {}
        Some(code) => errs.push(format!("exit code {code} != {EXPECTED_FAULT_EXIT_CODE}")),
        None => errs.push("no QEMU exit code (killed or unreaped)".to_string()),
    }

    let log = outcome.serial_log.as_str();
    if log.trim().is_empty() {
        errs.push("empty serial log".to_string());
        return Err(errs);
    }

    // Collect M1RFault lines, CASE lines, and scan for anything fatal.
    let mut fault_lines: Vec<(usize, &str)> = Vec::new();
    let mut case_lines: Vec<(usize, &str)> = Vec::new();
    for (idx, raw) in log.lines().enumerate() {
        // GRUB's banner ends LF+CR, leaving a leading CR on the next line.
        let line = raw.trim_matches(['\r']);
        if line.starts_with("M1RFault ") {
            fault_lines.push((idx, line));
            continue;
        }
        if line.starts_with("CASE ") {
            case_lines.push((idx, line));
            continue;
        }
        if line.starts_with("MINIOS_X86_64_TEST_") {
            if !line.starts_with(MARKER_PASS) {
                errs.push(format!("non-pass marker in log: {line}"));
            }
            continue;
        }
        if line.contains(MARKER_FAIL)
            || line.contains(MARKER_FATAL)
            || line.to_ascii_lowercase().contains("panic")
        {
            errs.push(format!("fatal/fail/panic in log: {line}"));
        }
    }

    // Fault-count identity: exactly one line per declared case, in order.
    if fault_lines.len() != spec.cases.len() {
        errs.push(format!(
            "expected exactly {} M1RFault lines, found {}",
            spec.cases.len(),
            fault_lines.len()
        ));
    }
    for (i, (line_idx, line)) in fault_lines.iter().enumerate() {
        let Some(case_spec) = spec.cases.get(i) else {
            break;
        };
        check_fault_line(i, *line_idx, case_spec, line, log, &mut errs);
    }

    // CASE lines: every declared fault id, PASS, in order.
    if case_lines.len() != spec.cases.len() {
        errs.push(format!(
            "expected exactly {} CASE lines, found {}",
            spec.cases.len(),
            case_lines.len()
        ));
    }
    for (i, (_, line)) in case_lines.iter().enumerate() {
        let Some(case_spec) = spec.cases.get(i) else {
            break;
        };
        let want = format!("CASE {} PASS", case_spec.fault_id);
        if *line != want {
            errs.push(format!("CASE #{i} '{line}', expected '{want}'"));
        }
    }

    // Totals marker: reuse the shared suite judge on a synthetic spec (the
    // exit-code expectation differs and is already recorded above).
    let leaked: &'static [&'static str] = spec
        .cases
        .iter()
        .map(|c| c.fault_id)
        .collect::<Vec<_>>()
        .leak();
    let suite_outcome = RunOutcome {
        serial_log: outcome.serial_log.clone(),
        exit_code: Some(super::super::EXPECTED_EXIT_CODE),
        timed_out: outcome.timed_out,
    };
    let suite_spec = SuiteSpec {
        suite: spec.suite,
        run: spec.run,
        expected_cases: leaked,
    };
    if let Err(marker_errs) = judge(&suite_spec, &suite_outcome) {
        for e in marker_errs {
            if !e.starts_with("exit code") {
                errs.push(e);
            }
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Check one observed M1RFault line against its declared case.
fn check_fault_line(
    i: usize,
    line_idx: usize,
    case_spec: &FaultCaseSpec,
    line: &str,
    log: &str,
    errs: &mut Vec<String>,
) {
    let fl = parse_fields(line);
    match fl.get("fault_id") {
        Some(id) if id == case_spec.fault_id => {}
        Some(id) => errs.push(format!(
            "M1RFault #{i} fault_id={id}, expected {}",
            case_spec.fault_id
        )),
        None => errs.push(format!("M1RFault #{i} missing fault_id")),
    }
    if let Some(v) = fl.req("kind", errs) {
        if v != case_spec.kind {
            errs.push(format!(
                "M1RFault #{i} kind={v}, expected {}",
                case_spec.kind
            ));
        }
    }
    if let Some(v) = fl.req("ec", errs) {
        if let Some(ec) = parse_hex(v, "ec", i, errs) {
            if ec != case_spec.ec {
                errs.push(format!(
                    "M1RFault #{i} ec={ec:#x}, expected {:#x}",
                    case_spec.ec
                ));
            }
        }
    }
    // Register contract (rdi carries the faulting VA for pf.unmapped).
    let mut regs: Vec<(&str, u64)> = vec![
        ("rax", MAGIC_RAX),
        ("rbx", MAGIC_RBX),
        ("rsi", MAGIC_RSI),
        ("rdx", MAGIC_RDX),
        ("r10", MAGIC_R10),
        ("cs", KERNEL_CS),
        (
            "rdi",
            if case_spec.cr2_eq_rdi {
                USER_UNMAPPED_VA
            } else {
                MAGIC_RDI
            },
        ),
    ];
    for (k, want) in regs.drain(..) {
        if let Some(v) = fl.req(k, errs) {
            if let Some(got) = parse_hex(v, k, i, errs) {
                if got != want {
                    errs.push(format!("M1RFault #{i} {k}={got:#x}, expected {want:#x}"));
                }
            }
        }
    }
    // RIP: symbolized prefix match (D-3), offset free.
    if let Some(v) = fl.req("rip", errs) {
        let want_prefix = format!("{}+", case_spec.rip_sym);
        if !v.starts_with(&want_prefix) {
            errs.push(format!(
                "M1RFault #{i} rip={v}, expected prefix {want_prefix}"
            ));
        }
    }
    // Backtrace head: the fixed driver frame (D-3 prefix granularity).
    if let Some(v) = fl.req("backtrace", errs) {
        let want_prefix = format!("{}+", super::FRAME0_SYM);
        if !v.starts_with(&want_prefix) {
            errs.push(format!(
                "M1RFault #{i} backtrace head {v}, expected prefix {want_prefix}"
            ));
        }
    }
    // cr2 cross-check for the unmapped-VA fault.
    if case_spec.cr2_eq_rdi {
        let (cr2, rdi) = (fl.get("cr2"), fl.get("rdi"));
        if cr2.is_some() && rdi.is_some() && cr2 != rdi {
            errs.push(format!(
                "M1RFault #{i} cr2={} != rdi={} for the unmapped-VA fault",
                cr2.unwrap(),
                rdi.unwrap()
            ));
        }
    }
    // The extra fault-kind line directly under a #PF line.
    if let Some(pf_kind) = case_spec.pf_kind {
        let next = log
            .lines()
            .nth(line_idx + 1)
            .map(|l| l.trim_end_matches(['\r']));
        match next {
            Some(l) if l == format!("  fault-kind: {pf_kind}") => {}
            other => errs.push(format!(
                "M1RFault #{i} expected following line '  fault-kind: {pf_kind}', got {other:?}"
            )),
        }
    }
}
