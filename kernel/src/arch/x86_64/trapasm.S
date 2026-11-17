/*
 * M1R trap/syscall entry paths (plans/linux26/21 §1, 04 §3).
 *
 * Trapframe on the kernel stack (repr(C) twin in kernel/traps.rs):
 *   [rdi rsi rdx rcx rax r8 r9 r10 r11 rbx rbp r12 r13 r14 r15]
 *   [vector][err][rip][cs][rflags][rsp][ss]
 * Frame offsets (bytes, compile-time asserted on the Rust side):
 *   gprs 0..119, vector 120, err 128, rip 136, cs 144,
 *   rflags 152, rsp 160, ss 168.
 *
 * swapgs discipline: exactly one swapgs per user<->kernel crossing.
 * The syscall entry runs with interrupts off from the first instruction,
 * closing the swapgs/stack-switch window (risk R-C in plans/linux26/50).
 * 64-bit iretq always pops five frame slots, so kernel-mode interrupt
 * frames are normalized to the full tail by shifting the saved frame
 * down 16 bytes and synthesizing rsp/ss.
 */

/* This file holds only the per-vector stub table; the shared entry,
 * syscall path, and return path live in trapcommon.S (both assembled as
 * one logical unit; cross-references resolve at link time). */
.macro VEC n, has_ec
.text
.globl _vector_\n
_vector_\n:
.if \has_ec
    /* hardware already pushed the error code below rip */
    pushq $\n
.else
    pushq $0
    pushq $\n
.endif
    jmp isr_common
.pushsection .rodata
    .quad _vector_\n
.popsection
.endm

.pushsection .rodata
.globl _vector_table
_vector_table:
.popsection

VEC 0, 0
VEC 1, 0
VEC 2, 0
VEC 3, 0
VEC 4, 0
VEC 5, 0
VEC 6, 0
VEC 7, 0
VEC 8, 1
VEC 9, 0
VEC 10, 1
VEC 11, 1
VEC 12, 1
VEC 13, 1
VEC 14, 1
VEC 15, 0
VEC 16, 0
VEC 17, 1
VEC 18, 0
VEC 19, 0
VEC 20, 0
VEC 21, 1
VEC 22, 0
VEC 23, 0
VEC 24, 0
VEC 25, 0
VEC 26, 0
VEC 27, 0
VEC 28, 0
VEC 29, 1
VEC 30, 1
VEC 31, 0
VEC 32, 0
VEC 33, 0
VEC 34, 0
VEC 35, 0
VEC 36, 0
VEC 37, 0
VEC 38, 0
VEC 39, 0
VEC 40, 0
VEC 41, 0
VEC 42, 0
VEC 43, 0
VEC 44, 0
VEC 45, 0
VEC 46, 0
VEC 47, 0
VEC 48, 0
VEC 49, 0
VEC 50, 0
VEC 51, 0
VEC 52, 0
VEC 53, 0
VEC 54, 0
VEC 55, 0
VEC 56, 0
VEC 57, 0
VEC 58, 0
VEC 59, 0
VEC 60, 0
VEC 61, 0
VEC 62, 0
VEC 63, 0
VEC 64, 0
VEC 65, 0
VEC 66, 0
VEC 67, 0
VEC 68, 0
VEC 69, 0
VEC 70, 0
VEC 71, 0
VEC 72, 0
VEC 73, 0
VEC 74, 0
VEC 75, 0
VEC 76, 0
VEC 77, 0
VEC 78, 0
VEC 79, 0
VEC 80, 0
VEC 81, 0
VEC 82, 0
VEC 83, 0
VEC 84, 0
VEC 85, 0
VEC 86, 0
VEC 87, 0
VEC 88, 0
VEC 89, 0
VEC 90, 0
VEC 91, 0
VEC 92, 0
VEC 93, 0
VEC 94, 0
VEC 95, 0
VEC 96, 0
VEC 97, 0
VEC 98, 0
VEC 99, 0
VEC 100, 0
VEC 101, 0
VEC 102, 0
VEC 103, 0
VEC 104, 0
VEC 105, 0
VEC 106, 0
VEC 107, 0
VEC 108, 0
VEC 109, 0
VEC 110, 0
VEC 111, 0
VEC 112, 0
VEC 113, 0
VEC 114, 0
VEC 115, 0
VEC 116, 0
VEC 117, 0
VEC 118, 0
VEC 119, 0
VEC 120, 0
VEC 121, 0
VEC 122, 0
VEC 123, 0
VEC 124, 0
VEC 125, 0
VEC 126, 0
VEC 127, 0
VEC 128, 0
VEC 129, 0
VEC 130, 0
VEC 131, 0
VEC 132, 0
VEC 133, 0
VEC 134, 0
VEC 135, 0
VEC 136, 0
VEC 137, 0
VEC 138, 0
VEC 139, 0
VEC 140, 0
VEC 141, 0
VEC 142, 0
VEC 143, 0
VEC 144, 0
VEC 145, 0
VEC 146, 0
VEC 147, 0
VEC 148, 0
VEC 149, 0
VEC 150, 0
VEC 151, 0
VEC 152, 0
VEC 153, 0
VEC 154, 0
VEC 155, 0
VEC 156, 0
VEC 157, 0
VEC 158, 0
VEC 159, 0
VEC 160, 0
VEC 161, 0
VEC 162, 0
VEC 163, 0
VEC 164, 0
VEC 165, 0
VEC 166, 0
VEC 167, 0
VEC 168, 0
VEC 169, 0
VEC 170, 0
VEC 171, 0
VEC 172, 0
VEC 173, 0
VEC 174, 0
VEC 175, 0
VEC 176, 0
VEC 177, 0
VEC 178, 0
VEC 179, 0
VEC 180, 0
VEC 181, 0
VEC 182, 0
VEC 183, 0
VEC 184, 0
VEC 185, 0
VEC 186, 0
VEC 187, 0
VEC 188, 0
VEC 189, 0
VEC 190, 0
VEC 191, 0
VEC 192, 0
VEC 193, 0
VEC 194, 0
VEC 195, 0
VEC 196, 0
VEC 197, 0
VEC 198, 0
VEC 199, 0
VEC 200, 0
VEC 201, 0
VEC 202, 0
VEC 203, 0
VEC 204, 0
VEC 205, 0
VEC 206, 0
VEC 207, 0
VEC 208, 0
VEC 209, 0
VEC 210, 0
VEC 211, 0
VEC 212, 0
VEC 213, 0
VEC 214, 0
VEC 215, 0
VEC 216, 0
VEC 217, 0
VEC 218, 0
VEC 219, 0
VEC 220, 0
VEC 221, 0
VEC 222, 0
VEC 223, 0
VEC 224, 0
VEC 225, 0
VEC 226, 0
VEC 227, 0
VEC 228, 0
VEC 229, 0
VEC 230, 0
VEC 231, 0
VEC 232, 0
VEC 233, 0
VEC 234, 0
VEC 235, 0
VEC 236, 0
VEC 237, 0
VEC 238, 0
VEC 239, 0
VEC 240, 0
VEC 241, 0
VEC 242, 0
VEC 243, 0
VEC 244, 0
VEC 245, 0
VEC 246, 0
VEC 247, 0
VEC 248, 0
VEC 249, 0
VEC 250, 0
VEC 251, 0
VEC 252, 0
VEC 253, 0
VEC 254, 0
VEC 255, 0

