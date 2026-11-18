/*
 * M1R trap/syscall common paths: frame constants, register save/restore,
 * the C handoff, the syscall entry, and the shared return path
 * (sysretq/iretq). The per-vector stubs live in trapasm.S.
 */
.set T_SYSCALL, 0x80
.set TF_VECTOR, 120
.set TF_RIP,    136
.set TF_CS,     144
.set TF_RFLAGS, 152
.set TF_RSP,    160
.set TF_SS,     168
.set PERCPU_USER_RSP,   0x08
.set PERCPU_KERNEL_RSP, 0x00
.set SEL_KDATA, 0x10

.macro push_all_regs
    pushq %r15
    pushq %r14
    pushq %r13
    pushq %r12
    pushq %rbp
    pushq %rbx
    pushq %r11
    pushq %r10
    pushq %r9
    pushq %r8
    pushq %rax
    pushq %rcx
    pushq %rdx
    pushq %rsi
    pushq %rdi
.endm

.macro pop_all_regs
    popq %rdi
    popq %rsi
    popq %rdx
    popq %rcx
    popq %rax
    popq %r8
    popq %r9
    popq %r10
    popq %r11
    popq %rbx
    popq %rbp
    popq %r12
    popq %r13
    popq %r14
    popq %r15
.endm

/* --- common interrupt entry ---------------------------------------------
 * Stub stack: [vec][err][rip][cs][rflags]        (from kernel)
 *              [vec][err][rip][cs][rflags][rsp][ss]  (from user)
 */
.text
isr_common:
    testb $3, 24(%rsp)        /* cs of the interrupted frame */
    jz .Lfrom_kernel
    swapgs
    push_all_regs
    jmp .Lframe_ready
.Lfrom_kernel:
    push_all_regs
    /* Shift the whole 20-qword frame down 16 bytes and synthesize rsp/ss
     * (64-bit iretq pops all five tail slots). Registers are free: every
     * GPR is already saved inside the frame being copied. */
    subq $16, %rsp
    movq %rsp, %rdi
    leaq 16(%rsp), %rsi
    movl $20, %ecx
    cld
    rep movsq
    leaq 176(%rsp), %rax      /* rsp at trap time: frame base + 176 */
    movq %rax, 160(%rsp)
    movq $SEL_KDATA, 168(%rsp)
.Lframe_ready:
    movq %rsp, %rdi
    cld
    call traps_dispatch
    jmp trapret_common

/* --- syscall entry --------------------------------------------------------
 * Runs with IF still at its user value until the explicit cli; the only
 * instructions between swapgs and the kernel stack are the two GS slots.
 */
.text
.globl _syscall_entry
_syscall_entry:
    swapgs
    cli
    movq %rsp, %gs:PERCPU_USER_RSP
    movq %gs:PERCPU_KERNEL_RSP, %rsp
    /* hardware set rcx=user rip, r11=user rflags */
    pushq $0x2b               /* ss  */
    pushq %gs:PERCPU_USER_RSP /* rsp */
    pushq %r11                /* rflags */
    pushq $0x33               /* cs  */
    pushq %rcx                /* rip */
    pushq $0                  /* err */
    pushq $T_SYSCALL          /* vector */
    push_all_regs
    movq %rsp, %rdi
    cld
    call syscall_dispatch
    jmp trapret_common

/* --- shared return path -----------------------------------------------------
 * sysretq only when: vector==T_SYSCALL, cs==0x33, ss==0x2b, rip < 2^47
 * (canonical user address), and the forced-iret hook (m1r-fault boot mode)
 * is clear. Everything else, including all interrupt returns and signal-like
 * exception returns, goes through iretq.
 */
.text
trapret_common:
    cmpq $T_SYSCALL, TF_VECTOR(%rsp)
    jne .Lreturn_iret
    cmpq $0x33, TF_CS(%rsp)
    jne .Lreturn_iret
    cmpq $0x2b, TF_SS(%rsp)
    jne .Lreturn_iret
    movq TF_RIP(%rsp), %rax
    movabsq $0x0000800000000000, %rdx
    cmpq %rdx, %rax           /* canonical user rip check */
    jae .Lreturn_iret
    cmpb $0, m1r_force_iret(%rip)
    jne .Lreturn_iret
    /* sysretq path */
    pop_all_regs
    addq $16, %rsp            /* stack now: [rip][cs][rflags][rsp][ss] */
    movq 0(%rsp), %rcx        /* rip */
    movq 16(%rsp), %r11       /* rflags */
    movq 24(%rsp), %rax       /* user rsp */
    movq %rax, %rsp
    swapgs
    sysretq
.Lreturn_iret:
    pop_all_regs
    addq $16, %rsp            /* stack now: [rip][cs][rflags][rsp][ss] */
    testb $3, 8(%rsp)         /* returning to user? */
    jnz 2f
    /* CPL0 return: a same-privilege iretq pops only rip/cs/rflags, so
     * the synthesized rsp/ss would stay unpopped and the interrupted
     * context would resume with rsp 16 bytes low — its first epilogue
     * would unwind garbage. Slide the three-slot tail up over the
     * synthesized pair; once iretq pops it, rsp lands exactly on the
     * trap-time rsp (frame base + 176). Copy the highest slot first. */
    movq 16(%rsp), %rax       /* rflags -> ss slot */
    movq %rax, 32(%rsp)
    movq 8(%rsp), %rax        /* cs -> rsp slot */
    movq %rax, 24(%rsp)
    movq 0(%rsp), %rax        /* rip -> rflags slot */
    movq %rax, 16(%rsp)
    addq $16, %rsp
    iretq
2:
    swapgs
    iretq

/* --- first return to user mode ---------------------------------------------
 * Entered by `ret` from forkret with rsp pointing at the seeded trap frame
 * (frame base = context sp + 64; the two return-address slots sit below).
 */
.text
.globl _trapret_user
_trapret_user:
    jmp trapret_common
