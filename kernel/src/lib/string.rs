//! string: faithful port of xv6 string.c.
//!
//! Raw-pointer signatures (`*mut u8`/`*const u8`) keep the C-level contract:
//! callers guarantee valid lengths. No classes, no traits, free functions only.

/// memset: fill `n` bytes at `dst` with the low byte of `c`.
///
/// (string.c:5 — the 4-byte `stosl` fast path is kept, written as plain
/// stores instead of inline `rep stosl`.)
#[no_mangle]
pub extern "C" fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8 {
    let c = (c & 0xFF) as u8;
    // SAFETY: caller guarantees [dst, dst+n) is writable.
    unsafe {
        if (dst as usize) % 4 == 0 && n % 4 == 0 {
            let c4 = ((c as u32) << 24) | ((c as u32) << 16) | ((c as u32) << 8) | c as u32;
            let mut p = dst as *mut u32;
            let mut i = n / 4;
            while i > 0 {
                p.write_volatile(c4);
                p = p.add(1);
                i -= 1;
            }
        } else {
            let mut p = dst;
            let mut i = n;
            while i > 0 {
                p.write_volatile(c);
                p = p.add(1);
                i -= 1;
            }
        }
    }
    dst
}

/// memmove: copy `n` bytes, safe when the regions overlap (string.c:24).
#[no_mangle]
pub extern "C" fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: caller guarantees [dst, dst+n) writable and [src, src+n)
    // readable; the backward loop below handles src < dst < src+n overlap.
    unsafe {
        let mut s = src;
        let mut d = dst;
        if (s as usize) < (d as usize) && s.add(n) > d {
            // overlap and dst above src: copy backwards
            s = s.add(n);
            d = d.add(n);
            let mut i = n;
            while i > 0 {
                i -= 1;
                d = d.sub(1);
                s = s.sub(1);
                d.write_volatile(s.read_volatile());
            }
        } else {
            let mut i = n;
            while i > 0 {
                i -= 1;
                d.write_volatile(s.read_volatile());
                d = d.add(1);
                s = s.add(1);
            }
        }
    }
    dst
}

/// memcpy exists to placate GCC; use memmove (string.c:45).
#[no_mangle]
pub extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    memmove(dst, src, n)
}

/// memcmp: bytewise compare, `*s1 - *s2` of the first differing byte
/// (string.c:13).
#[no_mangle]
pub extern "C" fn memcmp(v1: *const u8, v2: *const u8, n: usize) -> i32 {
    // SAFETY: caller guarantees n readable bytes at each pointer.
    unsafe {
        let mut s1 = v1;
        let mut s2 = v2;
        let mut i = n;
        while i > 0 {
            i -= 1;
            let a = s1.read_volatile();
            let b = s2.read_volatile();
            if a != b {
                return a as i32 - b as i32;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
        }
    }
    0
}

/// strncmp: compare up to `n` bytes, stopping at NUL or difference
/// (string.c:51).
#[allow(dead_code)] // string.c port; namecmp uses its own inline comparison
pub fn strncmp(p: *const u8, q: *const u8, n: usize) -> i32 {
    // SAFETY: caller guarantees NUL-terminated-or-n-long readable strings.
    unsafe {
        let mut p = p;
        let mut q = q;
        let mut n = n;
        while n > 0 && *p != 0 && *p == *q {
            n -= 1;
            p = p.add(1);
            q = q.add(1);
        }
        if n == 0 {
            return 0;
        }
        *p as i32 - *q as i32
    }
}

/// safestrcpy: like strncpy but guaranteed to NUL-terminate (string.c:75).
pub fn safestrcpy(s: *mut u8, t: *const u8, n: i32) -> *mut u8 {
    // SAFETY: caller guarantees n writable bytes at s and a NUL-terminated
    // readable string at t.
    unsafe {
        let os = s;
        if n <= 0 {
            return os;
        }
        let mut s = s;
        let mut t = t;
        let mut n = n;
        while {
            n -= 1;
            n > 0
        } {
            let c = *t;
            *s = c;
            s = s.add(1);
            t = t.add(1);
            if c == 0 {
                break;
            }
        }
        *s = 0;
    }
    s
}

/// strlen: count bytes before the NUL (string.c:90). xv6 returns `int`.
#[no_mangle]
pub extern "C" fn strlen(s: *const u8) -> usize {
    // SAFETY: caller guarantees NUL-terminated string.
    unsafe {
        let mut p = s;
        let mut n: usize = 0;
        while *p != 0 {
            n += 1;
            p = p.add(1);
        }
        n
    }
}
