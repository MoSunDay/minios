//! pcache.rs (m4r-s3b): the kernel-side page-cache glue — the
//! descriptor pool (a static arena, the DENTRY_POOL discipline) plus
//! the read entry the m4 suite and s4 mmap share: lookup under
//! PC_LOCK, fill through the AddressSpaceOps vtable with the lock
//! dropped (readpage's ilock may sleep in process context), relock to
//! copy and unpin. sys_read stays on the frozen legacy path (bytes
//! pinned by the battery); the switchover needs its own byte-equality
//! proof and lands with s4/s5.
//!
//! Lock order (ADR-0024): DENTRY_LOCK → PC_LOCK → KMEM_LOCK
//! (wrapper teardown nests PC inside DENTRY; reclaim frees frames
//! under PC). PC_LOCK is never taken from interrupt context and never
//! held across a sleeping fs call. finish_refs must be called OUTSIDE
//! PC_LOCK: dput takes DENTRY_LOCK (order DENTRY → PC) and its prune
//! → wrapper_free path re-enters PC_LOCK (non-reentrant).

use crate::fs::vfs::pagecache::{AddressSpace, PageCache, PageCacheStats, PageDesc};
use crate::fs::vfs::{imode_has_type, VfsErr, VfsResult, S_IFREG};
use crate::fs::xv6fs::inode::iput;
use crate::klib::sync::Spinlock;
use crate::mm::frame::{kalloc, kfree, PGSIZE};

use super::dput_unlocked;
use super::kern_namei_d;
use super::wrapper::VfsInode;

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

/// Readahead hook (M4 counts only — the window algorithm is Parity,
/// 23 §1; s4 mmap is the first real caller).
#[allow(dead_code)] // s4: fault-around readahead
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
    // SAFETY: regular-file gate; then the legacy size read under ilock
    // (the wrapper's iget ref keeps the inode cached, ilock makes the
    // dinode fields valid).
    unsafe {
        if !imode_has_type((*w).c.i_mode, S_IFREG) {
            let _ = finish_refs(ip, d);
            return Err(VfsErr::IsDir);
        }
        crate::fs::xv6fs::inode::ilock(ip);
        let size = (*ip).size as u64;
        crate::fs::xv6fs::inode::iunlock(ip);
        let end = core::cmp::min(off + buf.len() as u64, size);
        if end <= off {
            let _ = finish_refs(ip, d);
            return Ok(0);
        }
        let mut done = 0usize;
        let mut err = None;
        while (off + done as u64) < end {
            let cur = off + done as u64;
            let index = cur / PGSIZE as u64;
            let pgoff = (cur % PGSIZE as u64) as usize;
            let want = core::cmp::min(end - cur, PGSIZE as u64) as usize;
            let n = core::cmp::min(PGSIZE - pgoff, want);
            match read_page_bytes(w, index, pgoff, &mut buf[done..done + n]) {
                Ok(()) => done += n,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        let _ = finish_refs(ip, d);
        match err {
            Some(e) => Err(e),
            None => Ok(done),
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

/// One page through the cache: hit → copy + unpin; miss → kalloc,
/// insert (pinned), readpage fill with the lock dropped, then copy +
/// unpin. A failed fill removes the page and returns the frame.
unsafe fn read_page_bytes(
    w: *mut VfsInode,
    index: u64,
    pgoff: usize,
    out: &mut [u8],
) -> VfsResult<()> {
    let m = unsafe { mapping_of(w) };
    // Fast path under one guard: hit copies and unpins.
    {
        let _g = PC_LOCK.lock();
        let pc = pcache();
        // SAFETY: live mapping.
        let pg = pc.lookup(&mut *m, index);
        if !pg.is_null() {
            // SAFETY: pinned by the lookup; frame is PGSIZE.
            unsafe {
                debug_assert!((*pg).uptodate, "pagecache: hit on !uptodate page");
                core::ptr::copy_nonoverlapping((*pg).data.add(pgoff), out.as_mut_ptr(), out.len());
                pc.put_page(pg);
            }
            return Ok(());
        }
    }
    // Miss: frame + descriptor first, fill second (no PC_LOCK across
    // the fs call — readpage's ilock may sleep).
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
            Err(VfsErr::Busy) => {
                // Inserted concurrently (UP: only a same-context retry);
                // share the winner's page and drop our frame.
                // SAFETY: live mapping.
                let winner = pc.lookup(&mut *m, index);
                kfree(frame);
                if !winner.is_null() {
                    // SAFETY: pinned; frame is PGSIZE.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            (*winner).data.add(pgoff),
                            out.as_mut_ptr(),
                            out.len(),
                        );
                        pc.put_page(winner);
                    }
                    return Ok(());
                }
                return Err(VfsErr::Busy);
            }
            Err(e) => return Err(e),
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
    if r.is_err() {
        let _g = PC_LOCK.lock();
        // SAFETY: we hold the insert pin.
        let data = unsafe { pcache().remove(&mut *m, pg) };
        kfree(data);
        return r;
    }
    {
        let _g = PC_LOCK.lock();
        let pc = pcache();
        // SAFETY: pinned; frame filled by readpage.
        unsafe {
            core::ptr::copy_nonoverlapping((*pg).data.add(pgoff), out.as_mut_ptr(), out.len());
            pc.put_page(pg);
        }
    }
    Ok(())
}
