//! M1R specs (plans/linux26 40 §2.1.6, decision D-1/D-2): the ordinary
//! m1r-suite case list (PASS exit 33) and the frozen expected-fault
//! manifest (PASS exit 35, its own namespace). The fault judge lives in
//! `fault`; self-tests in `tests`.

pub mod fault;
#[cfg(test)]
mod tests;

pub use fault::judge_fault;

use super::SuiteSpec;

/// m1r-suite v1 (decision D-2): kernel-mode cases (trap runtime, uaccess,
/// ksym, S4 block layer), the S4 fs cases, and the S3 user-phase cases
/// (plans/linux26 04 §5: user runtime fork/exec/brk/SSE/regs), all merged
/// into one driver marker, PASS exit 33.
pub fn m1r_suite_spec() -> SuiteSpec {
    SuiteSpec {
        suite: "m1r",
        run: "core-up",
        expected_cases: &[
            "m1r.irq.pit",
            "m1r.uaccess.copyin_efault",
            "m1r.uaccess.copyout_efault",
            "m1r.uaccess.kernelptr_efault",
            "m1r.ksym.self",
            "m1r.fs.blockread",
            "m1r.fs.inode",
            "m1r.user.write",
            "m1r.user.getpid_fork",
            "m1r.user.fork_wait",
            "m1r.user.execve",
            "m1r.user.execve_enoent",
            "m1r.user.brk",
            "m1r.user.sse_crosstalk",
            "m1r.user.syscall_regs",
        ],
    }
}

/// m1r-fault PASS exit code (D-1): 0x11 -> QEMU 35, a namespace distinct
/// from the ordinary-suite 33.
pub const EXPECTED_FAULT_EXIT_CODE: i32 = 35;

/// Magic registers the injections carry at fault time (m1rfault.S).
pub const MAGIC_RAX: u64 = 0xcafe_0000_0000_0001;
pub const MAGIC_RBX: u64 = 0xcafe_0000_0000_0002;
pub const MAGIC_RDI: u64 = 0xcafe_0000_0000_0003;
pub const MAGIC_RSI: u64 = 0xcafe_0000_0000_0004;
pub const MAGIC_RDX: u64 = 0xcafe_0000_0000_0005;
pub const MAGIC_R10: u64 = 0xcafe_0000_0000_0006;
/// The unmapped canonical user VA used by the #PF-unmapped injection
/// (carried in rdi; cr2 must equal it).
pub const USER_UNMAPPED_VA: u64 = 0x0000_5555_0000_0000;
/// Kernel CS at fault time (GDT: code 0x08, data 0x10; CPL0 boot context).
pub const KERNEL_CS: u64 = 0x08;
/// First backtrace symbol (the fixed no-mangle driver frame; deeper Rust
/// frames may inline freely — prefix matching only, decision D-3).
pub const FRAME0_SYM: &str = "_m1r_fault_driver_frame";

/// One declared expected fault (mirrors kernel faultsuite::FAULT_IDS).
pub struct FaultCaseSpec {
    pub fault_id: &'static str,
    /// M1RFault kind tag: PF | GP | DE | BP.
    pub kind: &'static str,
    /// Error code value (printed as hex).
    pub ec: u64,
    /// Extra "  fault-kind: ..." line after #PF lines.
    pub pf_kind: Option<&'static str>,
    /// RIP must symbolize into this symbol (prefix `sym+`).
    pub rip_sym: &'static str,
    /// cr2 must equal rdi (pf.unmapped); unchecked elsewhere.
    pub cr2_eq_rdi: bool,
}

/// The frozen m1r-fault manifest.
pub struct FaultSpec {
    pub suite: &'static str,
    pub run: &'static str,
    pub cases: &'static [FaultCaseSpec],
}

pub fn m1r_fault_spec() -> FaultSpec {
    FaultSpec {
        suite: "m1r-fault",
        run: "core-up",
        cases: &[
            FaultCaseSpec {
                fault_id: "m1rfault.pf.unmapped",
                kind: "PF",
                ec: 0,
                pf_kind: Some("not-present"),
                rip_sym: "_m1rf_inject_pf_unmapped",
                cr2_eq_rdi: true,
            },
            FaultCaseSpec {
                fault_id: "m1rfault.pf.rodata",
                kind: "PF",
                ec: 3,
                pf_kind: Some("kernel-write-to-readonly"),
                rip_sym: "_m1rf_inject_pf_rodata",
                cr2_eq_rdi: false,
            },
            FaultCaseSpec {
                fault_id: "m1rfault.gp.noncanon",
                kind: "GP",
                ec: 0,
                pf_kind: None,
                rip_sym: "_m1rf_inject_gp_noncanon",
                cr2_eq_rdi: false,
            },
            FaultCaseSpec {
                fault_id: "m1rfault.de.div0",
                kind: "DE",
                ec: 0,
                pf_kind: None,
                rip_sym: "_m1rf_inject_de_div0",
                cr2_eq_rdi: false,
            },
            FaultCaseSpec {
                fault_id: "m1rfault.bp.int3",
                kind: "BP",
                ec: 0,
                pf_kind: None,
                rip_sym: "_m1rf_inject_bp_int3",
                cr2_eq_rdi: false,
            },
        ],
    }
}
