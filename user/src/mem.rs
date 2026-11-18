//! C ABI memory intrinsics (no libc on the linux-gnu codegen target): the
//! compiler lowers slice writes/reads and bss clearing onto these.

/// SAFETY contract: standard memset semantics over valid memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut u8, val: i32, len: usize) -> *mut u8 {
    // SAFETY: caller-contracted dst..dst+len.
    unsafe {
        let b = val as u8;
        for i in 0..len {
            dst.add(i).write_volatile(b);
        }
        dst
    }
}

/// SAFETY contract: standard memcpy semantics (non-overlapping).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    // SAFETY: caller-contracted ranges.
    unsafe {
        for i in 0..len {
            dst.add(i).write_volatile(src.add(i).read_volatile());
        }
        dst
    }
}

/// SAFETY contract: standard memmove semantics (overlapping allowed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    // SAFETY: caller-contracted ranges; direction handles overlap.
    unsafe {
        if (dst as usize) < (src as usize) {
            for i in 0..len {
                dst.add(i).write_volatile(src.add(i).read_volatile());
            }
        } else {
            for i in (0..len).rev() {
                dst.add(i).write_volatile(src.add(i).read_volatile());
            }
        }
        dst
    }
}

/// SAFETY contract: standard memcmp semantics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, len: usize) -> i32 {
    // SAFETY: caller-contracted readable ranges.
    unsafe {
        for i in 0..len {
            let (x, y) = (a.add(i).read_volatile(), b.add(i).read_volatile());
            if x != y {
                return x as i32 - y as i32;
            }
        }
        0
    }
}
