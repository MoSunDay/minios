//! page-cache wave-2 cases (m4r-s3): the read path through the cache
//! on the real root disk — hit/miss/IO-counter reconciliation, shared
//! handles over one frame, fill-equals-readi with the zeroed EOF
//! tail, and clean-page reclaim with re-fill. Every case builds its
//! own file (pattern-filled through the legacy writei inside a
//! transaction), verifies, and tears down to the slot-ledger baseline.

use super::{drained_slots, with_inode};
use crate::fs::buffer::bread_count;
use crate::fs::vfs_glue::pcache::{
    kern_gate_peek, kern_page_probe, kern_pc_reclaim, kern_read_cached, kern_window_set, pc_stats,
    PageProbe,
};
use crate::fs::vfs_glue::{kern_create, kern_unlink, CreateKind};
use crate::fs::xv6fs::inode::{ilock, iput, iunlock, readi, writei, Inode};
use crate::fs::xv6fs::log::{begin_op, end_op};
use crate::mm::frame::PGSIZE;

/// Deterministic file content: two terms so every offset bit moves.
fn pat(off: usize) -> u8 {
    (0x5a ^ off as u8).wrapping_add((off >> 8) as u8)
}

/// Create `path` and pattern-fill `size` bytes. One transaction per
/// chunk (the per-write discipline: LOGSIZE=30 caps a single tx, and
/// the 16 KiB case writes 32 data blocks); each tx is the adapter's
/// begin/ilock/write/iunlock/end sequence. Returns false on any
/// refusal; the caller checks the slot ledger at teardown.
fn make_file(path: &[u8], size: usize) -> bool {
    let ip = match kern_create(path, CreateKind::File) {
        Ok(ip) => ip,
        Err(_) => return false,
    };
    // SAFETY: the created reference; writei under ilock inside each
    // transaction is the legacy discipline (boot-context IDE polls).
    unsafe {
        let mut off = 0usize;
        let mut ok = true;
        let mut chunk = [0u8; 512];
        while off < size {
            let n = core::cmp::min(512, size - off);
            for i in 0..n {
                chunk[i] = pat(off + i);
            }
            begin_op();
            ilock(ip);
            if writei(ip, chunk.as_ptr(), off as u32, n as u32) != n as i32 {
                ok = false;
            }
            iunlock(ip);
            end_op();
            if !ok {
                break;
            }
            off += n;
        }
        iput(ip);
        ok
    }
}

/// Read the whole file through the cache page by page and verify the
/// pattern (also the pass driver for hit/miss accounting).
fn read_verify(path: &[u8], size: usize) -> bool {
    let mut buf = [0u8; PGSIZE];
    let mut off = 0usize;
    while off < size {
        let n = core::cmp::min(PGSIZE, size - off);
        match kern_read_cached(path, off as u64, &mut buf[..n]) {
            Ok(c) if c == n => {}
            _ => return false,
        }
        for i in 0..n {
            if buf[i] != pat(off + i) {
                return false;
            }
        }
        off += n;
    }
    true
}

/// Same range twice: the first pass misses (fill + readpage + real
/// disk traffic), the second hits purely from frames — counters and
/// the bread count must reconcile exactly.
pub fn case_pc_hit_miss() -> bool {
    let path = b"/m4pchit0";
    let base = drained_slots();
    let size = 3 * PGSIZE + 100; // 4 pages, partial tail
    let made = make_file(path, size);
    let s0 = pc_stats();
    let pass1 = made && read_verify(path, size);
    let s1 = pc_stats();
    let b1 = bread_count();
    let pass2 = pass1 && read_verify(path, size);
    let s2 = pc_stats();
    let b2 = bread_count();
    // Pass 1 (readahead active, m4r-s4a): idx0 cold miss, idx1
    // sequential miss + 双页窗 RA{2,3}, idx2/3 prefetched hits.
    let fill = s1.misses - s0.misses == 2
        && s1.readpages - s0.readpages == 4
        && s1.hits - s0.hits == 2
        && s1.inserts - s0.inserts == 4
        && s1.readahead - s0.readahead == 2
        && b1 > 0;
    // Pass 2: every page a hit, zero refill, zero disk traffic.
    let cached = s2.hits - s1.hits == 4
        && s2.misses == s1.misses
        && s2.readpages == s1.readpages
        && s2.evicts == s1.evicts
        && b2 == b1;
    // Teardown (either spelling removes the same dirent).
    let torn = kern_unlink(path).is_ok();
    pass1 && pass2 && fill && cached && torn && drained_slots() == base
}

/// Two lookups of one resident page share the same frame (the
/// find_get_page contract at the glue surface); the mapping holds
/// exactly the pages read.
pub fn case_pc_share_handles() -> bool {
    let path = b"/m4pcshr0";
    let base = drained_slots();
    let size = 2 * PGSIZE;
    let made = make_file(path, size);
    let warmed = made && read_verify(path, size); // page 0/1 resident
    let mut p1 = PageProbe::ABSENT;
    let mut p2 = PageProbe::ABSENT;
    let mut p3 = PageProbe::ABSENT;
    let probed = warmed
        && kern_page_probe(path, 0, &mut p1).is_ok()
        && kern_page_probe(path, 0, &mut p2).is_ok()
        && kern_page_probe(path, 1, &mut p3).is_ok();
    let shares = probed
        && p1.present
        && p2.present
        && p3.present
        && p1.frame == p2.frame // same page, same frame
        && p1.frame != p3.frame // neighbor page, distinct frame
        && p1.refcount == 1 // probe pin observed at hold time
        && p1.nrpages == 2;
    // The file page still reads back byte-identical through the
    // shared frame.
    let still_reads = shares && read_verify(path, size);
    let torn = kern_unlink(path).is_ok();
    warmed && probed && shares && still_reads && torn && drained_slots() == base
}

/// The cache's bytes equal a direct legacy readi of the same range,
/// and the frame's tail past EOF is zeroed (readpage's short-read
/// fill), never stale frame garbage.
pub fn case_pc_fill_matches_readi() -> bool {
    let path = b"/m4pcfil0";
    let base = drained_slots();
    let size = 2 * PGSIZE + 123; // page 2 partial
    let made = make_file(path, size);
    let warmed = made && read_verify(path, size);
    // Frame-level inspection of the partial page.
    let mut probe = PageProbe::ABSENT;
    let pinned = warmed && kern_page_probe(path, 2, &mut probe).is_ok() && probe.present;
    let mut frame_ok = false;
    if pinned {
        // SAFETY: the probe's frame is a resident PGSIZE page (the
        // engine's kalloc'd frame); read-only inspection.
        unsafe {
            let f = probe.frame as *const u8;
            let mut ok = true;
            for i in 0..123usize {
                if *f.add(i) != pat(2 * PGSIZE + i) {
                    ok = false;
                }
            }
            for i in 123..PGSIZE {
                if *f.add(i) != 0 {
                    ok = false;
                }
            }
            frame_ok = ok;
        }
    }
    // Direct readi of the same partial page into a zeroed buffer.
    let direct_ok = with_inode(path, |ip: &Inode| {
        let mut buf = [0u8; PGSIZE];
        // SAFETY: with_inode's reference keeps the inode cached;
        // readi under ilock is the legacy contract.
        unsafe {
            ilock(ip as *const Inode as *mut Inode);
            let n = readi(
                ip as *const Inode as *mut Inode,
                buf.as_mut_ptr(),
                (2 * PGSIZE) as u32,
                PGSIZE as u32,
            );
            iunlock(ip as *const Inode as *mut Inode);
            if n != 123 {
                return false;
            }
        }
        // SAFETY: probe frame inspection mirrors these bytes.
        unsafe {
            let f = probe.frame as *const u8;
            for i in 0..123usize {
                if *f.add(i) != buf[i] {
                    return false;
                }
            }
        }
        true
    });
    let torn = kern_unlink(path).is_ok();
    made && warmed && pinned && frame_ok && direct_ok && torn && drained_slots() == base
}

/// Pool pressure: reclaim drops the oldest clean idle pages (FIFO),
/// survivors stay resident, and an evicted page re-fills byte-correct
/// on the next read (miss + readpage again).
pub fn case_pc_evict_clean() -> bool {
    let path = b"/m4pcev0";
    let base = drained_slots();
    let size = 4 * PGSIZE;
    let made = make_file(path, size);
    let warmed = made && read_verify(path, size);
    let s0 = pc_stats();
    // Pages were read 0..4 in order: reclaim takes 0,1,2 first.
    let evicted = kern_pc_reclaim(3);
    let mut gone = PageProbe::ABSENT;
    let mut kept = PageProbe::ABSENT;
    let probed = warmed
        && evicted == 3
        && kern_page_probe(path, 0, &mut gone).is_ok()
        && kern_page_probe(path, 3, &mut kept).is_ok()
        && !gone.present
        && kept.present;
    // Re-read page 0: miss + refill + bytes correct again.
    let mut buf = [0u8; PGSIZE];
    let refilled = probed
        && match kern_read_cached(path, 0, &mut buf) {
            Ok(c) if c == PGSIZE => {
                let mut ok = true;
                for i in 0..PGSIZE {
                    if buf[i] != pat(i) {
                        ok = false;
                    }
                }
                ok
            }
            _ => false,
        };
    let s1 = pc_stats();
    let accounted = refilled
        && s1.misses - s0.misses == 2 // probe(0) miss + refill miss
        && s1.readpages - s0.readpages == 1 // only the refill hit disk
        && s1.evicts - s0.evicts == 3;
    // Over-asking reclaims only what is resident (page 3 + page 0).
    let drained_cache = accounted && kern_pc_reclaim(100) == 2;
    let mut none = PageProbe::ABSENT;
    let empty = drained_cache && kern_page_probe(path, 3, &mut none).is_ok() && !none.present;
    let torn = kern_unlink(path).is_ok();
    made && warmed
        && probed
        && refilled
        && accounted
        && drained_cache
        && empty
        && torn
        && drained_slots() == base
}

// ---- wave-3 (m4r-s4a): fill-window gate, readahead, probe under pressure ----

/// The ADR-0025 fill window: a resident !uptodate page classifies as
/// Filling at the SAME gate the read path spins on (never a copy of a
/// half-filled frame), flips back to Ready when the fill completes,
/// and peeks carry no hit/miss counters. Bytes through the closed
/// window still read back correct.
pub fn case_pc_gate_window() -> bool {
    let path = b"/m4pcgw0";
    let base = drained_slots();
    let size = PGSIZE;
    let made = make_file(path, size);
    let warmed = made && read_verify(path, size);
    // Ready on the filled page.
    let ready = warmed && kern_gate_peek(path, 0) == 2;
    // Open the window: the page goes !uptodate mid-fill shape.
    let opened = ready && kern_window_set(path, 0, false);
    let filling = opened && kern_gate_peek(path, 0) == 1;
    // Gate peeks are counter-free (the wait loop must not pollute).
    let quiet = filling && {
        let s1 = pc_stats();
        kern_gate_peek(path, 0);
        let s2 = pc_stats();
        s2.hits == s1.hits && s2.misses == s1.misses
    };
    // Close the window: Ready again, and the bytes never changed.
    let closed = quiet && kern_window_set(path, 0, true) && kern_gate_peek(path, 0) == 2;
    let reads_ok = closed && read_verify(path, size);
    let torn = kern_unlink(path).is_ok();
    made && warmed
        && ready
        && opened
        && filling
        && quiet
        && closed
        && reads_ok
        && torn
        && drained_slots() == base
}

/// Sequential readahead (双页窗): a sequential demand miss fills the
/// window ahead; prefetched pages hit without further IO; a window
/// clamped at the last page prefetches nothing past EOF; broken-run
/// misses never fill ahead. Counters must reconcile exactly.
pub fn case_pc_readahead_seq() -> bool {
    let path = b"/m4pcra0";
    let path2 = b"/m4pcra1";
    let base = drained_slots();
    let size = 6 * PGSIZE;
    let made = make_file(path, size) && make_file(path2, 4 * PGSIZE);
    crate::fs::vfs_glue::readahead::reset();
    let s0 = pc_stats();
    let pass1 = made && read_verify(path, size);
    let s1 = pc_stats();
    let b1 = bread_count();
    // idx0 miss (cold hint); idx1 seq miss + RA{2,3}; idx2/3 hits;
    // idx4 seq miss + RA{5} (window clamped at last page 5); idx5 hit.
    let fill = pass1
        && s1.misses - s0.misses == 3
        && s1.readpages - s0.readpages == 6
        && s1.hits - s0.hits == 3
        && s1.inserts - s0.inserts == 6
        && s1.readahead - s0.readahead == 3
        && b1 > 0;
    // Re-read: pure hits, zero fills, zero disk traffic.
    let pass2 = fill && read_verify(path, size);
    let s2 = pc_stats();
    let b2 = bread_count();
    let cached = pass2
        && s2.hits - s1.hits == 6
        && s2.misses == s1.misses
        && s2.readpages == s1.readpages
        && b2 == b1;
    // Broken runs on a cold file: idx2 then idx0 both miss without a
    // predecessor; idx1 (sequential after 0) misses and its window
    // finds idx2 resident, prefetching only idx3.
    let mut buf = [0u8; PGSIZE];
    let r1 = cached
        && match kern_read_cached(path2, 2 * PGSIZE as u64, &mut buf) {
            Ok(PGSIZE) => (0..PGSIZE).all(|i| buf[i] == pat(2 * PGSIZE + i)),
            _ => false,
        };
    let s3 = pc_stats();
    let broke1 = r1 && s3.misses - s2.misses == 1 && s3.readpages - s2.readpages == 1;
    let r2 = broke1
        && match kern_read_cached(path2, 0, &mut buf) {
            Ok(PGSIZE) => (0..PGSIZE).all(|i| buf[i] == pat(i)),
            _ => false,
        };
    let s4 = pc_stats();
    let broke2 = r2 && s4.misses - s3.misses == 1 && s4.readpages - s3.readpages == 1;
    let r3 = broke2
        && match kern_read_cached(path2, PGSIZE as u64, &mut buf) {
            Ok(PGSIZE) => (0..PGSIZE).all(|i| buf[i] == pat(PGSIZE + i)),
            _ => false,
        };
    let s5 = pc_stats();
    let seq_tail = r3
        && s5.misses - s4.misses == 1
        && s5.readpages - s4.readpages == 2 // demand idx1 + RA idx3
        && s5.readahead - s4.readahead == 1;
    let torn = kern_unlink(path).is_ok() && kern_unlink(path2).is_ok();
    made && pass1
        && fill
        && pass2
        && cached
        && broke1
        && broke2
        && seq_tail
        && torn
        && drained_slots() == base
}

/// Probe under dcache pressure: churn enough names to evict the
/// warmed file's idle dentry — the wrapper (and its mapping) dies
/// with it, the probe walks a FRESH wrapper and sees no pages, and
/// the re-read refills byte-correct from disk.
/// Concurrent churn files: past the 50-slot xv6 relief valve, well
/// under the 200-inode disk budget.
const CHURN_FILES: usize = 120;

pub fn case_pc_probe_pressure() -> bool {
    let path = b"/m4pcpp0";
    let base = drained_slots();
    let made = make_file(path, PGSIZE);
    let warmed = made && read_verify(path, PGSIZE);
    let mut p0 = PageProbe::ABSENT;
    let resident =
        warmed && kern_page_probe(path, 0, &mut p0).is_ok() && p0.present && p0.nrpages == 1;
    // Churn enough fresh names to evict the warmed dentry: the
    // 50-slot xv6 table forces relieve_inode_pressure to prune idle
    // LRU dentries long before the 256-slot dcache fills, and this
    // file's dentry is the oldest idle one. 120 stays inside the
    // 200-inode disk budget (base + warmed + churn < 200).
    let mut churn_ok = true;
    let mut name = [0u8; 16];
    for i in 0..CHURN_FILES {
        let len = write_name(&mut name, b"/m4pp", i);
        match kern_create(&name[..len], CreateKind::File) {
            Ok(ip) => {
                // SAFETY: balancing put for the created reference.
                unsafe { iput(ip) };
            }
            Err(_) => {
                churn_ok = false;
                break;
            }
        }
    }
    let mut gone = PageProbe::ABSENT;
    let evicted = churn_ok
        && resident
        && kern_page_probe(path, 0, &mut gone).is_ok()
        && !gone.present
        && gone.nrpages == 0;
    // The file itself is intact: re-read refills byte-correct.
    let refilled = evicted && read_verify(path, PGSIZE);
    let torn = refilled && kern_unlink(path).is_ok();
    // Churn teardown (slot ledger must return to baseline).
    let mut cleanup = true;
    for i in 0..CHURN_FILES {
        let len = write_name(&mut name, b"/m4pp", i);
        if kern_unlink(&name[..len]).is_err() {
            cleanup = false;
        }
    }
    made && warmed
        && resident
        && churn_ok
        && evicted
        && refilled
        && torn
        && cleanup
        && drained_slots() == base
}

/// `/prefix` + decimal `i` + NUL into `name`; returns the length.
fn write_name(name: &mut [u8; 16], prefix: &[u8], i: usize) -> usize {
    let mut len = prefix.len();
    name[..len].copy_from_slice(prefix);
    let mut buf = [0u8; 8];
    let mut j = buf.len();
    let mut n = i;
    loop {
        j -= 1;
        buf[j] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    name[len..len + buf.len() - j].copy_from_slice(&buf[j..]);
    len += buf.len() - j;
    name[len] = 0;
    len
}
