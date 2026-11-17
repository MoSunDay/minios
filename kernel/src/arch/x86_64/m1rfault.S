/* M1R expected-fault injection points (S2, plans/linux26 40 §2.1.6).
 *
 * Every faulting instruction lives in its own .S symbol and is registered
 * in .m1r.fixup (uaccess.S paradigm), so the observed RIP is a fixed symbol
 * address regardless of -O2 reordering/inlining of Rust callers.
 *
 * Register contract at fault time (verdict-checked magics, r10 keeps the
 * syscall-ABI slot free): rax/rbx/rsi/rdx/r10 carry distinct constants; rdi
 * carries the caller argument — the faulting VA for pf.unmapped, the magic
 * RDI elsewhere. Each injection pushes a frame so the first backtrace entry
 * is the fixed _m1r_fault_driver_frame symbol.
 */
.set MRAX, 0xcafe000000000001
.set MRBX, 0xcafe000000000002
.set MRDI, 0xcafe000000000003
.set MRSI, 0xcafe000000000004
.set MRDX, 0xcafe000000000005
.set MR10, 0xcafe000000000006

.macro LOAD_MAGICS
    movabsq $MRAX, %rax
    movabsq $MRBX, %rbx
    movabsq $MRDI, %rdi
    movabsq $MRSI, %rsi
    movabsq $MRDX, %rdx
    movabsq $MR10, %r10
.endm

.macro M1RF_FIXUP_PAIR fault_label, fix_label
.section .m1r.fixup, "a"
    .quad \fault_label
    .quad \fix_label
.previous
.endm

/* One-instruction spacer: the first injector must not sit on KERNEL_VMA
 * itself — an address tie with the link-base symbol degrades rip
 * symbolization (prefix contract, decision D-3). */
.text
.globl _m1rf_prologue
_m1rf_prologue:
    xorl %eax, %eax
    ret

/* #PF not-present: read from the unmapped canonical user VA in rdi. */
.globl _m1rf_inject_pf_unmapped
_m1rf_inject_pf_unmapped:
    pushq %rbp
    movq %rsp, %rbp
    movabsq $MRAX, %rax
    movabsq $MRBX, %rbx
    movabsq $MRSI, %rsi
    movabsq $MRDX, %rdx
    movabsq $MR10, %r10
1:  movq (%rdi), %r8
2:  xorl %eax, %eax
    popq %rbp
    ret
    M1RF_FIXUP_PAIR 1b, 2b

/* #PF write-to-read-only: store to a byte in kernel .rodata. */
.text
.globl _m1rf_inject_pf_rodata
_m1rf_inject_pf_rodata:
    pushq %rbp
    movq %rsp, %rbp
    LOAD_MAGICS
    leaq _m1rf_rodata_target(%rip), %r8
1:  movb %dil, (%r8)
2:  xorl %eax, %eax
    popq %rbp
    ret
    M1RF_FIXUP_PAIR 1b, 2b

.pushsection .rodata
.globl _m1rf_rodata_target
_m1rf_rodata_target: .byte 0x5a
.popsection

/* #GP(0): data access through the non-canonical MAGIC_RAX (bits 63:48 =
 * 0xcafe). QEMU 6.2 TCG silently drops unknown-MSR WRMSR/RDMSR, so the
 * classic MSR #GP is unobtainable there; a non-canonical operand raises
 * #GP(0) deterministically on TCG and real hardware alike, with every
 * verdict register (including rax) intact. */
.text
.globl _m1rf_inject_gp_noncanon
_m1rf_inject_gp_noncanon:
    pushq %rbp
    movq %rsp, %rbp
    LOAD_MAGICS
1:  movq (%rax), %r8
2:  xorl %eax, %eax
    popq %rbp
    ret
    M1RF_FIXUP_PAIR 1b, 2b

/* #DE: unsigned divide by zero (rcx is not a verdict register). */
.text
.globl _m1rf_inject_de_div0
_m1rf_inject_de_div0:
    pushq %rbp
    movq %rsp, %rbp
    LOAD_MAGICS
    xorl %ecx, %ecx
1:  divq %rcx
2:  xorl %eax, %eax
    popq %rbp
    ret
    M1RF_FIXUP_PAIR 1b, 2b

/* #BP: software breakpoint — a trap, so the frame RIP is the *next*
 * instruction; the fixup pair registers that label. */
.text
.globl _m1rf_inject_bp_int3
_m1rf_inject_bp_int3:
    pushq %rbp
    movq %rsp, %rbp
    LOAD_MAGICS
    int3
1:  nop
2:  xorl %eax, %eax
    popq %rbp
    ret
    M1RF_FIXUP_PAIR 1b, 2b
