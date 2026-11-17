/* swtch: xv6-style context switch of callee-saved state.
 *
 * rdi = &old.sp slot, rsi = new sp value.
 *
 * The callee-saved registers are pushed on the OLD thread's own stack; the
 * return address pushed by the caller's `call` instruction becomes the
 * resume rip, so a switched-out thread resumes right after its _swtch call.
 * A fresh thread seeds the identical 7-slot image by hand (see
 * proc::allocproc): [rbp][rbx][r12][r13][r14][r15][resume-rip], with the
 * slot above the resume rip holding the next return address.
 */
.text
.globl _swtch
_swtch:
    pushq %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, (%rdi)       /* old->sp = saved-register frame base */
    movq %rsi, %rsp         /* switch stacks */
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    ret                     /* resume rip from the image's 7th slot */
