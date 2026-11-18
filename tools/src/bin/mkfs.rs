//! minios-mkfs: build the xv6 root image (plans/linux26/04 §5).
//! Usage: minios-mkfs <fs.img> <file>... — files land as root-directory
//! entries in argument order, contents verbatim. Deterministic: same
//! inputs, same bytes.

use minios_tools::{build_image, InputFile};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: minios-mkfs <fs.img> <file>...");
        std::process::exit(1);
    }
    let out = &args[1];
    let mut files = Vec::new();
    for path in &args[2..] {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{path}: {e}");
                std::process::exit(1);
            }
        };
        // mkfs.c parity: strip a leading '_' so build-tree artifacts can
        // dodge host PATH shadows (_cat, _rm...) without renaming on disk.
        let name = path.strip_prefix("./").unwrap_or(path);
        let name = name.strip_prefix('_').unwrap_or(name);
        let name = name.rsplit('/').next().unwrap_or(name);
        files.push((name.to_string(), data));
    }
    let inputs: Vec<InputFile<'_>> = files
        .iter()
        .map(|(n, d)| InputFile { name: n, data: d })
        .collect();
    let (img, sb, usedblocks) = build_image(&inputs);
    if let Err(e) = std::fs::write(out, &img) {
        eprintln!("{out}: {e}");
        std::process::exit(1);
    }
    // Summary parity with mkfs.c: the accounting line reports the
    // pre-data state (metadata blocks only), balloc reports the final
    // bitmap fill.
    let bitblocks = sb.size / (512 * 8) + 1;
    let meta_used = sb.ninodes / 8 + 3 + bitblocks;
    println!(
        "used {} (bit {} ninode {}) free {} log {} total {}",
        meta_used,
        bitblocks,
        sb.ninodes / 8 + 1,
        meta_used,
        sb.nlog,
        sb.nblocks + meta_used + sb.nlog
    );
    println!("balloc: first {usedblocks} blocks have been allocated");
    println!("balloc: write bitmap block at sector {}", sb.bmapstart);
}
