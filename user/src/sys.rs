//! Raw syscall layer (plans/linux26/04 §3): RAX number, RDI/RSI/RDX/R10/
//! R8/R9 arguments, RCX/R11 clobbered by the hardware; raw returns in
//! -4095..=-1 convert to `Err(errno)`, everything else stays a value.

use abi::is_neg_errno;

/// Raw syscall: number in RAX, six register arguments, full 64-bit raw
/// return in RAX (errno range left for the typed wrappers).
#[inline(always)]
pub unsafe fn syscall6(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    let ret;
    // SAFETY: the syscall instruction is the entire contract; memory
    // operands are plain reads/writes of caller-owned buffers.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            in("r9") a6,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Typed syscall result: errno-range negatives become `Err(errno)`.
#[inline]
pub fn call6(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> Result<i64, i64> {
    // SAFETY: plain register plumbing, no memory effects beyond args.
    let raw = unsafe { syscall6(nr, a1, a2, a3, a4, a5, a6) };
    if is_neg_errno(raw) {
        Err(raw)
    } else {
        Ok(raw)
    }
}

/// The never-returning exits (the kernel reaper owns the rest).
pub fn exit(status: i32) -> ! {
    // SAFETY: terminal call; never returns to this frame. The full i32
    // carries the driver verdict (S4e-2): (total<<8)|failed.
    unsafe { syscall6(abi::SYS_EXIT, status as u64, 0, 0, 0, 0, 0) };
    unreachable!("exit returned");
}

pub fn write(fd: i32, buf: &[u8]) -> Result<usize, i64> {
    // SAFETY: kernel reads exactly buf.len() user bytes.
    call6(
        abi::SYS_WRITE,
        fd as u64,
        buf.as_ptr() as u64,
        buf.len() as u64,
        0,
        0,
        0,
    )
    .map(|n| n as usize)
}

pub fn fork() -> Result<i64, i64> {
    call6(abi::SYS_FORK, 0, 0, 0, 0, 0, 0)
}

pub fn getpid() -> Result<i64, i64> {
    call6(abi::SYS_GETPID, 0, 0, 0, 0, 0, 0)
}

/// execve(path, argv, envp). argv/envp are NULL-terminated pointer arrays
/// in the caller's address space; envp is ignored by the M1R kernel
/// (empty environment).
pub fn execve(path: &[u8], argv: *const usize, envp: *const usize) -> Result<i64, i64> {
    // path is NUL-terminated by the caller (rt/builds the C strings).
    // SAFETY: plain register plumbing.
    call6(
        abi::SYS_EXECVE,
        path.as_ptr() as u64,
        argv as u64,
        envp as u64,
        0,
        0,
        0,
    )
}

/// wait4(pid, wstatus, options, rusage). Returns the reaped pid.
pub fn wait4(pid: i32, wstatus: *mut i32) -> Result<i64, i64> {
    // SAFETY: kernel writes 4 bytes through wstatus when non-NULL.
    call6(abi::SYS_WAIT4, pid as u64, wstatus as u64, 0, 0, 0, 0)
}

/// brk(0) queries the current break; brk(addr) moves it.
pub fn brk(addr: u64) -> Result<u64, i64> {
    call6(abi::SYS_BRK, addr, 0, 0, 0, 0, 0).map(|v| v as u64)
}

/// mmap(addr, len, prot, flags, fd, off): M2 anonymous subset. addr 0
/// asks for top-down placement; returns the mapping start.
pub fn mmap(
    addr: usize,
    len: usize,
    prot: u64,
    flags: u64,
    fd: i64,
    off: u64,
) -> Result<usize, i64> {
    // SAFETY: plain register plumbing.
    call6(
        abi::SYS_MMAP,
        addr as u64,
        len as u64,
        prot,
        flags,
        fd as u64,
        off,
    )
    .map(|v| v as usize)
}

/// munmap(addr, len): anonymous mappings only in M2.
pub fn munmap(addr: usize, len: usize) -> Result<i64, i64> {
    // SAFETY: plain register plumbing.
    call6(abi::SYS_MUNMAP, addr as u64, len as u64, 0, 0, 0, 0)
}

/// mprotect(addr, len, prot).
pub fn mprotect(addr: usize, len: usize, prot: u64) -> Result<i64, i64> {
    // SAFETY: plain register plumbing.
    call6(abi::SYS_MPROTECT, addr as u64, len as u64, prot, 0, 0, 0)
}

pub fn kill(pid: i32) -> Result<i64, i64> {
    call6(abi::SYS_KILL, pid as u64, 0, 0, 0, 0, 0)
}

/// open(path, flags). `path` must be NUL-terminated (rt builds C strings).
pub fn open(path: &[u8], flags: u64) -> Result<i64, i64> {
    // SAFETY: kernel reads the path via copyinstr.
    call6(abi::SYS_OPEN, path.as_ptr() as u64, flags, 0, 0, 0, 0)
}

/// read(fd, buf) -> bytes read (0 == EOF).
pub fn read(fd: i32, buf: &mut [u8]) -> Result<usize, i64> {
    // SAFETY: kernel writes exactly buf.len() user bytes on success.
    call6(
        abi::SYS_READ,
        fd as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        0,
        0,
        0,
    )
    .map(|n| n as usize)
}

pub fn close(fd: i32) -> Result<i64, i64> {
    call6(abi::SYS_CLOSE, fd as u64, 0, 0, 0, 0, 0)
}

pub fn dup(fd: i32) -> Result<i64, i64> {
    call6(abi::SYS_DUP, fd as u64, 0, 0, 0, 0, 0)
}

/// pipe(fds): fds[0] = read end, fds[1] = write end.
pub fn pipe(fds: &mut [i32; 2]) -> Result<i64, i64> {
    // SAFETY: kernel writes 8 bytes through fds.
    call6(abi::SYS_PIPE, fds.as_mut_ptr() as u64, 0, 0, 0, 0, 0)
}

pub fn mknod(path: &[u8], major: u64, minor: u64) -> Result<i64, i64> {
    // SAFETY: kernel reads the path via copyinstr.
    call6(abi::SYS_MKNOD, path.as_ptr() as u64, major, minor, 0, 0, 0)
}

/// unlink(2): remove a directory entry; the inode frees when unlinked.
pub fn unlink(path: &[u8]) -> Result<i64, i64> {
    // SAFETY: kernel reads the path via copyinstr.
    call6(abi::SYS_UNLINK, path.as_ptr() as u64, 0, 0, 0, 0, 0)
}

/// link(2): create a new name for an existing file.
pub fn link(old: &[u8], new: &[u8]) -> Result<i64, i64> {
    // SAFETY: kernel reads both paths via copyinstr.
    call6(
        abi::SYS_LINK,
        old.as_ptr() as u64,
        new.as_ptr() as u64,
        0,
        0,
        0,
        0,
    )
}

/// mkdir(2): create a directory.
pub fn mkdir(path: &[u8]) -> Result<i64, i64> {
    // SAFETY: kernel reads the path via copyinstr.
    call6(abi::SYS_MKDIR, path.as_ptr() as u64, 0, 0, 0, 0, 0)
}

/// chdir(2): change the process working directory.
pub fn chdir(path: &[u8]) -> Result<i64, i64> {
    // SAFETY: kernel reads the path via copyinstr.
    call6(abi::SYS_CHDIR, path.as_ptr() as u64, 0, 0, 0, 0, 0)
}

/// fstat(2): inode metadata for an open descriptor into `st`.
pub fn fstat(fd: i32, st: &mut abi::Stat) -> Result<i64, i64> {
    // SAFETY: kernel writes exactly size_of::<Stat>() bytes on success.
    call6(
        abi::SYS_FSTAT,
        fd as u64,
        st as *mut abi::Stat as u64,
        0,
        0,
        0,
        0,
    )
}

/// stat(2) the xv6 way: open + fstat + close.
pub fn stat(path: &[u8], st: &mut abi::Stat) -> Result<i64, i64> {
    let fd = open(path, abi::O_RDONLY)?;
    let r = fstat(fd as i32, st);
    let _ = close(fd as i32);
    r
}

/// sleep(n) in whole seconds (zombie.c semantics; the xv6 unit).
pub fn sleep(seconds: u64) -> Result<i64, i64> {
    let mut ts = [0u8; 16];
    ts[0..8].copy_from_slice(&seconds.to_le_bytes());
    // SAFETY: kernel reads the 16-byte timespec from user memory.
    call6(abi::SYS_NANOSLEEP, ts.as_ptr() as u64, 0, 0, 0, 0, 0)
}

/// sbrk(n): move the break by `n`, returning the previous break address
/// (0 = query without moving). xv6/umalloc's growth primitive over brk.
pub fn sbrk(n: i64) -> Result<u64, i64> {
    let cur = brk(0)?;
    if n == 0 {
        return Ok(cur);
    }
    let want = (cur as i64).wrapping_add(n);
    if want < 0 {
        return Err(abi::ENOMEM);
    }
    brk(want as u64)?;
    Ok(cur)
}
