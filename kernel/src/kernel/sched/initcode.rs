//! initcode.S port: the first user program, execs "/init".
//!
//! xv6 links initcode.o into the kernel and maps its bytes at user va 0 in
//! userinit(). Here the linked image is emitted as a byte array with the
//! instruction-by-instruction commentary; the bytes are identical to the
//! assembled xv6 `initcode` binary (verified against the reference build):
//!
//! ```text
//! start:
//!   0: pushl $argv        ; 68 24 00 00 00   (argv at 0x24)
//!   5: pushl $init        ; 68 1c 00 00 00   ("/init" at 0x1c)
//!  10: pushl $0           ; 6a 00            fake return PC
//!  12: movl $SYS_exec,%eax; b8 07 00 00 00   SYS_exec == 7
//!  17: int  $T_SYSCALL    ; cd 40            T_SYSCALL == 64
//! exit:                   ;                  for(;;) exit();
//!  19: movl $SYS_exit,%eax; b8 02 00 00 00   SYS_exit == 2
//!  24: int  $T_SYSCALL    ; cd 40
//!  26: jmp  exit          ; eb f7            (jmp -9)
//! init:
//!  28: "/init\0"          ; 2f 69 6e 69 74 00
//!  34: .p2align padding   ; 00 90
//! argv:
//!  36: .long init         ; 1c 00 00 00      argv[0]
//!  40: .long 0            ; 00 00 00 00      argv[1] == 0 sentinel
//! ```
//!
//! Total 44 bytes; userinit maps one page and runs it at eip 0, esp PGSIZE.

/// The first user program: `exec("/init", ["/init", 0])`, then exit loop.
pub static INITCODE: &[u8] = &[
    0x68, 0x24, 0x00, 0x00, 0x00, // 0:  push $0x24 (argv)
    0x68, 0x1c, 0x00, 0x00, 0x00, // 5:  push $0x1c (init "/init\0")
    0x6a, 0x00, // 10: push $0 (fake caller pc)
    0xb8, 0x07, 0x00, 0x00, 0x00, // 12: mov $SYS_exec(7), %eax
    0xcd, 0x40, // 17: int $T_SYSCALL(64)
    0xb8, 0x02, 0x00, 0x00, 0x00, // 19: mov $SYS_exit(2), %eax
    0xcd, 0x40, // 24: int $T_SYSCALL
    0xeb, 0xf7, // 26: jmp exit
    0x2f, 0x69, 0x6e, 0x69, 0x74, 0x00, // 28: init: "/init\0"
    0x00, 0x90, // 34: .p2align padding
    0x1c, 0x00, 0x00, 0x00, // 36: argv[0] = init
    0x00, 0x00, 0x00, 0x00, // 40: argv[1] = 0
];
