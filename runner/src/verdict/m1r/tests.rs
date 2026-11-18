//! Verdict self-tests for the M1R specs and the expected-fault judge
//! (plans/linux26 40 §2.2: every failure mode must be red, and one legal
//! complete log must be green).

use super::super::{judge, RunOutcome, EXPECTED_EXIT_CODE};
use super::*;

const GOOD_M1R_LOG: &str = "loading minios x86_64 M1R suite\r\n\
CASE m1r.irq.pit PASS\n\
CASE m1r.uaccess.copyin_efault PASS\n\
CASE m1r.uaccess.copyout_efault PASS\n\
CASE m1r.uaccess.kernelptr_efault PASS\n\
CASE m1r.ksym.self PASS\n\
CASE m1r.fs.blockread PASS\n\
CASE m1r.fs.inode PASS\n\
CASE m1r.fs.file PASS\n\
m1r-user-write-ok\n\
CASE m1r.user.write PASS\n\
CASE m1r.user.getpid_fork PASS\n\
CASE m1r.user.fork_wait PASS\n\
m1r-exec-child: replaced image\n\
CASE m1r.user.execve PASS\n\
CASE m1r.user.execve_enoent PASS\n\
CASE m1r.user.brk PASS\n\
CASE m1r.user.sse_crosstalk PASS\n\
r\n\
CASE m1r.user.syscall_regs PASS\n\
CASE m1r.user.open_read PASS\n\
CASE m1r.user.pipe PASS\n\
m1r-user-dup-ok\n\
d\n\
CASE m1r.user.dup_close PASS\n\
MINIOS_X86_64_TEST_PASS_V1 suite=m1r run=core-up total=19 passed=19 failed=0 skipped=0\n";

fn fault_outcome(log: &str, exit: Option<i32>, timeout: bool) -> RunOutcome {
    RunOutcome {
        serial_log: log.to_string(),
        exit_code: exit,
        timed_out: timeout,
    }
}

fn good_fault_log() -> String {
    let mut log = String::from("loading minios x86_64 M1R fault\r\n");
    for c in m1r_fault_spec().cases {
        let rdi = if c.cr2_eq_rdi {
            USER_UNMAPPED_VA
        } else {
            MAGIC_RDI
        };
        let cr2 = if c.cr2_eq_rdi {
            format!("0x{USER_UNMAPPED_VA:x}")
        } else {
            "0x0".to_string()
        };
        log.push_str(&format!(
            "M1RFault fault_id={} kind={} ec=0x{:x} cr2={} rip={}+0x8 rax=0x{:x} rbx=0x{:x} \
             rdi=0x{:x} rsi=0x{:x} rdx=0x{:x} r10=0x{:x} cs=0x8 \
             backtrace={FRAME0_SYM}+0x1a|deeper_mangled_sym+0x22\n",
            c.fault_id,
            c.kind,
            c.ec,
            cr2,
            c.rip_sym,
            MAGIC_RAX,
            MAGIC_RBX,
            rdi,
            MAGIC_RSI,
            MAGIC_RDX,
            MAGIC_R10
        ));
        if let Some(pk) = c.pf_kind {
            log.push_str(&format!("  fault-kind: {pk}\n"));
        }
        log.push_str(&format!("CASE {} PASS\n", c.fault_id));
    }
    log.push_str(
        "MINIOS_X86_64_TEST_PASS_V1 suite=m1r-fault run=core-up total=5 passed=5 failed=0 skipped=0\n",
    );
    log
}

fn judge_bad(log: &str, exit: Option<i32>, needle: &str) {
    let err = judge_fault(&m1r_fault_spec(), &fault_outcome(log, exit, false))
        .expect_err("must not be green");
    assert!(
        err.iter().any(|e| e.contains(needle)),
        "expected reason containing {needle:?}, got {err:?}"
    );
}

#[test]
fn m1r_suite_valid_log_is_green() {
    let outcome = RunOutcome {
        serial_log: GOOD_M1R_LOG.to_string(),
        exit_code: Some(EXPECTED_EXIT_CODE),
        timed_out: false,
    };
    assert!(judge(&m1r_suite_spec(), &outcome).is_ok());
}

#[test]
fn m1r_suite_missing_case_is_red() {
    let log = GOOD_M1R_LOG.replace("CASE m1r.ksym.self PASS\n", "");
    let outcome = RunOutcome {
        serial_log: log,
        exit_code: Some(EXPECTED_EXIT_CODE),
        timed_out: false,
    };
    let err = judge(&m1r_suite_spec(), &outcome).expect_err("must be red");
    assert!(err.iter().any(|e| e.contains("missing expected case")));
}

#[test]
fn valid_fault_log_is_green() {
    assert!(judge_fault(
        &m1r_fault_spec(),
        &fault_outcome(&good_fault_log(), Some(EXPECTED_FAULT_EXIT_CODE), false)
    )
    .is_ok());
}

#[test]
fn empty_log_is_red() {
    judge_bad("", Some(EXPECTED_FAULT_EXIT_CODE), "empty serial log");
}

#[test]
fn wrong_exit_is_red() {
    judge_bad(
        &good_fault_log(),
        Some(EXPECTED_EXIT_CODE),
        "exit code 33 != 35",
    );
}

#[test]
fn timeout_is_red() {
    let err = judge_fault(
        &m1r_fault_spec(),
        &fault_outcome(&good_fault_log(), None, true),
    )
    .expect_err("must not be green");
    assert!(err.iter().any(|e| e.contains("timeout")));
}

#[test]
fn missing_fault_line_is_red() {
    let gp = "M1RFault fault_id=m1rfault.gp.noncanon kind=GP ec=0x0 cr2=0x0 \
              rip=_m1rf_inject_gp_noncanon+0x8 rax=0xcafe000000000001 rbx=0xcafe000000000002 \
              rdi=0xcafe000000000003 rsi=0xcafe000000000004 rdx=0xcafe000000000005 \
              r10=0xcafe000000000006 cs=0x8 backtrace=_m1r_fault_driver_frame+0x1a|deeper_mangled_sym+0x22\n";
    let log = good_fault_log().replace(gp, "");
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "found 4");
}

#[test]
fn extra_fault_line_is_red() {
    let mut log = good_fault_log();
    log.push_str(
        "M1RFault fault_id=m1rfault.gp.noncanon kind=GP ec=0x0 cr2=0x0 \
                  rip=_m1rf_inject_gp_noncanon+0x8 rax=0xcafe000000000001 rbx=0xcafe000000000002 \
                  rdi=0xcafe000000000003 rsi=0xcafe000000000004 rdx=0xcafe000000000005 \
                  r10=0xcafe000000000006 cs=0x8 backtrace=_m1r_fault_driver_frame+0x1a\n",
    );
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "found 6");
}

#[test]
fn wrong_fault_id_order_is_red() {
    let log = good_fault_log().replace("m1rfault.de.div0", "m1rfault.bp.int3");
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "fault_id=m1rfault.bp.int3, expected m1rfault.de.div0",
    );
}

#[test]
fn wrong_ec_is_red() {
    let log = good_fault_log().replacen("ec=0x3", "ec=0x2", 1);
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "ec=0x2, expected 0x3");
}

#[test]
fn wrong_magic_register_is_red() {
    let log = good_fault_log().replacen("rax=0xcafe000000000001", "rax=0xcafe0000000000ff", 1);
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "rax=0xcafe0000000000ff",
    );
}

#[test]
fn wrong_cs_is_red() {
    let log = good_fault_log().replacen("cs=0x8", "cs=0x33", 1);
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "cs=0x33");
}

#[test]
fn wrong_rip_symbol_is_red() {
    let log = good_fault_log().replacen(
        "rip=_m1rf_inject_de_div0+0x8",
        "rip=_m1rf_inject_bp_int3+0x8",
        1,
    );
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "expected prefix _m1rf_inject_de_div0+",
    );
}

#[test]
fn wrong_backtrace_head_is_red() {
    let log = good_fault_log().replacen(
        "backtrace=_m1r_fault_driver_frame+0x1a|",
        "backtrace=_some_other_frame+0x1a|",
        1,
    );
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "expected prefix _m1r_fault_driver_frame+",
    );
}

#[test]
fn cr2_rdi_mismatch_is_red() {
    let log = good_fault_log().replacen(
        "cr2=0x555500000000 rip=_m1rf_inject_pf_unmapped",
        "cr2=0x1234 rip=_m1rf_inject_pf_unmapped",
        1,
    );
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "cr2=0x1234 != rdi=0x555500000000",
    );
}

#[test]
fn missing_fault_kind_line_is_red() {
    let log = good_fault_log().replacen("  fault-kind: not-present\n", "", 1);
    judge_bad(
        &log,
        Some(EXPECTED_FAULT_EXIT_CODE),
        "fault-kind: not-present",
    );
}

#[test]
fn fatal_after_marker_is_red() {
    let mut log = good_fault_log();
    log.push_str("MINIOS_X86_64_TEST_FATAL_V1 suite=m1r-fault run=core-up reason=panic\n");
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "FATAL_V1");
}

#[test]
fn wrong_case_status_is_red() {
    let log = good_fault_log().replacen(
        "CASE m1rfault.bp.int3 PASS",
        "CASE m1rfault.bp.int3 FAIL",
        1,
    );
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "CASE #4");
}

#[test]
fn wrong_suite_marker_is_red() {
    let log = good_fault_log().replace("suite=m1r-fault", "suite=m9");
    judge_bad(&log, Some(EXPECTED_FAULT_EXIT_CODE), "suite=m9");
}

#[test]
fn specs_are_frozen() {
    let suite = m1r_suite_spec();
    assert_eq!(suite.suite, "m1r");
    assert_eq!(suite.expected_cases.len(), 19);
    let fault = m1r_fault_spec();
    assert_eq!(fault.suite, "m1r-fault");
    assert_eq!(fault.cases.len(), 5);
}
