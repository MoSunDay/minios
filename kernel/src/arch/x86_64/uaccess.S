/* UserPtr copy primitives with fault fixup (plans/linux26/20 M1R:
 * copyin/copyout/copyinstr convert #PF/#GP at the registered instruction
 * into -EFAULT instead of oopsing). Each fixup entry is a (fault_rip,
 * fixup_rip) pair in .m1r.fixup, bounded by __fixup_start/__fixup_end.
 *
 * rdi=dst, rsi=src, rdx=len on entry; rax=0 or -EFAULT on return.
 * clobbers rcx, rax, flags; direction flag cleared.
 */
.set EFAULT_NEG, -14

.macro FIXUP_PAIR fault_label, fix_label
.section .m1r.fixup, "a"
    .quad \fault_label
    .quad \fix_label
.previous
.endm

.text
.globl _copyout_bytes
_copyout_bytes:
    cld
    movq %rdx, %rcx
    testq %rcx, %rcx
    jz 2f
1:  rep movsb
2:  xorl %eax, %eax
    ret
3:  movq $EFAULT_NEG, %rax
    ret
    FIXUP_PAIR 1b, 3b

.text
.globl _copyin_bytes
_copyin_bytes:
    cld
    movq %rdx, %rcx
    testq %rcx, %rcx
    jz 2f
1:  rep movsb
2:  xorl %eax, %eax
    ret
3:  movq $EFAULT_NEG, %rax
    ret
    FIXUP_PAIR 1b, 3b

/* copyinstr: copy at most rdx bytes, stopping after the first NUL
 * (which is copied). Returns 0 on success, -EFAULT on fault, -ENAMETOOLONG
 * (-36) when no NUL was found within the limit. */
.set ENAMETOOLONG_NEG, -36
.text
.globl _copyinstr
_copyinstr:
    cld
    xorl %eax, %eax
    testq %rdx, %rdx
    jz 5f
    xorl %ecx, %ecx
1:  movb (%rsi), %al
2:  movb %al, (%rdi)
    incq %rsi
    incq %rdi
    incq %rcx
    testb %al, %al
    jz 4f
    cmpq %rdx, %rcx
    jb 1b
    /* limit reached without a terminator */
    movq $ENAMETOOLONG_NEG, %rax
    ret
4:  xorl %eax, %eax
    ret
5:  xorl %eax, %eax
    ret
3:  movq $EFAULT_NEG, %rax
    ret
    FIXUP_PAIR 1b, 3b
    FIXUP_PAIR 2b, 3b

/* m1r_probe_read: never-faulting kernel read of one byte (expected-fault
 * suite and UserPtr probes). Returns the byte value or -EFAULT. */
.text
.globl _m1r_probe_read
_m1r_probe_read:
    xorl %eax, %eax
1:  movb (%rdi), %al
    ret
2:  movq $EFAULT_NEG, %rax
    ret
    FIXUP_PAIR 1b, 2b

/* m1r_probe_write: never-faulting kernel write of one byte (sil). */
.text
.globl _m1r_probe_write
_m1r_probe_write:
1:  movb %sil, (%rdi)
    xorl %eax, %eax
    ret
2:  movq $EFAULT_NEG, %rax
    ret
    FIXUP_PAIR 1b, 2b
