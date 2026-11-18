//! Embedded user images (S3): the execve image source until the S4 file
//! system. build.rs copies the linked user ELFs from the Makefile-driven
//! user target dir into OUT_DIR/userimg; the kernel embeds them by
//! include_bytes. Empty files mean "not built yet" and every lookup
//! answers None (the suite then fails loudly instead of booting garbage).

/// An embedded image with its exec path.
pub struct UserImage {
    pub path: &'static str,
    pub elf: &'static [u8],
}

const SUITE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/userimg/m1r-suite"));
const EXEC_CHILD_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/userimg/m1r-exec-child"));
const USERTESTS_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/userimg/usertests"));
const INIT_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/userimg/init"));

/// The embedded image table (S3 image source; S4 adds the fs lookup).
pub const IMAGES: &[UserImage] = &[
    UserImage {
        path: "/m1r-suite",
        elf: SUITE_ELF,
    },
    UserImage {
        path: "/m1r-exec-child",
        elf: EXEC_CHILD_ELF,
    },
    UserImage {
        path: "/usertests",
        elf: USERTESTS_ELF,
    },
    // BootMode::Full (S4e-4): the pid-1 seed; /sh is then execve'd from
    // the real xv6fs root image like every later program.
    UserImage {
        path: "/init",
        elf: INIT_ELF,
    },
];

/// Look up `path` (NUL-free byte string, no trailing NUL) in the table.
pub fn lookup(path: &[u8]) -> Option<&'static [u8]> {
    for img in IMAGES {
        if img.elf.is_empty() {
            continue; // placeholder written before the first user build
        }
        if img.path.as_bytes() == path {
            return Some(img.elf);
        }
    }
    None
}
