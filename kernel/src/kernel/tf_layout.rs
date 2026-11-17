//! Trapframe layout shared by the asm entry paths and the Rust dispatchers
//! (frozen offsets, compile-time asserted here and re-asserted by host
//! tests; the asm twins live in trapasm.S).

/// The trap frame built by trapasm.S: 15 GPRs + [vector][err][rip][cs]
/// [rflags][rsp][ss], size 176.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Trapframe {
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub vector: u64,
    pub err: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl Trapframe {
    /// True when the frame was taken from ring 3.
    pub fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }
}

const _: () = {
    use core::mem::offset_of;
    assert!(core::mem::size_of::<Trapframe>() == 176);
    assert!(offset_of!(Trapframe, rdi) == 0);
    assert!(offset_of!(Trapframe, r15) == 112);
    assert!(offset_of!(Trapframe, vector) == 120);
    assert!(offset_of!(Trapframe, err) == 128);
    assert!(offset_of!(Trapframe, rip) == 136);
    assert!(offset_of!(Trapframe, cs) == 144);
    assert!(offset_of!(Trapframe, rflags) == 152);
    assert!(offset_of!(Trapframe, rsp) == 160);
    assert!(offset_of!(Trapframe, ss) == 168);
};
