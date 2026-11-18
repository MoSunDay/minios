//! ls(1), xv6 ls.c: list files; directories raw-read their dirents and
//! stat each entry name (resolved against the process cwd, like xv6).

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

const DIRSIZ: usize = 14;

/// fmtname: the last path element, space-padded to DIRSIZ, returned as a
/// fixed-size buffer (xv6 pads in place and printf %s stops at the pad).
fn fmtname(path: &[u8]) -> [u8; DIRSIZ + 1] {
    let mut start = 0usize;
    for (i, &b) in path.iter().enumerate() {
        if b == b'/' {
            start = i + 1;
        }
    }
    let name = &path[start.min(path.len())..];
    let mut out = [b' '; DIRSIZ + 1];
    let n = name.len().min(DIRSIZ);
    out[..n].copy_from_slice(&name[..n]);
    out[DIRSIZ] = 0;
    out
}

fn ls(path: &[u8]) {
    let mut st = abi::Stat::default();
    let mut pbuf = [0u8; 128];
    let plen = path.len().min(127);
    pbuf[..plen].copy_from_slice(path);
    if sys::stat(&pbuf[..plen + 1], &mut st).is_err() {
        let _ = io::eprint_str("ls: cannot stat ");
        let _ = io::write_all(io::STDERR, path);
        let _ = io::eprint_str("\n");
        return;
    }
    let fmt = fmtname(path);
    match st.type_ {
        abi::Stat::T_FILE => print_entry(&fmt, &st),
        abi::Stat::T_DIR => {
            let fd = match sys::open(path, abi::O_RDONLY) {
                Ok(fd) => fd as i32,
                Err(_) => {
                    let _ = io::eprint_str("ls: cannot open ");
                    let _ = io::write_all(io::STDERR, path);
                    let _ = io::eprint_str("\n");
                    return;
                }
            };
            // Raw directory walk: 16-byte dirents (u16 inum + name).
            let mut buf = [0u8; 512];
            'dir: loop {
                let base = match sys::read(fd, &mut buf) {
                    Ok(n) => n,
                    Err(_) => break 'dir,
                };
                let mut off = 0usize;
                while off + 16 <= base {
                    let inum = u16::from_le_bytes([buf[off], buf[off + 1]]);
                    let mut name = [0u8; DIRSIZ + 1];
                    name[..DIRSIZ].copy_from_slice(&buf[off + 2..off + 16]);
                    if inum != 0 && name[0] != 0 {
                        let mut cst = abi::Stat::default();
                        if sys::stat(&name, &mut cst).is_ok() {
                            print_entry(
                                &fmtname(&name[..name.iter().position(|&b| b == 0).unwrap_or(0)]),
                                &cst,
                            );
                        } else {
                            let _ = io::eprint_str("ls: cannot stat ");
                            let _ = io::write_all(io::STDERR, &name);
                            let _ = io::eprint_str("\n");
                        }
                    }
                    off += 16;
                }
                if base < 512 {
                    break 'dir;
                }
            }
            let _ = sys::close(fd);
        }
        _ => print_entry(&fmt, &st),
    }
}

fn print_entry(name: &[u8; DIRSIZ + 1], st: &abi::Stat) {
    let _ = io::print_uint(st.ino as u64);
    let _ = io::print_str(" ");
    match st.type_ {
        abi::Stat::T_DIR => {
            let _ = io::print_str("d ");
        }
        abi::Stat::T_DEV => {
            let _ = io::print_str("d ");
        }
        _ => {
            let _ = io::print_str("- ");
        }
    }
    let _ = io::print_uint(st.nlink as i64 as u64);
    let _ = io::print_str(" ");
    let _ = io::print_uint(st.size as u64);
    let _ = io::print_str(" ");
    let _ = io::write_all(io::STDOUT, name);
    let _ = io::print_str("\n");
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() < 2 {
        ls(b".\0");
    } else {
        for i in 1..a.argc() {
            if let Some(p) = a.arg(i) {
                ls(p);
            }
        }
    }
    sys::exit(0)
}
