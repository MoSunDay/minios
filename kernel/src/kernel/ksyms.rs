//! Symbol lookup over the fixed-size kernel symbol table generated at
//! build time from the kernel ELF (risk R-D: no external nm dependency;
//! table layout is size-stable so a second link pass converges).
//!
//! This module is pure and host-testable (host-tests includes it by path).

/// Capacity of the generated table; the generator pads to exactly this many
/// entries so the kernel layout never shifts between generation passes.
pub const KSYM_CAP: usize = 2048;
/// Width of one inline symbol-name slot (bytes, NUL padded).
pub const KSYM_NAME: usize = 56;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ksym {
    pub addr: u64,
    pub name: [u8; KSYM_NAME],
}

impl Ksym {
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&c| c == 0).unwrap_or(KSYM_NAME);
        core::str::from_utf8(&self.name[..end]).unwrap_or("?")
    }
}

/// Binary search the largest entry with `entry.addr <= addr`.
/// Returns None when the table is empty or addr sorts before the first entry.
pub fn lookup(table: &[Ksym], addr: u64) -> Option<(u64, &Ksym)> {
    if table.is_empty() || addr < table[0].addr {
        return None;
    }
    let (mut lo, mut hi) = (0usize, table.len() - 1);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if table[mid].addr <= addr {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some((addr - table[lo].addr, &table[lo]))
}

/// Format a symbolized address as `name+0xoff` (or `?0xADDR` when unknown)
/// into `out`; returns the number of bytes written.
pub fn symbolize_into(table: &[Ksym], addr: u64, out: &mut [u8]) -> usize {
    let mut buf = [0u8; 96];
    let mut n = 0usize;
    let mut put = |b: u8| {
        if n < buf.len() {
            buf[n] = b;
            n += 1;
        }
    };
    let target: u64;
    match lookup(table, addr) {
        Some((off, sym)) => {
            for &b in sym.name_str().as_bytes() {
                put(b);
            }
            put(b'+');
            put(b'0');
            put(b'x');
            target = off;
        }
        None => {
            put(b'?');
            put(b'0');
            put(b'x');
            target = addr;
        }
    }
    let hex = b"0123456789abcdef";
    let mut digits = [0u8; 16];
    let mut d = 0usize;
    let mut v = target;
    if v == 0 {
        digits[d] = b'0';
        d += 1;
    }
    while v > 0 && d < digits.len() {
        digits[d] = hex[(v & 0xf) as usize];
        d += 1;
        v >>= 4;
    }
    while d > 0 {
        d -= 1;
        put(digits[d]);
    }
    let n = n.min(out.len());
    out[..n].copy_from_slice(&buf[..n]);
    n
}
