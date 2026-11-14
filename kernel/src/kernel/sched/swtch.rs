//! swtch.S port: kernel context switch.
//!
//!   void swtch(struct context **old, struct context *new);
//!
//! Save the current callee-saved registers into *old (the caller's context
//! slot, e.g. `cpu->scheduler` or `proc->context`) and load the new context.
//! `ret` pops the new eip from the top of the new context — that is how
//! allocproc() arranges for a fresh process to start at `forkret`, which
//! then returns into `trapret`.

use core::arch::global_asm;

use crate::arch::x86::kernel::Context;

global_asm!(
    ".globl __xv6_swtch",
    "__xv6_swtch:",
    "  movl 4(%esp), %eax", // eax = &old-slot
    "  movl 8(%esp), %edx", // edx = new context
    // Save old callee-save registers.
    "  pushl %ebp",
    "  pushl %ebx",
    "  pushl %esi",
    "  pushl %edi",
    // Switch stacks.
    "  movl %esp, (%eax)",
    "  movl %edx, %esp",
    // Load new callee-save registers.
    "  popl %edi",
    "  popl %esi",
    "  popl %ebx",
    "  popl %ebp",
    "  ret",
    options(att_syntax)
);

extern "C" {
    fn __xv6_swtch(old: *mut *mut Context, new: *const Context);
}

/// Same switch with the natural `**context` typing (xv6 call sites).
///
/// SAFETY: see swtch().
pub unsafe fn swtch_slot(old: *mut *mut Context, new: *const Context) {
    // SAFETY: see swtch().
    unsafe { __xv6_swtch(old, new) }
}
