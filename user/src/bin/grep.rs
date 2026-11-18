//! grep(1), xv6 grep.c: the Kernighan & Pike regular-expression matcher
//! (literal chars, '.', '*', '^', '$'); matching is line-based.

#![no_std]
#![no_main]

use minios_user::{args, io, sys};

fn match_here(re: &[u8], text: &[u8]) -> bool {
    if re.is_empty() {
        return true;
    }
    if re.len() > 1 && re[1] == b'*' {
        return match_star(re[0], &re[2..], text);
    }
    if re[0] == b'$' && re.len() == 1 {
        return text.is_empty();
    }
    if !text.is_empty() && (re[0] == b'.' || re[0] == text[0]) {
        return match_here(&re[1..], &text[1..]);
    }
    false
}

fn match_star(c: u8, re: &[u8], text: &[u8]) -> bool {
    let mut i = 0usize;
    loop {
        if match_here(re, &text[i.min(text.len())..]) {
            return true;
        }
        if i >= text.len() || (text[i] != c && c != b'.') {
            return false;
        }
        i += 1;
    }
}

/// True when `re` occurs anywhere in `text` (anchored only on ^).
fn rgrep(re: &[u8], text: &[u8]) -> bool {
    if !re.is_empty() && re[0] == b'^' {
        return match_here(&re[1..], text);
    }
    let mut i = 0usize;
    loop {
        if match_here(re, &text[i..]) {
            return true;
        }
        if i >= text.len() {
            return false;
        }
        i += 1;
    }
}

fn grep_fd(fd: i32, pattern: &[u8]) {
    let mut buf = [0u8; 1024];
    let mut len = 0usize;
    let mut eof = false;
    while !eof {
        let mut one = [0u8; 1];
        let c = match sys::read(fd, &mut one) {
            Ok(0) => {
                eof = true;
                b'\n' // EOF flushes any partial line
            }
            Ok(_) => one[0],
            Err(_) => break,
        };
        if len < buf.len() {
            buf[len] = c;
            len += 1;
        }
        if c == b'\n' {
            // The line content is buf[..len-1] (newline excluded).
            if len > 1 && rgrep(pattern, &buf[..len - 1]) {
                let _ = io::write_all(io::STDOUT, &buf[..len]);
            }
            len = 0;
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // SAFETY: _start passes the initial stack argv layout.
    let a = unsafe { args::from_stack(argc as usize, argv as *const u64) };
    if a.argc() <= 1 {
        let _ = io::eprint_str("usage: grep pattern [file...]\n");
        sys::exit(1);
    }
    let pattern = a.arg(1).unwrap_or(b"");
    if a.argc() <= 2 {
        grep_fd(0, pattern);
        sys::exit(0);
    }
    for i in 2..a.argc() {
        match a.arg(i) {
            Some(p) => match sys::open(p, abi::O_RDONLY) {
                Ok(fd) => {
                    grep_fd(fd as i32, pattern);
                    let _ = sys::close(fd as i32);
                }
                Err(_) => {
                    let _ = io::eprint_str("grep: cannot open ");
                    let _ = io::write_all(io::STDERR, p);
                    let _ = io::eprint_str("\n");
                }
            },
            None => continue,
        }
    }
    sys::exit(0)
}
