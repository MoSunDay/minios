//! The xv6 usertests battery (S4e-2), run by pid 1 in the `m1r-user`
//! boot: each case is an xv6 usertests.c test ported to the Rust user
//! runtime, plus exercisers for the S4e-1 program set. The thin
//! `usertests` bin seeds the console and drives this table; the kernel
//! merges the counts into the `suite=m1r-user` verdict marker.

pub mod exec;
pub mod fs_basic;
pub mod fs_big;
pub mod fs_link;
pub mod fs_names;
pub mod ipc;
pub mod mem;
pub mod prog;
pub mod signal;

use crate::io;

/// Verdict counts carried by the driver's exit status.
pub struct Counts {
    pub total: u32,
    pub failed: u32,
}

type CaseFn = fn() -> bool;

/// The case table in xv6 usertests.c main() order, then the program
/// exercisers. Case ids are frozen: the runner spec mirrors this list.
const CASES: &[(&str, CaseFn)] = &[
    ("xv6.mem.bsstest", mem::case_bsstest),
    ("xv6.mem.sbrk", mem::case_sbrk),
    ("xv6.mem.mmap", mem::case_mmap),
    ("xv6.mem.munmap", mem::case_munmap),
    ("xv6.mem.cow", mem::case_cow),
    ("xv6.mem.oom", mem::case_oom),
    ("xv6.fs.opentest", fs_basic::case_opentest),
    ("xv6.fs.writetest", fs_basic::case_writetest),
    ("xv6.fs.writetest1", fs_basic::case_writetest1),
    ("xv6.fs.createtest", fs_basic::case_createtest),
    ("xv6.ipc.pipe1", ipc::case_pipe1),
    ("xv6.ipc.preempt", ipc::case_preempt),
    ("xv6.ipc.exitwait", ipc::case_exitwait),
    ("xv6.fs.rmdot", fs_names::case_rmdot),
    ("xv6.fs.fourteen", fs_names::case_fourteen),
    ("xv6.fs.bigwrite", fs_big::case_bigwrite),
    ("xv6.fs.bigfile", fs_big::case_bigfile),
    ("xv6.fs.subdir", fs_names::case_subdir),
    ("xv6.fs.linkunlink", fs_link::case_linkunlink),
    ("xv6.fs.linktest", fs_link::case_linktest),
    ("xv6.fs.unlinkread", fs_link::case_unlinkread),
    ("xv6.fs.createdelete", fs_basic::case_createdelete),
    ("xv6.fs.twofiles", fs_basic::case_twofiles),
    ("xv6.fs.sharedfd", fs_basic::case_sharedfd),
    ("xv6.fs.dirfile", fs_names::case_dirfile),
    ("xv6.fs.bigdir", fs_big::case_bigdir),
    ("xv6.exec.exectest", exec::case_exectest),
    ("xv6.prog.echo", prog::case_prog_echo),
    ("xv6.prog.cat", prog::case_prog_cat),
    ("xv6.prog.grep", prog::case_prog_grep),
    ("xv6.prog.wc", prog::case_prog_wc),
    ("xv6.prog.ls", prog::case_prog_ls),
    ("xv6.prog.mkdir", prog::case_prog_mkdir),
    ("xv6.prog.rm", prog::case_prog_rm),
    ("xv6.prog.ln", prog::case_prog_ln),
    ("xv6.prog.kill", prog::case_prog_kill),
    ("xv6.prog.zombie", prog::case_prog_zombie),
    ("xv6.prog.forktest", prog::case_prog_forktest),
    ("xv6.prog.stressfs", prog::case_prog_stressfs),
    ("xv6.sig.roundtrip", signal::case_roundtrip),
    ("xv6.sig.mask_defer", signal::case_mask_defer),
    ("xv6.ipc.sigpipe", signal::case_sigpipe),
    ("xv6.sig.except", signal::case_except),
    ("xv6.sig.chld", signal::case_chld),
    ("xv6.sig.alarm", signal::case_alarm),
];

/// Run every case, printing one `CASE <id> PASS|FAIL` line each.
pub fn run_all() -> Counts {
    let mut counts = Counts {
        total: 0,
        failed: 0,
    };
    for (id, case) in CASES {
        counts.total += 1;
        let ok = case();
        if !ok {
            counts.failed += 1;
        }
        let _ = io::write_all(io::STDOUT, b"CASE ");
        let _ = io::write_all(io::STDOUT, id.as_bytes());
        let _ = io::write_all(io::STDOUT, if ok { b" PASS\n" } else { b" FAIL\n" });
    }
    counts
}
