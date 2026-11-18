//! `demo-full`: interactive BootMode::Full demo (cmdline `full` boots
//! /init -> sh on the serial console). Serial is `-serial stdio`, so this
//! process owns QEMU's stdin/stdout and drives the sh session itself
//! (echo/ls/CTRL-D). The marker/feed table and the cursor-based byte
//! search are pure and unit-tested; only the spawn/poll plumbing is
//! impure. QEMU is always killed by us — the verdict never relies on
//! isa-debug-exit.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const CMD: &str = "demo-full";
pub const CONFIG_ID_FULL: &str = "CORE-UP-GRUB-FULL";
pub const MARKER_PASS: &str = "MINIOS_X86_64_DEMO_FULL_PASS_V1";
pub const MARKER_FAIL: &str = "MINIOS_X86_64_DEMO_FULL_FAIL_V1";
const SERIAL_DUMP_BYTES: usize = 2048;
const PER_MARKER_DEADLINE: Duration = Duration::from_secs(45);
const TOTAL_DEADLINE: Duration = Duration::from_secs(150);

/// One scripted interaction: bytes fed to sh's stdin (empty = observe
/// only), then the markers that must appear on serial, in order.
pub struct Step {
    pub label: &'static str,
    pub feed: &'static [u8],
    pub markers: &'static [&'static [u8]],
}

/// One-line builder so the frozen table below stays compact.
const fn step(label: &'static str, feed: &'static [u8], markers: &'static [&'static [u8]]) -> Step {
    Step {
        label,
        feed,
        markers,
    }
}

/// The frozen 6-step session. Step 6's `init: starting sh` search starts
/// after step 5's marker end, so it matches the *second* occurrence (the
/// restart), never the boot-time one.
pub const SESSION: [Step; 6] = [
    step("boot-init", b"", &[b"init: starting sh"]),
    step("sh-prompt", b"", &[b"$ "]),
    step(
        "echo",
        b"echo minios-full-boot-demo\n",
        &[b"minios-full-boot-demo"],
    ),
    // mkfs embed order: /console (mknod'd by init post-boot) is last.
    step("ls", b"ls\n", &[b"usertests", b"console"]),
    step("ctrl-d", b"\x04", &[b"init: sh exited, restarting"]),
    step("sh-restart", b"", &[b"init: starting sh"]),
];

/// Byte-substring search that only matches at or after `from` (the
/// session cursor: each marker hunt starts after the previous end).
pub fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

/// Advance over `markers` in order from cursor `from`: each search
/// starts after the previous marker's end. Ok = new cursor just past the
/// last marker; Err = `step=<label> marker=<missing>`. The live loop
/// calls this with one-marker slices; replay drives whole steps.
pub fn advance(
    label: &str,
    markers: &[&[u8]],
    buf: &[u8],
    mut from: usize,
) -> Result<usize, String> {
    for m in markers {
        match find_from(buf, m, from) {
            Some(i) => from = i + m.len(),
            None => {
                let marker = String::from_utf8_lossy(m);
                return Err(format!("step={label} marker={marker}"));
            }
        }
    }
    Ok(from)
}

/// Replay the whole frozen session against a complete serial buffer —
/// the unit-test driver for the exact table the live loop walks.
pub fn replay(buf: &[u8]) -> Result<(), String> {
    let mut cursor = 0;
    for step in SESSION.iter() {
        cursor = advance(step.label, step.markers, buf, cursor)?;
    }
    Ok(())
}

/// Frozen demo argv: the M1R suite argv (CORE-UP-GRUB-R1 + raw IDE root
/// drive + isa-debug-exit) with exactly ONE element changed — `-serial
/// stdio` in place of `file:<serial>` — so the runner owns stdio. The
/// frozen qemu_argv stays untouched; the delta is pinned by unit test.
pub fn demo_argv(iso: &Path, root: &Path) -> Vec<String> {
    let mut argv = crate::qemu_argv(iso, Path::new("serial.log"), Some(root));
    let idx = argv
        .iter()
        .position(|a| a == "-serial")
        .expect("frozen argv has -serial");
    argv[idx + 1] = "stdio".into();
    argv
}

/// Reader thread: append QEMU stdout to the shared buffer and mirror it
/// incrementally into the serial log (evidence survives a hard kill).
fn spawn_reader(
    mut out: Box<dyn Read + Send>,
    shared: Arc<Mutex<Vec<u8>>>,
    mut log: std::fs::File,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut buf = shared.lock().unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let _ = log.write_all(&chunk[..n]);
                    let _ = log.flush();
                }
            }
        }
    })
}

/// `<why> step=<label> marker=<marker>` — the one FAIL-reason shape.
fn miss(step: &Step, marker: &[u8], why: &str) -> String {
    let m = String::from_utf8_lossy(marker);
    format!("{why} step={} marker={m}", step.label)
}

/// Thin cmd: parse args, spawn QEMU, walk the frozen session, verdict.
pub fn cmd_demo_full(args: &[String]) -> i32 {
    let mut iso: Option<PathBuf> = None;
    let mut root: Option<PathBuf> = None;
    let mut work_dir: Option<PathBuf> = None;
    let mut i = 0;
    let bad = |what: &str| -> i32 {
        eprintln!("{CMD}: {what}");
        2
    };
    while i < args.len() {
        let flag = args[i].as_str();
        let Some(v) = args.get(i + 1) else {
            eprintln!("{CMD}: {flag} needs a value");
            return 2;
        };
        match flag {
            "--iso" => iso = Some(PathBuf::from(v)),
            "--root" => root = Some(PathBuf::from(v)),
            "--work-dir" => work_dir = Some(PathBuf::from(v)),
            other => {
                eprintln!("{CMD}: unknown arg {other}");
                return 2;
            }
        }
        i += 2;
    }
    let (Some(iso), Some(root), Some(work)) = (iso, root, work_dir) else {
        return bad("--iso/--root/--work-dir are all required");
    };
    if !iso.is_file() {
        return bad(&format!("iso not found: {}", iso.display()));
    }
    if !root.is_file() {
        return bad(&format!("root disk not found: {}", root.display()));
    }
    if let Err(e) = std::fs::create_dir_all(&work) {
        return bad(&format!("cannot create work dir {}: {e}", work.display()));
    }
    let serial_path = work.join("serial.log");

    let argv = demo_argv(&iso, &root);
    println!("config_id={CONFIG_ID_FULL} argv={}", argv.join(" "));
    // Pin the exact QEMU build alongside every run's evidence (40 §1.1).
    let qver = match Command::new(crate::QEMU).arg("--version").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim_end().to_string(),
        Ok(o) => format!("qemu --version exited with status {}", o.status),
        Err(e) => format!("cannot run {} --version: {e}", crate::QEMU),
    };
    let _ = std::fs::write(work.join("qemu.version"), format!("{qver}\n"));
    let stderr_file = match std::fs::File::create(work.join("qemu.stderr")) {
        Ok(f) => f,
        Err(e) => return bad(&format!("cannot create qemu.stderr: {e}")),
    };
    let mut child = match Command::new(crate::QEMU)
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file))
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return bad(&format!("cannot spawn {}: {e}", crate::QEMU)),
    };
    let serial_log = match std::fs::File::create(&serial_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = child.kill();
            return bad(&format!("cannot create {}: {e}", serial_path.display()));
        }
    };
    let shared: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let out = child.stdout.take();
    let mut reader = out.map(|o| spawn_reader(Box::new(o), Arc::clone(&shared), serial_log));
    let mut stdin: Option<ChildStdin> = child.stdin.take();

    let started = Instant::now();
    let mut cursor = 0usize;
    let mut reason: Option<String> = None;
    'session: for step in SESSION.iter() {
        if !step.feed.is_empty() {
            let fed = stdin
                .as_mut()
                .map(|s| s.write_all(step.feed).and_then(|_| s.flush()));
            if !matches!(fed, Some(Ok(()))) {
                reason = Some(match fed {
                    Some(Err(e)) => format!("feed-failed step={}: {e}", step.label),
                    _ => format!("feed-failed step={}: stdin closed", step.label),
                });
                break;
            }
        }
        for marker in step.markers {
            let marker_deadline = Instant::now() + PER_MARKER_DEADLINE;
            let wait = std::slice::from_ref(marker);
            loop {
                if let Ok(next) = advance(step.label, wait, &shared.lock().unwrap(), cursor) {
                    cursor = next;
                    break;
                }
                if Instant::now() >= marker_deadline {
                    reason = Some(miss(step, marker, "timeout"));
                    break 'session;
                }
                if started.elapsed() >= TOTAL_DEADLINE {
                    reason = Some(miss(step, marker, "total-deadline"));
                    break 'session;
                }
                match child.try_wait() {
                    // QEMU is gone: drain (join) the reader once, re-check
                    // this marker against the final buffer. A hit still
                    // advances the session; a miss reports qemu-exited.
                    Ok(Some(status)) => {
                        if let Some(h) = reader.take() {
                            let _ = h.join();
                        }
                        if let Ok(next) = advance(step.label, wait, &shared.lock().unwrap(), cursor)
                        {
                            cursor = next;
                            break;
                        }
                        reason = Some(format!(
                            "{} code={}",
                            miss(step, marker, "qemu-exited"),
                            status.code().unwrap_or(-1)
                        ));
                        break 'session;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        reason = Some(format!("wait-failed step={}: {e}", step.label));
                        break 'session;
                    }
                }
                sleep(Duration::from_millis(50));
            }
        }
    }

    // Common teardown: stop QEMU, drain the reader, persist final serial.
    let _ = child.kill();
    let _ = child.wait();
    drop(stdin);
    if let Some(h) = reader.take() {
        let _ = h.join();
    }
    let buf = shared.lock().unwrap().clone();
    let _ = std::fs::write(&serial_path, &buf);

    // Final gate on the captured transcript: the whole frozen session
    // must replay over the complete buffer (the live cursor walk and the
    // whole-buffer replay agree, or something is wrong).
    match reason {
        None if replay(&buf).is_ok() => {
            println!("{MARKER_PASS} markers=6 session=echo/ls/ctrl-d");
            0
        }
        None => {
            eprintln!("{MARKER_FAIL} reason=session-replay-mismatch");
            dump_tail(&buf);
            1
        }
        Some(r) => {
            eprintln!("{MARKER_FAIL} reason={r}");
            dump_tail(&buf);
            1
        }
    }
}

/// Last SERIAL_DUMP_BYTES bytes of serial to stdout (FAIL evidence).
fn dump_tail(buf: &[u8]) {
    let tail = buf.len().saturating_sub(SERIAL_DUMP_BYTES);
    println!("--- serial tail (last {SERIAL_DUMP_BYTES} bytes) ---");
    println!("{}", String::from_utf8_lossy(&buf[tail..]));
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Canned full-boot serial transcript (CRLF like the real console;
    /// input is not echoed, so only program output appears).
    fn canned(exit_restart: bool, second_start: bool) -> Vec<u8> {
        let mut b: Vec<u8> = b"minios x86_64 full boot\r\n\
             init: starting sh\r\n\
             $ minios-full-boot-demo\r\n\
             $ cat        echo      ls        usertests console\r\n\
             $ "
        .to_vec();
        if exit_restart {
            b.extend_from_slice(b"init: sh exited, restarting\r\n");
        }
        if second_start {
            b.extend_from_slice(b"init: starting sh\r\n$ ");
        }
        b
    }

    #[test]
    fn demo_argv_is_frozen_base_with_only_serial_swapped() {
        let iso = Path::new("/iso/minios-full.iso");
        let root = Path::new("/run/root.raw");
        let base = crate::qemu_argv(iso, Path::new("/run/serial.log"), Some(root));
        let demo = demo_argv(iso, root);
        assert_eq!(demo.len(), base.len());
        let diffs: Vec<usize> = (0..base.len()).filter(|&i| base[i] != demo[i]).collect();
        assert_eq!(diffs.len(), 1, "exactly one element changes");
        let i = diffs[0];
        assert_eq!(&base[i - 1..=i], &["-serial", "file:/run/serial.log"]);
        assert_eq!(&demo[i - 1..=i], &["-serial", "stdio"]);
        // Root drive rides exactly like the M1R suites; debug-exit kept.
        assert_eq!(
            demo.last().unwrap(),
            "file=/run/root.raw,format=raw,if=ide,index=0,cache=writeback,snapshot=off"
        );
        assert!(demo
            .iter()
            .any(|a| a == "isa-debug-exit,iobase=0xf4,iosize=0x04"));
    }

    #[test]
    fn replay_passes_all_steps_in_order() {
        assert_eq!(replay(&canned(true, true)), Ok(()));
    }

    #[test]
    fn replay_fails_at_ctrl_d_when_marker5_missing() {
        let err = replay(&canned(false, true)).unwrap_err();
        assert!(
            err.contains("step=ctrl-d") && err.contains("init: sh exited, restarting"),
            "{err}"
        );
    }

    #[test]
    fn replay_requires_the_second_start_marker() {
        // Restart marker present but no second `init: starting sh`: the
        // step-6 search starts after the restart marker, so the boot-time
        // occurrence must NOT satisfy it.
        let err = replay(&canned(true, false)).unwrap_err();
        assert!(
            err.contains("step=sh-restart") && err.contains("marker=init: starting sh"),
            "{err}"
        );
    }

    #[test]
    fn advance_enforces_marker_order_within_a_step() {
        let step = &SESSION[3]; // ls: usertests, then console (embed order)
        assert!(advance(step.label, step.markers, b"$ console usertests\r\n", 0).is_err());
        assert!(advance(step.label, step.markers, b"$ usertests console\r\n", 0).is_ok());
    }
}
