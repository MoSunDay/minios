//! Initial-stack argument walking (plans/linux26/04 §3): argc/argv[]
//! plus the envp[]/auxv cursors of the Linux start ABI.

pub struct Args {
    argc: usize,
    argv: *const u64,
    /// Cursor into the auxv chain (key/value pairs, AT_NULL terminated).
    auxv: *const u64,
}

/// Safety: `argv` must point at a NULL-terminated pointer array followed
/// by a NULL-terminated envp array and an AT_NULL-terminated auxv chain
/// (the layout exec seeds and _start passes on).
pub unsafe fn from_stack(argc: usize, argv: *const u64) -> Args {
    // SAFETY: see contract above.
    unsafe {
        let mut p = argv.add(argc); // argv[argc] == NULL
        p = p.add(1); // skip the argv NULL
        while *p != 0 {
            p = p.add(1); // skip envp strings' pointers
        }
        p = p.add(1); // skip the envp NULL -> auxv begins
        Args {
            argc,
            argv,
            auxv: p,
        }
    }
}

impl Args {
    /// Argument count as passed in argc.
    pub fn argc(&self) -> usize {
        self.argc
    }

    /// Argument `i` as a NUL-terminated byte string slice, None when the
    /// pointer is NULL or the bytes are not a valid in-bounds string.
    pub fn arg(&self, i: usize) -> Option<&[u8]> {
        if i >= self.argc {
            return None;
        }
        // SAFETY: argv[i] is a user pointer to a NUL-terminated string.
        let p = unsafe { *self.argv.add(i) } as *const u8;
        if p.is_null() {
            return None;
        }
        let mut len = 0usize;
        // SAFETY: bounded scan over the caller's own stack strings.
        unsafe {
            while *p.add(len) != 0 && len < 4096 {
                len += 1;
            }
        }
        // SAFETY: len bytes readable by construction.
        Some(unsafe { core::slice::from_raw_parts(p, len) })
    }

    /// auxv lookup: the value for `key`, None when absent.
    pub fn auxv(&self, key: u64) -> Option<u64> {
        let mut p = self.auxv;
        // SAFETY: chain is AT_NULL terminated.
        unsafe {
            loop {
                let k = *p;
                let v = *p.add(1);
                if k == abi::AT_NULL {
                    return None;
                }
                if k == key {
                    return Some(v);
                }
                p = p.add(2);
            }
        }
    }
}
