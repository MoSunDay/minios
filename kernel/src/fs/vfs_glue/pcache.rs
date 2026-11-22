//! pcache.rs (m4r-s3b): the kernel-side page-cache glue — the
//! descriptor pool (a static arena, the DENTRY_POOL discipline) plus
//! the read entry the m4 suite and s4 mmap share: lookup under
//! PC_LOCK, fill through the AddressSpaceOps vtable with the lock
//! dropped (readpage's ilock may sleep in process context), relock to
//! copy and unpin. Since m4r-s4a sys_read's T_FILE descriptors ride
//! this path too (byte-equality proven by the shadow-diff battery
//! run, then the flip — ADR-0025), and the fill window is closed by
//! the uptodate busy-wait gate with a bounded spin + yield.
//!
//! Lock order (ADR-0024): DENTRY_LOCK → PC_LOCK → KMEM_LOCK
//! (wrapper teardown nests PC inside DENTRY; reclaim frees frames
//! under PC). PC_LOCK is never taken from interrupt context and never
//! held across a sleeping fs call. finish_refs must be called OUTSIDE
//! PC_LOCK: dput takes DENTRY_LOCK (order DENTRY → PC) and its prune
//! → wrapper_free path re-enters PC_LOCK (non-reentrant).

use crate::fs::vfs::pagecache::{AddressSpace, PageCache, PageCacheStats, PageDesc, Peek};
use crate::fs::vfs::{imode_has_type, VfsErr, VfsResult, S_IFREG};
use crate::fs::xv6fs::inode::{ilock, iput, iunlock, readi, Inode as Xv6Inode};
use crate::fs::xv6fs::layout::T_FILE;
use crate::klib::sync::Spinlock;
use crate::mm::frame::{kalloc, kfree, PGSIZE};

use super::dput_unlocked;
use super::kern_namei_d;
use super::readahead;
use super::wrapper::{wrapper_epoch, wrapper_get, wrapper_get_for_ip, wrapper_put, VfsInode};

/// Descriptor pool: 256 pages = 1 MiB cache ceiling (pool pressure
/// self-reclaims clean pages — the evict-clean driver).
pub const NPAGE_DESC: usize = 256;

static mut PC: PageCache = PageCache::new();
static mut PAGE_POOL: [PageDesc; NPAGE_DESC] = [const { PageDesc::EMPTY }; NPAGE_DESC];
pub(super) static PC_LOCK: Spinlock<()> = Spinlock::new("pcache", ());

/// Exclusive engine borrow under PC_LOCK (the dc() arena pattern).
pub(super) unsafe fn pcache() -> &'static mut PageCache {
    unsafe { &mut *core::ptr::addr_of_mut!(PC) }
}

/// Boot-context init (vfs_glue::init). Frames leaving the cache go
/// straight back to the buddy.
pub unsafe fn init() {
    unsafe {
        let pc = pcache();
        pc.init(
            core::ptr::addr_of_mut!(PAGE_POOL).cast::<PageDesc>(),
            NPAGE_DESC,
        );
        pc.free_data = Some(kfree);
    }
}

/// Counter snapshot for the suite's gate-style reconciliation.
pub fn pc_stats() -> PageCacheStats {
    let _g = PC_LOCK.lock();
    // SAFETY: engine guarded by PC_LOCK.
    unsafe { pcache() }.stats
}

/// Count `n` prefetched pages (the fill loop in readahead_fill calls
/// this; the counter is part of the m4 gate's IO reconciliation).
pub fn kern_readahead(n: u32) {
    let _g = PC_LOCK.lock();
    // SAFETY: engine guarded by PC_LOCK.
    unsafe { pcache() }.note_readahead(n);
}

/// Reclaim `need` clean idle pages now (the suite's pool-pressure
/// probe); returns how many actually left.
pub fn kern_pc_reclaim(need: usize) -> usize {
    let _g = PC_LOCK.lock();
    // SAFETY: engine guarded by PC_LOCK.
    unsafe { pcache() }.reclaim_clean(need)
}

/// Suite probe: is `index` resident, and which frame backs it? The
/// lookup takes and drops a pin (hits/misses move — part of the
/// accounting the cases reconcile).
pub struct PageProbe {
    pub present: bool,
    pub frame: usize,
    pub refcount: u32,
    pub nrpages: u32,
}

impl PageProbe {
    pub const ABSENT: PageProbe = PageProbe {
        present: false,
        frame: 0,
        refcount: 0,
        nrpages: 0,
    };
}

/// Read `[off, off+buf.len())` of `path` through the page cache,
/// clamped at EOF (returns bytes copied). Each page: hit copies from
/// the frame; miss kallocs one, inserts it, and fills through
/// a_ops->readpage with PC_LOCK dropped.
pub fn kern_read_cached(path: &[u8], off: u64, buf: &mut [u8]) -> VfsResult<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let (ip, d) = kern_namei_d(path)?;
    // SAFETY: the dentry's inode is the wrapper (dcache invariant).
    let w = unsafe { (*d).inode() as *mut VfsInode };
    // SAFETY: regular-file gate, then a reader reference on the
    // wrapper (the dentry pin does NOT survive an unlink's
    // negate-in-place; i_count does — ADR-0025).
    unsafe {
        if !imode_has_type((*w).c.i_mode, S_IFREG) {
            let _ = finish_refs(ip, d);
            return Err(VfsErr::IsDir);
        }
        if !wrapper_get(w) {
            // The name was unlinked out from under the walk (negate
            // in place frees the wrapper despite the dentry pin):
            // read uncached — byte-equal fallback.
            let _ = finish_refs(ip, d);
            return read_uncached(ip, off, buf);
        }
        let r = read_cached_range(w, ip, off, buf);
        wrapper_put(w);
        let _ = finish_refs(ip, d);
        r
    }
}

/// The fd-side cached read (m4r-s4a): resolve the mapping from the
/// inode (wrapper scan), read through the shared range core. With no
/// cached name shadowing the inode the read falls back to the legacy
/// readi bytes — the uncached spelling is byte-equal by construction
/// (ADR-0025).
pub fn kern_read_cached_ip(ip: *mut Xv6Inode, off: u64, buf: &mut [u8]) -> VfsResult<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    // SAFETY: the File's reference keeps the inode cached and its
    // type stable (validated at open).
    unsafe {
        if (*ip).type_ != T_FILE {
            return Err(VfsErr::IsDir);
        }
        let w = wrapper_get_for_ip(ip);
        if w.is_null() {
            return read_uncached(ip, off, buf);
        }
        let r = read_cached_range(w, ip, off, buf);
        wrapper_put(w);
        r
    }
}

/// The uncached fallback: readi under ilock — byte-identical to the
/// legacy fileread chunk by construction.
///
/// SAFETY: `ip` holds a caller-owned reference.
unsafe fn read_uncached(ip: *mut Xv6Inode, off: u64, buf: &mut [u8]) -> VfsResult<usize> {
    unsafe {
        ilock(ip);
        let r = readi(ip, buf.as_mut_ptr(), off as u32, buf.len() as u32);
        iunlock(ip);
        if r < 0 {
            Err(VfsErr::Io) // readi's -1 spelling
        } else {
            Ok(r as usize)
        }
    }
}

/// The shared byte-equal read core (path and fd entries): readi-exact
/// clamping — off > size is the legacy -1; end = min(off+len, size);
/// end <= off reads nothing — then per-page gate/copy with the
/// sequential-window readahead riding demand misses (m4r-s4a).
///
/// SAFETY: caller holds a wrapper reader reference (i_count) and an
/// inode reference; `w`'s mapping shadows `ip`.
unsafe fn read_cached_range(
    w: *mut VfsInode,
    ip: *mut Xv6Inode,
    off: u64,
    buf: &mut [u8],
) -> VfsResult<usize> {
    unsafe {
        ilock(ip);
        let size = (*ip).size as u64;
        iunlock(ip);
        if off > size {
            return Err(VfsErr::Io);
        }
        let end = core::cmp::min(off.saturating_add(buf.len() as u64), size);
        if end <= off {
            return Ok(0);
        }
        let last_page = if size == 0 {
            u64::MAX
        } else {
            (size - 1) / PGSIZE as u64
        };
        let m = mapping_of(w);
        let epoch = wrapper_epoch(w);
        let mut done = 0usize;
        let mut err = None;
        while (off + done as u64) < end {
            let cur = off + done as u64;
            let index = cur / PGSIZE as u64;
            let pgoff = (cur % PGSIZE as u64) as usize;
            let want = core::cmp::min(end - cur, PGSIZE as u64) as usize;
            let n = core::cmp::min(PGSIZE - pgoff, want);
            match read_page_bytes(w, index, pgoff, &mut buf[done..done + n]) {
                Ok(filled) => {
                    done += n;
                    let seq = readahead::note_page(m, epoch, index);
                    if filled && seq {
                        readahead_fill(w, index, last_page);
                    }
                }
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(done),
        }
    }
}

/// Fill the sequential window after a demand miss at `index`
/// (readahead.rs owns the policy; this owns the machinery): each page
/// is checked resident first (no counters, no gate wait), then filled
/// through the same insert+readpage path and parked idle for the
/// upcoming demand hits.
///
/// SAFETY: caller holds a wrapper reader reference on `w`.
unsafe fn readahead_fill(w: *mut VfsInode, index: u64, last_page: u64) {
    unsafe {
        let m = mapping_of(w);
        let mut n = 0u32;
        for idx in readahead::window(index, last_page) {
            let absent = {
                let _g = PC_LOCK.lock();
                // SAFETY: live mapping under the guard.
                matches!(pcache().peek(&mut *m, idx), Peek::Absent)
            };
            if !absent {
                continue; // already resident (or filling): covered
            }
            match fill_page(m, idx) {
                Ok(pg) => {
                    let _g = PC_LOCK.lock();
                    // SAFETY: pinned fresh page; park it idle.
                    pcache().put_page(pg);
                    n += 1;
                }
                Err(_) => break, // advisory: stop at the first refusal
            }
        }
        if n > 0 {
            kern_readahead(n);
        }
    }
}

/// Probe residency of one page of `path` (no bytes copied).
pub fn kern_page_probe(path: &[u8], index: u64, out: &mut PageProbe) -> VfsResult<()> {
    *out = PageProbe::ABSENT;
    let (ip, d) = kern_namei_d(path)?;
    // SAFETY: dentry's inode is the wrapper.
    let w = unsafe { (*d).inode() as *mut VfsInode };
    // SAFETY: gate + mapping access as in kern_read_cached.
    unsafe {
        if !imode_has_type((*w).c.i_mode, S_IFREG) {
            let _ = finish_refs(ip, d);
            return Err(VfsErr::IsDir);
        }
        if !wrapper_get(w) {
            let _ = finish_refs(ip, d);
            return Err(VfsErr::NoEnt);
        }
        let m = mapping_of(w);
        {
            let _g = PC_LOCK.lock();
            let pc = pcache();
            let pg = pc.lookup(&mut *m, index);
            if !pg.is_null() {
                out.present = true;
                out.frame = (*pg).data as usize;
                out.refcount = (*pg).refcount;
                pc.put_page(pg);
            }
            out.nrpages = (*m).nrpages;
        }
        // finish_refs must run outside PC_LOCK (the kern_read_cached
        // teardown shape): dput takes DENTRY_LOCK and its prune path
        // re-enters PC_LOCK through wrapper_free.
        wrapper_put(w);
        let _ = finish_refs(ip, d);
    }
    Ok(())
}

/// Release the kern_namei_d pair (idup'd inode ref + dentry pin).
unsafe fn finish_refs(
    ip: *mut crate::fs::xv6fs::inode::Inode,
    d: *mut crate::fs::vfs::dentry::Dentry,
) -> VfsResult<()> {
    // SAFETY: caller-owned references.
    unsafe {
        dput_unlocked(d);
        iput(ip);
    }
    Ok(())
}

/// The wrapper's mapping (wrap_new binds i_mapping to its own i_data).
unsafe fn mapping_of(w: *mut VfsInode) -> *mut AddressSpace {
    // SAFETY: wrapper holds a live mapping binding.
    unsafe { (*w).c.i_mapping }
}

/// One page through the cache. Returns Ok(true) when the page was
/// demand-FILLED (the readahead trigger), Ok(false) on a hit. The
/// resident-but-!uptodate case (the ADR-0024 fill window) and the
/// insert-Busy winner both enter the SAME uptodate busy-wait gate
/// (ADR-0025): readers never copy a half-filled frame.
///
/// SAFETY: caller holds a wrapper reader reference on `w`.
unsafe fn read_page_bytes(
    w: *mut VfsInode,
    index: u64,
    pgoff: usize,
    out: &mut [u8],
) -> VfsResult<bool> {
    let m = unsafe { mapping_of(w) };
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        if attempts > PAGE_ATTEMPTS_MAX {
            return Err(VfsErr::Busy);
        }
        // Fast path under one guard: an UPTODATE hit copies and unpins.
        let hit: Option<VfsResult<()>> = {
            let _g = PC_LOCK.lock();
            let pc = pcache();
            // SAFETY: live mapping.
            let pg = pc.lookup(&mut *m, index);
            if pg.is_null() {
                None // miss → fill below
            } else if unsafe { (*pg).uptodate } {
                // SAFETY: pinned by the lookup; frame is PGSIZE.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (*pg).data.add(pgoff),
                        out.as_mut_ptr(),
                        out.len(),
                    );
                    pc.put_page(pg);
                }
                Some(Ok(()))
            } else {
                Some(Err(VfsErr::Busy)) // filling: enter the gate
            }
        };
        if let Some(r) = hit {
            match r {
                Ok(()) => return Ok(false),
                Err(VfsErr::Busy) => {
                    // SAFETY: live mapping; gate below.
                    match unsafe { wait_gate_copy(m, index, pgoff, out) } {
                        Ok(()) => return Ok(false),
                        // The filler failed and removed the page:
                        // retry the whole page (bounded).
                        Err(VfsErr::Busy) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Err(e) => return Err(e),
            }
        }
        // Miss: fill then copy.
        // SAFETY: live mapping; we hold the wrapper reader ref.
        match unsafe { fill_page(m, index) } {
            Ok(pg) => {
                let _g = PC_LOCK.lock();
                let pc = pcache();
                // SAFETY: pinned; frame filled by readpage.
                unsafe {
                    debug_assert!((*pg).uptodate);
                    core::ptr::copy_nonoverlapping(
                        (*pg).data.add(pgoff),
                        out.as_mut_ptr(),
                        out.len(),
                    );
                    pc.put_page(pg);
                }
                return Ok(true);
            }
            // A concurrent filler owns the slot: same gate door
            // (ADR-0025 "Busy-winner 分支同门进门").
            Err(VfsErr::Busy) => match unsafe { wait_gate_copy(m, index, pgoff, out) } {
                Ok(()) => return Ok(false),
                Err(VfsErr::Busy) => continue,
                Err(e) => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
}

/// Bounded page-read attempts (gate Absent retries); exhausting them
/// is a livelock-class bug, never a normal result.
const PAGE_ATTEMPTS_MAX: u32 = 4;

/// Spin bound for the uptodate gate: generous for the process-context
/// yield loop (each round is one reschedule), an instant trip-wire in
/// the boot context (where the window is unreachable — single
/// context, ADR-0025).
const GATE_SPIN_MAX: u32 = 1_000_000;

/// The ADR-0025 uptodate busy-wait gate: a resident !uptodate page is
/// mid-fill by another context; spin (yielding in process context so
/// the filler can run — the kernel does not preempt) until it turns
/// Ready, then copy under the lock. Absent returns Err(Busy): the
/// filler failed and removed the page — the caller retries the miss
/// path. The bound panics loudly on livelock (a same-context fill
/// cannot be waited out).
///
/// SAFETY: `m` is a live mapping; caller holds a reader reference.
unsafe fn wait_gate_copy(
    m: *mut AddressSpace,
    index: u64,
    pgoff: usize,
    out: &mut [u8],
) -> VfsResult<()> {
    let mut spins: u32 = 0;
    loop {
        {
            let _g = PC_LOCK.lock();
            let pc = pcache();
            // SAFETY: live mapping under the guard.
            match pc.peek(&mut *m, index) {
                Peek::Ready(pg) => {
                    // SAFETY: uptodate ⇒ frame complete; copying under
                    // the guard needs no pin (reclaim takes this lock).
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            (*pg).data.add(pgoff),
                            out.as_mut_ptr(),
                            out.len(),
                        );
                    }
                    return Ok(());
                }
                Peek::Absent => return Err(VfsErr::Busy),
                Peek::Filling(_) => {}
            }
        }
        spins += 1;
        if spins > GATE_SPIN_MAX {
            panic!("pagecache: uptodate gate livelock (index {})", index);
        }
        if crate::proc::proc::myproc().is_null() {
            core::hint::spin_loop();
        } else {
            crate::proc::sched::yield_cpu();
        }
    }
}

/// Insert + fill one page (the demand-miss and readahead paths share
/// it): kalloc a frame, insert pinned with PC_LOCK dropped for the
/// a_ops->readpage fill (its ilock may sleep — ADR-0024), and return
/// the page pinned and uptodate. The insert-then-unlock-fill window
/// is exactly what the uptodate gate closes.
///
/// SAFETY: `m` is a live mapping; caller holds a reader reference.
unsafe fn fill_page(m: *mut AddressSpace, index: u64) -> VfsResult<*mut PageDesc> {
    let frame = kalloc();
    if frame.is_null() {
        return Err(VfsErr::NoSpc);
    }
    let pg;
    {
        let _g = PC_LOCK.lock();
        let pc = pcache();
        pc.note_readpage();
        // SAFETY: fresh frame, live mapping.
        match pc.insert(&mut *m, index, frame) {
            Ok(p) => pg = p,
            Err(_) => {
                // Busy (a concurrent filler) or pool refusal: the
                // frame stays ours — hand it back and let the caller
                // take the gate door / propagate.
                drop(_g);
                kfree(frame);
                return Err(VfsErr::Busy);
            }
        }
    }
    let a_ops = unsafe { (*m).a_ops };
    if a_ops.is_null() {
        let _g = PC_LOCK.lock();
        // SAFETY: we hold the insert pin.
        let data = unsafe { pcache().remove(&mut *m, pg) };
        kfree(data);
        return Err(VfsErr::NotSup);
    }
    // SAFETY: pinned fresh page; vtable dispatch (xv6_obj::readpage).
    let r = unsafe { ((*a_ops).readpage)(m, index, pg) };
    if let Err(e) = r {
        let _g = PC_LOCK.lock();
        // SAFETY: we hold the insert pin.
        let data = unsafe { pcache().remove(&mut *m, pg) };
        kfree(data);
        return Err(e);
    }
    Ok(pg)
}

// ---- Suite hooks (m4r-s4a wave-3; the bread_count precedent) ----

/// Suite hook: classify one page through the SAME gate the read path
/// spins on — 0 absent, 1 filling (resident, !uptodate), 2 ready.
/// Gate peeks carry no hit/miss counters (the wait loop must not
/// pollute the reconciliation the cases pin).
pub fn kern_gate_peek(path: &[u8], index: u64) -> i32 {
    let (ip, d) = match kern_namei_d(path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    // SAFETY: walk-verified wrapper + reader ref, gate under PC_LOCK.
    unsafe {
        let w = (*d).inode() as *mut VfsInode;
        if !wrapper_get(w) {
            let _ = finish_refs(ip, d);
            return -1;
        }
        let m = mapping_of(w);
        let _g = PC_LOCK.lock();
        let r = match pcache().peek(&mut *m, index) {
            Peek::Absent => 0,
            Peek::Filling(_) => 1,
            Peek::Ready(_) => 2,
        };
        drop(_g);
        wrapper_put(w);
        let _ = finish_refs(ip, d);
        r
    }
}

/// Suite hook: open (uptodate=false) / close (true) the ADR-0025 fill
/// window on a RESIDENT page — flips the flag only, bytes untouched.
/// Returns false when the page is absent (nothing to pin a window on).
pub fn kern_window_set(path: &[u8], index: u64, uptodate: bool) -> bool {
    let (ip, d) = match kern_namei_d(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // SAFETY: walk-verified wrapper + reader ref; flag under PC_LOCK.
    unsafe {
        let w = (*d).inode() as *mut VfsInode;
        if !wrapper_get(w) {
            let _ = finish_refs(ip, d);
            return false;
        }
        let m = mapping_of(w);
        let _g = PC_LOCK.lock();
        let pc = pcache();
        let mut ok = false;
        match pc.peek(&mut *m, index) {
            Peek::Absent => {}
            // Ready|Filling both hand the descriptor: opening flips
            // Ready→Filling, closing flips Filling→Ready (the close
            // path was the original wave-3 bug — Filling used to
            // carry no pointer and the arm silently dropped it).
            Peek::Ready(pg) | Peek::Filling(pg) => {
                (*pg).uptodate = uptodate;
                ok = true;
            }
        }
        drop(_g);
        wrapper_put(w);
        let _ = finish_refs(ip, d);
        ok
    }
}
