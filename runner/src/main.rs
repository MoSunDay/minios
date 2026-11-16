//! minios M0R host runner: static ELF gate + CORE-UP-GRUB smoke run + verdict.
//! Shell/Make only launches this binary; all judgement lives in Rust
//! (plans/linux26 ADR-0013, 40 §2).

mod elfcheck;
mod verdict;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const QEMU: &str = "qemu-system-x86_64";
const DEFAULT_DEADLINE_SECS: u64 = 120;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("check-elf") => cmd_check_elf(&args[1..]),
        Some("run-smoke") => cmd_run_smoke(&args[1..]),
        _ => {
            eprintln!(
                "usage: minios-runner <command>\n\
                 \x20 check-elf <kernel-elf>\n\
                 \x20 run-smoke --iso <iso> [--deadline-secs N] [--work-dir DIR]"
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

fn cmd_run_smoke(args: &[String]) -> i32 {
    let mut iso: Option<PathBuf> = None;
    let mut work_dir: Option<PathBuf> = None;
    let mut deadline = DEFAULT_DEADLINE_SECS;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--iso" => {
                i += 1;
                match args.get(i) {
                    Some(v) => iso = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("run-smoke: --iso needs a value");
                        return 2;
                    }
                }
            }
            "--work-dir" => {
                i += 1;
                match args.get(i) {
                    Some(v) => work_dir = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("run-smoke: --work-dir needs a value");
                        return 2;
                    }
                }
            }
            "--deadline-secs" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u64>().ok()) {
                    Some(v) => deadline = v,
                    None => {
                        eprintln!("run-smoke: --deadline-secs needs a number");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("run-smoke: unknown arg {other}");
                return 2;
            }
        }
        i += 1;
    }
    let Some(iso) = iso else {
        eprintln!("run-smoke: --iso <iso> is required");
        return 2;
    };
    if !iso.is_file() {
        eprintln!("run-smoke: iso not found: {}", iso.display());
        return 2;
    }

    let work = match work_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("run-smoke: cannot create work dir {}: {e}", dir.display());
                return 2;
            }
            dir
        }
        None => fresh_work_dir(),
    };
    let serial = work.join("serial.log");
    let argv = qemu_argv(&iso, &serial);
    println!("config_id=CORE-UP-GRUB argv={}", argv.join(" "));

    let log = std::fs::read_to_string(&serial).unwrap_or_default();
    if !log.is_empty() {
        eprintln!(
            "run-smoke: serial log already exists and is non-empty: {}",
            serial.display()
        );
        return 2;
    }

    let stderr_path = work.join("qemu.stderr");
    let stderr_file = std::fs::File::create(&stderr_path)
        .map_err(|e| {
            eprintln!("run-smoke: cannot create {}: {e}", stderr_path.display());
        })
        .ok();
    let mut cmd = Command::new(QEMU);
    cmd.args(&argv[1..]).stdout(Stdio::null());
    match stderr_file {
        Some(f) => {
            cmd.stderr(Stdio::from(f));
        }
        None => {
            cmd.stderr(Stdio::null());
        }
    }
    let child = cmd.spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("run-smoke: cannot spawn {QEMU}: {e}");
            return 2;
        }
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
                eprintln!("run-smoke: wait failed: {e}");
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
    match verdict::judge(&verdict::m0r_spec(), &outcome) {
        Ok(()) => {
            println!("verdict: PASS suite=m0r run=core-up config_id=CORE-UP-GRUB");
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

fn qemu_argv(iso: &Path, serial: &Path) -> Vec<String> {
    [
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
    .to_vec()
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
