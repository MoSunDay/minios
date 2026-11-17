//! minios host runner: static ELF gate + CORE-UP-GRUB suite runs + verdict.
//! Shell/Make only launches this binary; all judgement lives in Rust
//! (plans/linux26 ADR-0013, 40 §2). M0R uses config CORE-UP-GRUB; the M1R
//! suites use CORE-UP-GRUB-R1, appending the frozen raw-IDE root drive
//! (40 §1.2) with byte-identical argv between the two M1R targets.

mod elfcheck;
mod verdict;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const QEMU: &str = "qemu-system-x86_64";
const DEFAULT_DEADLINE_SECS: u64 = 120;
pub const CONFIG_ID_M0R: &str = "CORE-UP-GRUB";
pub const CONFIG_ID_M1R: &str = "CORE-UP-GRUB-R1";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("check-elf") => cmd_check_elf(&args[1..]),
        Some("run-smoke") => cmd_run_suite(Suite::M0r, "run-smoke", &args[1..]),
        Some("run-m1r-suite") => cmd_run_suite(Suite::M1rSuite, "run-m1r-suite", &args[1..]),
        Some("run-m1r-fault") => cmd_run_suite(Suite::M1rFault, "run-m1r-fault", &args[1..]),
        _ => {
            eprintln!(
                "usage: minios-runner <command>\n\
                 \x20 check-elf <kernel-elf>\n\
                 \x20 run-smoke --iso <iso> [--deadline-secs N] [--work-dir DIR]\n\
                 \x20 run-m1r-suite --iso <iso> --root <root.raw> [--deadline-secs N] [--work-dir DIR]\n\
                 \x20 run-m1r-fault --iso <iso> --root <root.raw> [--deadline-secs N] [--work-dir DIR]"
            );
            2
        }
    };
    std::process::exit(code);
}

fn cmd_check_elf(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("check-elf: missing <kernel-elf>");
        return 2;
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("check-elf: cannot read {path}: {e}");
            return 2;
        }
    };
    match elfcheck::check_elf(&bytes) {
        Ok(report) => {
            println!("check-elf: PASS {}", path);
            println!(
                "  entry={:#x} multiboot@{:#x} magic={:#x} flags={} checksum={:#x} loads={}",
                report.entry,
                report.multiboot.offset,
                report.multiboot.magic,
                report.multiboot.flags,
                report.multiboot.checksum,
                report.loads.len()
            );
            for l in &report.loads {
                println!(
                    "  PT_LOAD off={:#x} filesz={:#x} vaddr={:#x} paddr={:#x}",
                    l.p_offset, l.p_filesz, l.p_vaddr, l.p_paddr
                );
            }
            0
        }
        Err(reason) => {
            eprintln!("check-elf: FAIL {path}: {reason}");
            1
        }
    }
}

/// The three runnable suites and their frozen identities.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Suite {
    M0r,
    M1rSuite,
    M1rFault,
}

impl Suite {
    fn needs_root(&self) -> bool {
        !matches!(self, Suite::M0r)
    }
    fn config_id(&self) -> &'static str {
        match self {
            Suite::M0r => CONFIG_ID_M0R,
            _ => CONFIG_ID_M1R,
        }
    }
    fn suite_label(&self) -> &'static str {
        match self {
            Suite::M0r => "m0r",
            Suite::M1rSuite => "m1r",
            Suite::M1rFault => "m1r-fault",
        }
    }
}

fn cmd_run_suite(suite: Suite, cmd: &str, args: &[String]) -> i32 {
    let mut iso: Option<PathBuf> = None;
    let mut root: Option<PathBuf> = None;
    let mut work_dir: Option<PathBuf> = None;
    let mut deadline = DEFAULT_DEADLINE_SECS;
    let mut i = 0;
    let bad = |what: &str| -> i32 {
        eprintln!("{cmd}: {what}");
        2
    };
    while i < args.len() {
        let take_value = |i: &mut usize, what: &str| -> Option<String> {
            *i += 1;
            match args.get(*i) {
                Some(v) => Some(v.clone()),
                None => {
                    eprintln!("{cmd}: {what} needs a value");
                    None
                }
            }
        };
        match args[i].as_str() {
            "--iso" => match take_value(&mut i, "--iso") {
                Some(v) => iso = Some(PathBuf::from(v)),
                None => return 2,
            },
            "--root" => match take_value(&mut i, "--root") {
                Some(v) => root = Some(PathBuf::from(v)),
                None => return 2,
            },
            "--work-dir" => match take_value(&mut i, "--work-dir") {
                Some(v) => work_dir = Some(PathBuf::from(v)),
                None => return 2,
            },
            "--deadline-secs" => match take_value(&mut i, "--deadline-secs") {
                Some(v) => match v.parse::<u64>() {
                    Ok(v) => deadline = v,
                    Err(_) => return bad("--deadline-secs needs a number"),
                },
                None => return 2,
            },
            other => {
                eprintln!("{cmd}: unknown arg {other}");
                return 2;
            }
        }
        i += 1;
    }
    let Some(iso) = iso else {
        return bad("--iso <iso> is required");
    };
    if !iso.is_file() {
        return bad(&format!("iso not found: {}", iso.display()));
    }
    let root = if suite.needs_root() {
        let Some(root) = root else {
            return bad("--root <root.raw> is required for M1R suites");
        };
        if !root.is_file() {
            return bad(&format!("root disk not found: {}", root.display()));
        }
        root
    } else {
        PathBuf::new()
    };

    let work = match work_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return bad(&format!("cannot create work dir {}: {e}", dir.display()));
            }
            dir
        }
        None => fresh_work_dir(),
    };
    let serial = work.join("serial.log");
    let argv = qemu_argv(
        &iso,
        &serial,
        if suite.needs_root() {
            Some(&root)
        } else {
            None
        },
    );
    println!("config_id={} argv={}", suite.config_id(), argv.join(" "));

    let log = std::fs::read_to_string(&serial).unwrap_or_default();
    if !log.is_empty() {
        return bad(&format!(
            "serial log already exists and is non-empty: {}",
            serial.display()
        ));
    }

    let stderr_path = work.join("qemu.stderr");
    let stderr_file = std::fs::File::create(&stderr_path)
        .map_err(|e| eprintln!("{cmd}: cannot create {}: {e}", stderr_path.display()))
        .ok();
    let mut cmdq = Command::new(QEMU);
    cmdq.args(&argv[1..]).stdout(Stdio::null());
    match stderr_file {
        Some(f) => {
            cmdq.stderr(Stdio::from(f));
        }
        None => {
            cmdq.stderr(Stdio::null());
        }
    }
    let mut child = match cmdq.spawn() {
        Ok(c) => c,
        Err(e) => return bad(&format!("cannot spawn {QEMU}: {e}")),
    };

    let started = Instant::now();
    let deadline_at = Duration::from_secs(deadline);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= deadline_at {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("{cmd}: wait failed: {e}");
                let _ = child.kill();
                return 2;
            }
        }
    };

    let serial_log = std::fs::read_to_string(&serial).unwrap_or_default();
    print!("--- serial ---\n{serial_log}");
    let exit_code = status.map(|s| s.code().unwrap_or(-1));
    println!(
        "exit_code={:?} timed_out={timed_out} serial_log={}",
        exit_code,
        serial.display()
    );

    let outcome = verdict::RunOutcome {
        serial_log,
        exit_code,
        timed_out,
    };
    let judged = match suite {
        Suite::M0r => verdict::judge(&verdict::m0r_spec(), &outcome).map(|_| 33),
        Suite::M1rSuite => verdict::judge(&verdict::m1r_suite_spec(), &outcome).map(|_| 33),
        Suite::M1rFault => verdict::judge_fault(&verdict::m1r_fault_spec(), &outcome)
            .map(|_| verdict::EXPECTED_FAULT_EXIT_CODE),
    };
    match judged {
        Ok(expected_exit) => {
            println!(
                "verdict: PASS suite={} run=core-up config_id={}",
                suite.suite_label(),
                suite.config_id()
            );
            debug_assert_eq!(expected_exit, exit_code.unwrap_or(-1));
            0
        }
        Err(reasons) => {
            for r in &reasons {
                eprintln!("verdict: FAIL {r}");
            }
            1
        }
    }
}

/// Frozen QEMU argv (CORE-UP-GRUB base, 40 §1). The M1R suites append the
/// frozen raw-IDE root drive (CORE-UP-GRUB-R1, 40 §1.2) — every parameter
/// explicit, never a shared writable template.
fn qemu_argv(iso: &Path, serial: &Path, root: Option<&Path>) -> Vec<String> {
    let mut argv = [
        QEMU.to_string(),
        "-machine".into(),
        "pc-i440fx-6.2,accel=tcg".into(),
        "-cpu".into(),
        "qemu64".into(),
        "-m".into(),
        "256M".into(),
        "-smp".into(),
        "1".into(),
        "-display".into(),
        "none".into(),
        "-monitor".into(),
        "none".into(),
        "-serial".into(),
        format!("file:{}", serial.display()),
        "-nic".into(),
        "none".into(),
        "-no-reboot".into(),
        "-device".into(),
        "isa-debug-exit,iobase=0xf4,iosize=0x04".into(),
        "-cdrom".into(),
        iso.display().to_string(),
    ]
    .to_vec();
    if let Some(root) = root {
        argv.push("-drive".into());
        argv.push(format!(
            "file={},format=raw,if=ide,index=0,cache=writeback,snapshot=off",
            root.display()
        ));
    }
    argv
}

fn fresh_work_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!("minios-m0r-{}-{}", std::process::id(), nanos));
    let _ = std::fs::create_dir_all(&base);
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m0r_argv_is_frozen() {
        let argv = qemu_argv(
            Path::new("/iso/m0r.iso"),
            Path::new("/run/serial.log"),
            None,
        );
        let want: Vec<&str> = [
            "qemu-system-x86_64",
            "-machine",
            "pc-i440fx-6.2,accel=tcg",
            "-cpu",
            "qemu64",
            "-m",
            "256M",
            "-smp",
            "1",
            "-display",
            "none",
            "-monitor",
            "none",
            "-serial",
            "file:/run/serial.log",
            "-nic",
            "none",
            "-no-reboot",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-cdrom",
            "/iso/m0r.iso",
        ]
        .to_vec();
        assert_eq!(argv, want);
    }

    #[test]
    fn m1r_argv_appends_frozen_ide_drive() {
        let suite = qemu_argv(
            Path::new("/iso/m1r.iso"),
            Path::new("/run/serial.log"),
            Some(Path::new("/run/root.raw")),
        );
        let fault = qemu_argv(
            Path::new("/iso/m1r.iso"),
            Path::new("/run/serial.log"),
            Some(Path::new("/run/root.raw")),
        );
        assert_eq!(suite[suite.len() - 2], "-drive");
        assert_eq!(
            suite.last().unwrap(),
            "file=/run/root.raw,format=raw,if=ide,index=0,cache=writeback,snapshot=off"
        );
        // The two M1R targets share one argv byte-for-byte (T5 gate).
        assert_eq!(suite, fault);
        // And the M0R argv is a strict prefix of it.
        let m0r = qemu_argv(
            Path::new("/iso/m1r.iso"),
            Path::new("/run/serial.log"),
            None,
        );
        assert_eq!(&suite[..m0r.len()], &m0r[..]);
        assert_eq!(suite.len(), m0r.len() + 2);
    }
}
