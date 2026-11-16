//! Static Multiboot1/ELF64 layout gate for the M0R kernel image
//! (plans/linux26 40 §1.1). Pure byte parsing, no I/O, no dependencies.

pub fn check_elf(bytes: &[u8]) -> Result<Report, String> {
    if bytes.len() < 64 {
        return Err("file shorter than ELF64 header".to_string());
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err("missing ELF magic".to_string());
    }
    if bytes[4] != 2 {
        return Err("not ELF64 (class != 2)".to_string());
    }
    if bytes[5] != 1 {
        return Err("not little-endian (data != 1)".to_string());
    }

    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32_at =
        |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let u64_at = |o: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&bytes[o..o + 8]);
        u64::from_le_bytes(b)
    };

    let e_type = u16_at(16);
    if e_type != 2 {
        return Err(format!("e_type {e_type} != ET_EXEC(2)"));
    }
    let e_entry = u64_at(24);
    if e_entry >= 1 << 32 {
        return Err(format!(
            "e_entry {e_entry:#x} is not a low 32-bit transition entry"
        ));
    }
    let e_phoff = u64_at(32) as usize;
    let e_phentsize = u16_at(54) as usize;
    let e_phnum = u16_at(56) as usize;
    if e_phentsize != 56 {
        return Err(format!("e_phentsize {e_phentsize} != 56"));
    }

    let mut loads: Vec<Load> = Vec::new();
    for i in 0..e_phnum {
        let off = e_phoff + i * e_phentsize;
        if off + 56 > bytes.len() {
            return Err("program header table out of bounds".to_string());
        }
        let p_type = u32_at(off);
        if p_type != 1 {
            continue; // PT_LOAD only
        }
        let p_offset = u64_at(off + 8) as usize;
        let p_filesz = u64_at(off + 32) as usize;
        let p_vaddr = u64_at(off + 16);
        let p_paddr = u64_at(off + 24);
        if p_paddr >= 1 << 32 {
            return Err(format!(
                "PT_LOAD p_paddr {p_paddr:#x} outside low 32-bit space"
            ));
        }
        loads.push(Load {
            p_offset,
            p_filesz,
            p_vaddr,
            p_paddr,
        });
    }
    if loads.is_empty() {
        return Err("no PT_LOAD segments".to_string());
    }
    let high_vma = loads.iter().map(|l| p_high(&l.p_vaddr)).any(|h| h);
    if !high_vma {
        return Err("no high-half kernel mapping (all p_vaddr low)".to_string());
    }

    let mb = find_multiboot(bytes)?
        .ok_or_else(|| "no Multiboot1 header in first 8192 bytes".to_string())?;
    let inside_load = loads
        .iter()
        .any(|l| mb.offset >= l.p_offset && mb.offset + 12 <= l.p_offset + l.p_filesz);
    if !inside_load {
        return Err("Multiboot header not inside a loadable segment".to_string());
    }

    Ok(Report {
        entry: e_entry,
        multiboot: mb,
        loads,
    })
}

fn p_high(vaddr: &u64) -> bool {
    *vaddr >= 0xffff_8000_0000_0000
}

#[derive(Debug)]
pub struct Load {
    pub p_offset: usize,
    pub p_filesz: usize,
    pub p_vaddr: u64,
    pub p_paddr: u64,
}

#[derive(Debug)]
pub struct MultibootHeader {
    pub offset: usize,
    pub magic: u32,
    pub flags: u32,
    pub checksum: u32,
}

#[derive(Debug)]
pub struct Report {
    pub entry: u64,
    pub multiboot: MultibootHeader,
    pub loads: Vec<Load>,
}

fn find_multiboot(bytes: &[u8]) -> Result<Option<MultibootHeader>, String> {
    let limit = bytes.len().min(8192);
    let mut offset = 0;
    while offset + 12 <= limit {
        let magic = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        if magic == 0x1BAD_B002 {
            let flags = u32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]);
            let checksum = u32::from_le_bytes([
                bytes[offset + 8],
                bytes[offset + 9],
                bytes[offset + 10],
                bytes[offset + 11],
            ]);
            let sum = magic.wrapping_add(flags).wrapping_add(checksum);
            if sum != 0 {
                return Err(format!(
                    "Multiboot header at {offset:#x}: magic+flags+checksum = {sum:#x} != 0"
                ));
            }
            return Ok(Some(MultibootHeader {
                offset,
                magic,
                flags,
                checksum,
            }));
        }
        offset += 4; // header must be 4-byte aligned in the file
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_elf(entry: u64, paddr: u64, vaddr: u64, mb_at: Option<usize>) -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = 2; // ELF64
        b[5] = 1; // LE
        b[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        b[24..32].copy_from_slice(&entry.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        b[54..56].copy_from_slice(&56u16.to_le_bytes()); // phentsize
        b[56..58].copy_from_slice(&1u16.to_le_bytes()); // phnum
                                                        // One PT_LOAD covering the whole file at `paddr`.
        let ph = 64;
        b[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        b[ph + 8..ph + 16].copy_from_slice(&0u64.to_le_bytes()); // offset
        b[ph + 16..ph + 24].copy_from_slice(&vaddr.to_le_bytes());
        b[ph + 24..ph + 32].copy_from_slice(&paddr.to_le_bytes());
        let filesz = b.len() as u64;
        b[ph + 32..ph + 40].copy_from_slice(&filesz.to_le_bytes()); // filesz
        if let Some(at) = mb_at {
            let flags = 3u32;
            let checksum = 0u32.wrapping_sub(0x1BAD_B002).wrapping_sub(flags);
            b[at..at + 4].copy_from_slice(&0x1BAD_B002u32.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
            b[at + 8..at + 12].copy_from_slice(&checksum.to_le_bytes());
        }
        b
    }

    #[test]
    fn valid_image_passes() {
        let b = synthetic_elf(0x10000c, 0x100000, 0xffff_ffff_8000_0000, Some(0x80));
        let r = check_elf(&b).expect("green");
        assert_eq!(r.entry, 0x10000c);
        assert_eq!(r.multiboot.offset, 0x80);
        assert_eq!(r.multiboot.flags, 3);
    }

    #[test]
    fn high_entry_is_red() {
        let b = synthetic_elf(
            0xffff_ffff_8000_0350,
            0x100000,
            0xffff_ffff_8000_0000,
            Some(0x80),
        );
        assert!(check_elf(&b).unwrap_err().contains("low 32-bit"));
    }

    #[test]
    fn high_paddr_is_red() {
        let b = synthetic_elf(
            0x10000c,
            0xffff_ffff_8000_0000,
            0xffff_ffff_8000_0000,
            Some(0x80),
        );
        assert!(check_elf(&b).unwrap_err().contains("p_paddr"));
    }

    #[test]
    fn missing_multiboot_is_red() {
        let b = synthetic_elf(0x10000c, 0x100000, 0xffff_ffff_8000_0000, None);
        assert!(check_elf(&b).unwrap_err().contains("Multiboot1 header"));
    }

    #[test]
    fn bad_checksum_is_red() {
        let mut b = synthetic_elf(0x10000c, 0x100000, 0xffff_ffff_8000_0000, Some(0x80));
        b[0x84] ^= 0xff; // corrupt flags byte -> checksum no longer sums to 0
        assert!(check_elf(&b).unwrap_err().contains("checksum"));
    }
}
