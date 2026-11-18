//! sh(1), xv6 sh.c: the shell. Recursive-descent parser (line `;`, pipe
//! `|`, parens, redirection `< > >>`, background `&`) over an enum AST,
//! then fork/exec per node. `cd` runs in the shell itself. EOF exits.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;
use minios_user::{io, sys};

const MAXARGS: usize = 10;
const BUFSZ: usize = 100;

/// The command tree (xv6's tagged structs as one enum; data only).
enum Cmd {
    Exec {
        argv: Vec<Vec<u8>>,
    },
    Redir {
        fd: i32,
        path: Vec<u8>,
        append: bool,
        next: Box<Cmd>,
    },
    Pipe {
        left: Box<Cmd>,
        right: Box<Cmd>,
    },
    List {
        left: Box<Cmd>,
        right: Option<Box<Cmd>>,
    },
    Back {
        next: Box<Cmd>,
    },
}

/// Parser state: the whitespace-split line, token cursor.
struct Ps {
    toks: Vec<Vec<u8>>,
    pos: usize,
}

impl Ps {
    fn peek(&self) -> Option<&[u8]> {
        self.toks.get(self.pos).map(|v| v.as_slice())
    }
    fn next(&mut self) -> Option<Vec<u8>> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
}

fn panic_exit(why: &str) -> ! {
    let _ = io::eprint_str("panic: ");
    let _ = io::eprint_str(why);
    let _ = io::eprint_str("\n");
    sys::exit(2)
}

/// Split the raw line into tokens; `;`, `|`, `(`, `)` stand alone, `>>`
/// folds into one token.
fn tokenize(line: &[u8]) -> Vec<Vec<u8>> {
    let mut toks = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let esc = |t: &mut Vec<u8>, c: u8| {
        t.push(c);
    };
    let mut i = 0usize;
    while i < line.len() {
        let c = line[i];
        if c == b' ' || c == b'\t' || c == b'\n' {
            if !cur.is_empty() {
                toks.push(core::mem::take(&mut cur));
            }
        } else if c == b';' || c == b'|' || c == b'(' || c == b')' || c == b'&' {
            if !cur.is_empty() {
                toks.push(core::mem::take(&mut cur));
            }
            let mut t = Vec::new();
            esc(&mut t, c);
            toks.push(t);
        } else if c == b'>' && i + 1 < line.len() && line[i + 1] == b'>' {
            if !cur.is_empty() {
                toks.push(core::mem::take(&mut cur));
            }
            let mut t = Vec::new();
            esc(&mut t, c);
            esc(&mut t, line[i + 1]);
            toks.push(t);
            i += 1;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

fn parse_exec(ps: &mut Ps) -> Cmd {
    let mut argv: Vec<Vec<u8>> = Vec::new();
    while let Some(t) = ps.peek() {
        if t == b";" || t == b"|" || t == b")" || t == b"&" {
            break;
        }
        // Redirections bind to this exec.
        if t == b"<" {
            ps.next();
            match ps.next() {
                Some(p) => {
                    return redir_wrap(Cmd::Exec { argv }, 0, p, false, ps);
                }
                None => panic_exit("syntax"),
            }
        }
        if t == b">" || t == b">>" {
            let append = t == b">>";
            ps.next();
            match ps.next() {
                Some(p) => {
                    return redir_wrap(Cmd::Exec { argv }, 1, p, append, ps);
                }
                None => panic_exit("syntax"),
            }
        }
        if let Some(tok) = ps.next() {
            if argv.len() == MAXARGS {
                panic_exit("too many args");
            }
            argv.push(tok);
        }
    }
    if argv.is_empty() {
        panic_exit("syntax");
    }
    Cmd::Exec { argv }
}

fn redir_wrap(next: Cmd, fd: i32, path: Vec<u8>, append: bool, ps: &mut Ps) -> Cmd {
    let inner = parse_redirs(next, ps);
    Cmd::Redir {
        fd,
        path,
        append,
        next: Box::new(inner),
    }
}

fn parse_redirs(mut node: Cmd, ps: &mut Ps) -> Cmd {
    loop {
        match ps.peek() {
            Some(b"<") => {
                ps.next();
                match ps.next() {
                    Some(p) => {
                        node = Cmd::Redir {
                            fd: 0,
                            path: p,
                            append: false,
                            next: Box::new(node),
                        };
                    }
                    None => panic_exit("syntax"),
                }
            }
            Some(b">") | Some(b">>") => {
                let append = ps.peek() == Some(b">>");
                ps.next();
                match ps.next() {
                    Some(p) => {
                        node = Cmd::Redir {
                            fd: 1,
                            path: p,
                            append,
                            next: Box::new(node),
                        };
                    }
                    None => panic_exit("syntax"),
                }
            }
            _ => return node,
        }
    }
}

fn parse_block(ps: &mut Ps) -> Cmd {
    // `(` already consumed; parse a line, expect `)`, then redirections.
    let inner = parse_line(ps);
    if ps.peek() != Some(b")") {
        panic_exit("syntax");
    }
    ps.next();
    parse_redirs(inner, ps)
}

fn parse_pipe(ps: &mut Ps) -> Cmd {
    let mut left = if ps.peek() == Some(b"(") {
        ps.next();
        parse_block(ps)
    } else {
        parse_exec(ps)
    };
    while ps.peek() == Some(b"|") {
        ps.next();
        let right = if ps.peek() == Some(b"(") {
            ps.next();
            parse_block(ps)
        } else {
            parse_exec(ps)
        };
        left = Cmd::Pipe {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    left
}

fn parse_line(ps: &mut Ps) -> Cmd {
    let mut node = parse_pipe(ps);
    if ps.peek() == Some(b"&") {
        ps.next();
        node = Cmd::Back {
            next: Box::new(node),
        };
    }
    if ps.peek() == Some(b";") {
        ps.next();
        let right = parse_line(ps);
        return Cmd::List {
            left: Box::new(node),
            right: Some(Box::new(right)),
        };
    }
    Cmd::List {
        left: Box::new(node),
        right: None,
    }
}

/// NUL-terminate argv bytes and exec; never returns on success.
fn run_exec(argv: &[Vec<u8>]) -> ! {
    let mut cpath: Vec<u8> = argv[0].clone();
    cpath.push(0);
    let mut ptrs: Vec<*const u8> = Vec::new();
    for a in argv {
        let mut ca = a.clone();
        ca.push(0);
        // Leak intentionally: this image is about to be replaced (or the
        // child exits on failure), like xv6's in-place nulterminate.
        let b = Box::leak(ca.into_boxed_slice());
        ptrs.push(b.as_ptr());
    }
    ptrs.push(ptr::null());
    match sys::execve(&cpath, ptrs.as_ptr() as *const usize, ptr::null()) {
        _ => {
            let _ = io::eprint_str("exec ");
            let _ = io::write_all(io::STDERR, &cpath[..cpath.len() - 1]);
            let _ = io::eprint_str(" failed\n");
            sys::exit(0)
        }
    }
}

fn open_redir(fd: i32, path: &[u8], append: bool) {
    let mut p: Vec<u8> = path.to_vec();
    p.push(0);
    let mut flags = if append {
        abi::O_WRONLY | abi::O_CREAT
    } else {
        abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC
    };
    if fd == 0 {
        flags = abi::O_RDONLY;
    } else if append {
        flags |= abi::O_APPEND;
    }
    match sys::open(&p, flags) {
        Ok(_) => {
            // The caller closed `fd` first, so open() landed exactly on
            // it (lowest-free descriptor): the classic sh.c:84-89 dance.
        }
        Err(_) => {
            let _ = io::eprint_str("open ");
            let _ = io::write_all(io::STDERR, path);
            let _ = io::eprint_str(" failed\n");
            sys::exit(0)
        }
    }
}

fn run_cmd(cmd: &Cmd) -> ! {
    match cmd {
        Cmd::Exec { argv } => run_exec(argv),
        Cmd::Redir {
            fd,
            path,
            append,
            next,
        } => {
            let _ = sys::close(*fd);
            open_redir(*fd, path, *append);
            run_cmd(next)
        }
        Cmd::Pipe { left, right } => {
            let mut fds = [0i32; 2];
            if sys::pipe(&mut fds).is_err() {
                panic_exit("pipe");
            }
            match sys::fork() {
                Ok(0) => {
                    let _ = sys::close(1);
                    if sys::dup(fds[1]).is_err() {
                        panic_exit("dup");
                    }
                    let _ = sys::close(fds[0]);
                    let _ = sys::close(fds[1]);
                    run_cmd(left)
                }
                _ => {}
            }
            match sys::fork() {
                Ok(0) => {
                    let _ = sys::close(0);
                    if sys::dup(fds[0]).is_err() {
                        panic_exit("dup");
                    }
                    let _ = sys::close(fds[0]);
                    let _ = sys::close(fds[1]);
                    run_cmd(right)
                }
                _ => {}
            }
            let _ = sys::close(fds[0]);
            let _ = sys::close(fds[1]);
            let _ = sys::wait4(-1, ptr::null_mut());
            let _ = sys::wait4(-1, ptr::null_mut());
            sys::exit(0)
        }
        Cmd::List { left, right } => {
            match sys::fork() {
                Ok(0) => run_cmd(left),
                _ => {}
            }
            let _ = sys::wait4(-1, ptr::null_mut());
            match right {
                Some(r) => run_cmd(r),
                None => sys::exit(0),
            }
        }
        Cmd::Back { next } => {
            match sys::fork() {
                Ok(0) => run_cmd(next),
                _ => {}
            }
            // No wait: the background child is reaped by init.
            sys::exit(0)
        }
    }
}

fn fork1() -> i64 {
    match sys::fork() {
        Ok(pid) => pid,
        Err(_) => panic_exit("fork"),
    }
}

fn getcmd(buf: &mut [u8; BUFSZ]) -> bool {
    let _ = io::write_all(2, b"$ ");
    buf.fill(0);
    io::gets(0, buf).is_some()
}

#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; BUFSZ];
    // Ensure fds 0..2 exist (xv6 sh opens console until fd 3, then dups).
    while matches!(sys::open(b"/console\0", abi::O_RDWR), Ok(fd) if fd < 3) {}
    loop {
        if !getcmd(&mut buf) {
            sys::exit(0); // EOF on the console: let init restart us
        }
        // cd runs in the shell process itself.
        if buf[0] == b'c' && buf[1] == b'd' && buf[2] == b' ' {
            let mut path: Vec<u8> = Vec::new();
            let mut i = 3usize;
            while buf[i] != 0 && buf[i] != b'\n' && i < buf.len() {
                path.push(buf[i]);
                i += 1;
            }
            if path.is_empty() {
                let _ = io::eprint_str("usage: cd path\n");
                continue;
            }
            path.push(0);
            if sys::chdir(&path).is_err() {
                let _ = io::eprint_str("cannot cd ");
                let _ = io::write_all(2, &path[..path.len() - 1]);
                let _ = io::eprint_str("\n");
            }
            continue;
        }
        let line_end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let toks = tokenize(&buf[..line_end]);
        if toks.is_empty() {
            continue;
        }
        let mut ps = Ps { toks, pos: 0 };
        let tree = parse_line(&mut ps);
        if fork1() == 0 {
            run_cmd(&tree);
        }
        let _ = sys::wait4(-1, ptr::null_mut());
    }
}
