use core::arch::asm;

#[repr(C, align(16))]
struct Tss {
    reserved0: u32,
    privilege_stacks: [u64; 3],
    interrupt_stacks: [u64; 7],
    reserved1: u64,
    iopb_offset: u16,
}

#[repr(C, align(8))]
struct Gdt {
    null: u64,
    kernel_code: u64,
    kernel_data: u64,
    user_code: u64,
    user_data: u64,
    tss_low: u64,
    tss_high: u64,
}

#[repr(C, packed)]
struct PseudoDescriptor {
    limit: u16,
    base: u64,
}

#[derive(Clone, Copy)]
#[repr(C, packed(8))]
struct IdtGate {
    low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    middle: u16,
    high: u32,
    reserved: u32,
}

#[repr(C)]
struct Idt {
    gates: [IdtGate; 256],
}

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base: u64,
}

static mut TSS: Tss = Tss {
    reserved0: 0,
    privilege_stacks: [0; 3],
    interrupt_stacks: [0; 7],
    reserved1: 0,
    iopb_offset: core::mem::size_of::<Tss>() as u16,
};

static mut GDT: Gdt = Gdt {
    null: 0,
    kernel_code: 0x0020_9a00_0000_0000,
    kernel_data: 0x0000_9200_0000_0000,
    user_code: 0x0020_fa00_0000_0000,
    user_data: 0x0000_f200_0000_0000,
    tss_low: 0,
    tss_high: 0,
};

static mut GDTR: PseudoDescriptor = PseudoDescriptor { limit: 0, base: 0 };
static mut IDT: Idt = Idt {
    gates: [IdtGate {
        low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        middle: 0,
        high: 0,
        reserved: 0,
    }; 256],
};
static mut IDTR: IdtDescriptor = IdtDescriptor { limit: 0, base: 0 };

pub fn init() {
    // SAFETY: these tables are initialized exactly once during uniprocessor boot.
    unsafe {
        let tss_base = core::ptr::addr_of_mut!(TSS) as u64;
        GDT.tss_low = 0x0000_8900_0000_0067
            | ((tss_base & 0x00ff_ffff) << 16)
            | ((tss_base & 0xff00_0000) << 32);
        GDT.tss_high = tss_base >> 32;
        GDTR.limit = (core::mem::size_of::<Gdt>() - 1) as u16;
        GDTR.base = core::ptr::addr_of_mut!(GDT) as u64;
        IDTR.limit = (core::mem::size_of::<Idt>() - 1) as u16;
        IDTR.base = core::ptr::addr_of_mut!(IDT) as u64;
        let gdtr = core::ptr::addr_of_mut!(GDTR);
        let idtr = core::ptr::addr_of_mut!(IDTR);
        asm!("lgdt ({0})", in(reg) gdtr as u64, options(att_syntax, nostack));
        asm!("lidt ({0})", in(reg) idtr as u64, options(att_syntax, nostack));
        asm!(
            "mov $0x10, %ax\n\tmov %ax, %ds\n\tmov %ax, %es\n\tmov %ax, %ss",
            out("ax") _,
            options(att_syntax, nostack)
        );
        asm!("mov $0x28, %ax\n\tltr %ax", out("ax") _, options(att_syntax, nostack));
    }
}
