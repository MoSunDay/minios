//! `check-rust-only`: static gate proving the tree stays C-free after the
//! legacy C retirement (user/*.c, tools/mkfs.c, kernel legacy dirs). The
//! scan core is pure — violations come back as data, never a process exit;
//! the thin cmd prints the verdict. All rules frozen here: the six source
//! roots, the x86_64 asm allowlist, and the compiler-token rule for the
//! text-scan set (repo Makefile, scripts/, .cargo/*.toml, every
//! Cargo.toml under the repo root).

use std::path::{Path, PathBuf};

pub const CMD: &str = "check-rust-only";

/// The six scanned source roots, relative to the repo root.
const ROOTS: [&str; 6] = ["abi", "kernel", "user", "tools", "host-tests", "tests"];
/// C/C++ source extensions (compared lowercased).
const C_EXTS: [&str; 11] = [
    "c", "h", "cc", "cpp", "cxx", "c++", "hpp", "hh", "hxx", "i", "ii",
];
/// The only place .S/.s may live: the x86_64 kernel entry/trap asm.
const ASM_ALLOWLIST: &str = "kernel/src/arch/x86_64";
/// Exact compiler basenames; any basename ending in -gcc/-cc also counts.
const CC_NAMES: [&str; 6] = ["cc", "gcc", "clang", "clang++", "g++", "c++"];
/// Directories the recursive Cargo.toml walk never enters: VCS/internal
/// dot-dirs, the vendored link_repos reference tree, and cargo build
/// outputs (`target`, `target-user`). Not project sources.
fn walk_skipped(name: &str) -> bool {
    name.starts_with('.') || name == "link_repos" || name.starts_with("target")
}

/// One violation: kind + repo-relative path (+ the offending token for
/// compiler invocations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub kind: &'static str,
    pub path: String,
    pub word: Option<String>,
}

impl Violation {
    fn new(kind: &'static str, path: &str) -> Self {
        Violation {
            kind,
            path: path.to_string(),
            word: None,
        }
    }
}

/// Pure scan outcome: counts plus every violation as data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    /// Files seen under the six roots.
    pub files: u64,
    /// .S/.s files inside the allowlist dir (allowed asm).
    pub asm_allowlist: u64,
    pub violations: Vec<Violation>,
}

/// Scan `root` (the repo root). Missing roots, planted C/C++ sources,
/// asm outside the allowlist, and compiler invocations in the text-scan
/// set all come back as violations — this fn never exits the process.
pub fn scan(root: &Path) -> ScanResult {
    let mut res = ScanResult::default();
    for name in ROOTS {
        let dir = root.join(name);
        if !dir.is_dir() {
            res.violations.push(Violation::new("missing-root", name));
            continue;
        }
        walk_root(&dir, name, &mut res);
    }
    scan_text(root, &mut res);
    res
}

/// Recursive walker over one source root; `rel` is the repo-relative dir
/// path ("" for the root itself). Entries are sorted for determinism;
/// symlinks are treated as files (never followed into).
fn walk_root(dir: &Path, rel: &str, res: &mut ScanResult) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().display().to_string())
        .collect();
    names.sort();
    for name in names {
        let child = dir.join(&name);
        let rel_child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if child.is_dir() {
            walk_root(&child, &rel_child, res);
            continue;
        }
        res.files += 1;
        let ext = Path::new(&rel_child)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let Some(ext) = ext else { continue };
        if C_EXTS.contains(&ext.as_str()) {
            res.violations.push(Violation::new("c-source", &rel_child));
        } else if ext == "s" {
            if rel_child.starts_with(&format!("{ASM_ALLOWLIST}/")) {
                res.asm_allowlist += 1;
            } else {
                res.violations
                    .push(Violation::new("asm-outside-allowlist", &rel_child));
            }
        }
    }
}

/// Content scan: repo-root Makefile, every file under scripts/, every
/// .toml under .cargo/, and every Cargo.toml under the repo root
/// (recursively, minus walk_skipped dirs) — all sorted for determinism.
fn scan_text(root: &Path, res: &mut ScanResult) {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let makefile = root.join("Makefile");
    if makefile.is_file() {
        files.push(("Makefile".to_string(), makefile));
    }
    collect_under(&root.join("scripts"), "scripts", &mut files, &|_| true);
    collect_under(&root.join(".cargo"), ".cargo", &mut files, &|n: &str| {
        n.ends_with(".toml")
    });
    collect_cargo_tomls(root, "", &mut files);
    files.sort();
    for (rel, path) in files {
        scan_tokens(&rel, &path, res);
    }
}

/// Append `(rel/name, path)` for every file under `dir` (recursively)
/// whose file name passes `keep`; missing dirs are silently skipped.
fn collect_under(
    dir: &Path,
    rel: &str,
    out: &mut Vec<(String, PathBuf)>,
    keep: &impl Fn(&str) -> bool,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().display().to_string())
        .collect();
    names.sort();
    for name in names {
        let child = dir.join(&name);
        let rel_child = format!("{rel}/{name}");
        if child.is_dir() {
            collect_under(&child, &rel_child, out, keep);
        } else if keep(&name) {
            out.push((rel_child, child));
        }
    }
}

/// Every Cargo.toml under the repo root, skipping build/VCS/vendored dirs.
fn collect_cargo_tomls(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().display().to_string())
        .collect();
    names.sort();
    for name in names {
        if walk_skipped(&name) {
            continue;
        }
        let child = dir.join(&name);
        let rel_child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if child.is_dir() {
            collect_cargo_tomls(&child, &rel_child, out);
        } else if name == "Cargo.toml" {
            out.push((rel_child, child));
        }
    }
}

/// Token rule: split on whitespace; a token is a C compiler invocation
/// when its basename (after the last `/`) is exactly one of CC_NAMES or
/// ends with `-gcc`/`-cc` (cross prefixes like x86_64-linux-gnu-gcc).
fn is_cc_word(word: &str) -> bool {
    let base = word.rsplit('/').next().unwrap_or(word);
    CC_NAMES.contains(&base) || base.ends_with("-gcc") || base.ends_with("-cc")
}

fn scan_tokens(rel: &str, path: &Path, res: &mut ScanResult) {
    // Lossy read: binary junk simply never matches the token rule.
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    for word in text.split_whitespace() {
        if is_cc_word(word) {
            res.violations.push(Violation {
                kind: "compiler-invocation",
                path: rel.to_string(),
                word: Some(word.to_string()),
            });
        }
    }
}

/// Thin cmd: scan, print the one-line verdict, return the exit code
/// (same PASS-stdout / FAIL-stderr split as the coverage gate).
pub fn cmd(args: &[String]) -> i32 {
    let mut root = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => match args.get(i + 1) {
                Some(v) => {
                    root = PathBuf::from(v);
                    i += 1;
                }
                None => {
                    eprintln!("{CMD}: --root needs a value");
                    return 2;
                }
            },
            other => {
                eprintln!("{CMD}: unknown arg {other}");
                return 2;
            }
        }
        i += 1;
    }
    let res = scan(&root);
    if res.violations.is_empty() {
        println!(
            "{CMD}: PASS roots={} files={} asm-allowlist={} c-sources=0 compiler-invocations=0",
            ROOTS.len(),
            res.files,
            res.asm_allowlist
        );
        0
    } else {
        for v in &res.violations {
            match &v.word {
                Some(word) => eprintln!("{CMD}: FAIL {} {} {}", v.kind, v.path, word),
                None => eprintln!("{CMD}: FAIL {} {}", v.kind, v.path),
            }
        }
        eprintln!(
            "{CMD}: FAIL roots={} violations={}",
            ROOTS.len(),
            res.violations.len()
        );
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Temp fixture dir (no external deps): unique per test+pid+nanos.
    fn fixture(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minios-rustonly-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn put(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// A clean Rust-only tree: six roots + benign text-scan files.
    fn clean_tree(dir: &Path) {
        for r in ROOTS {
            put(dir, &format!("{r}/src/lib.rs"), "// rust only\n");
        }
        put(dir, "Cargo.toml", "[workspace]\nmembers = [\"abi\"]\n");
        put(dir, "Makefile", "build:\n\tcargo build --locked\n");
        put(dir, "scripts/run.sh", "qemu-system-x86_64 -serial stdio\n");
        put(dir, ".cargo/config.toml", "[build]\ntarget = \"none\"\n");
        put(dir, "abi/Cargo.toml", "[package]\nname = \"a\"\n");
    }

    fn kinds(res: &ScanResult) -> Vec<&'static str> {
        res.violations.iter().map(|v| v.kind).collect()
    }

    #[test]
    fn clean_tree_passes() {
        let dir = fixture("clean");
        clean_tree(&dir);
        let res = scan(&dir);
        assert_eq!(res.violations, Vec::new());
        assert_eq!(res.files, 7); // 6 x src/lib.rs + abi/Cargo.toml
        assert_eq!(res.asm_allowlist, 0);
    }

    #[test]
    fn planted_c_and_h_files_fail() {
        let dir = fixture("c-h");
        clean_tree(&dir);
        put(&dir, "user/src/cat.c", "int main(void) { return 0; }\n");
        put(&dir, "tools/src/types.H", "#pragma once\n"); // uppercased ext
        let res = scan(&dir);
        assert_eq!(kinds(&res), ["c-source", "c-source"]);
        // ROOTS array order (user before tools), names sorted within.
        assert_eq!(res.violations[0].path, "user/src/cat.c");
        assert_eq!(res.violations[1].path, "tools/src/types.H");
    }

    #[test]
    fn asm_inside_allowlist_passes_and_counts() {
        let dir = fixture("allowlist");
        clean_tree(&dir);
        put(
            &dir,
            "kernel/src/arch/x86_64/boot.S",
            ".globl entry\nentry:\n\tret\n",
        );
        let res = scan(&dir);
        assert_eq!(res.violations, Vec::new());
        assert_eq!(res.asm_allowlist, 1);
        assert_eq!(res.files, 8);
    }

    #[test]
    fn asm_outside_allowlist_fails() {
        let dir = fixture("outside");
        clean_tree(&dir);
        put(&dir, "kernel/src/boot.S", ".globl x\n");
        put(&dir, "tools/src/startup.s", "nop\n");
        let res = scan(&dir);
        assert_eq!(
            res.violations
                .iter()
                .map(|v| (v.kind, v.path.as_str()))
                .collect::<Vec<_>>(),
            [
                ("asm-outside-allowlist", "kernel/src/boot.S"),
                ("asm-outside-allowlist", "tools/src/startup.s"),
            ]
        );
    }

    #[test]
    fn compiler_invocations_in_text_scan_set_fail() {
        let dir = fixture("compilers");
        clean_tree(&dir);
        put(&dir, "Makefile", "build:\n\tgcc -o x x.c\n");
        put(&dir, "scripts/build.sh", "clang -O2 demo.c\n");
        // Unquoted word: the rule splits on whitespace only, so a quoted
        // value would not match — quotes are part of the token.
        put(&dir, ".cargo/config.toml", "[x]\nl = /cc/riscv64-gcc\n");
        let res = scan(&dir);
        assert_eq!(
            res.violations
                .iter()
                .map(|v| (v.kind, v.path.as_str(), v.word.as_deref()))
                .collect::<Vec<_>>(),
            [
                (
                    "compiler-invocation",
                    ".cargo/config.toml",
                    Some("/cc/riscv64-gcc")
                ),
                ("compiler-invocation", "Makefile", Some("gcc")),
                ("compiler-invocation", "scripts/build.sh", Some("clang")),
            ]
        );
    }

    #[test]
    fn missing_root_fails() {
        let dir = fixture("missing");
        clean_tree(&dir);
        std::fs::remove_dir_all(dir.join("host-tests")).unwrap();
        let res = scan(&dir);
        assert_eq!(kinds(&res), ["missing-root"]);
        assert_eq!(res.violations[0].path, "host-tests");
    }
}
