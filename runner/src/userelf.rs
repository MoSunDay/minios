//! Static user-binary ELF gate for the M1R user chain (plans/linux26/04
//! §2): every minios-owned user binary must be a static non-PIE ET_EXEC
//! for x86-64 — no PT_INTERP, no PT_DYNAMIC — with at least one PT_LOAD
//! and an entry inside a PT_LOAD range. Pure byte parsing, no I/O.

const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;

pub struct UserReport {
    pub entry: u64,
    pub loads: usize,
    pub max_end: u64,
}

pub fn check_user_elf(bytes: &[u8]) -> Result<UserReport, String> {
    if bytes.len() < 64 {
        return Err("file shorter than ELF64 header".to_string());
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err("missing ELF magic".to_string());
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("not ELF64 little-endian".to_string());
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
    if e_type != ET_EXEC {
        return Err(format!("e_type {e_type} != ET_EXEC (PIE/dynamic build?)"));
    }
    if u16_at(18) != EM_X86_64 {
        return Err("e_machine != EM_X86_64".to_string());
    }
    let entry = u64_at(24);
    let phoff = u64_at(32) as usize;
    let phentsize = u16_at(54) as usize;
    let phnum = u16_at(56) as usize;
    if phentsize != 56 {
        return Err(format!("e_phentsize {phentsize} != 56"));
    }
    if phoff
        .checked_add(phnum * phentsize)
        .is_none_or(|e| e > bytes.len())
    {
        return Err("program headers out of file bounds".to_string());
    }

    let mut loads = 0usize;
    let mut max_end = 0u64;
    let mut entry_ok = false;
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        let p_type = u32_at(o);
        if p_type == PT_INTERP {
            return Err("PT_INTERP present (dynamic loader; forbidden in M1R)".to_string());
        }
        if p_type == PT_DYNAMIC {
            return Err("PT_DYNAMIC present (not a static image)".to_string());
        }
        if p_type != PT_LOAD {
            continue;
        }
        loads += 1;
        let vaddr = u64_at(o + 16);
        let memsz = u64_at(o + 40);
        let end = vaddr.checked_add(memsz).ok_or("PT_LOAD wraps u64")?;
        max_end = max_end.max(end);
        if entry >= vaddr && entry < end {
            entry_ok = true;
        }
    }
    if loads == 0 {
        return Err("no PT_LOAD segments".to_string());
    }
    if !entry_ok {
        return Err(format!("entry {entry:#x} outside every PT_LOAD"));
    }
    if max_end >= 0x0000_7000_0000_0000 {
        return Err(format!(
            "image extends to {max_end:#x} (kernel half / stack area)"
        ));
    }
    Ok(UserReport {
        entry,
        loads,
        max_end,
    })
}
