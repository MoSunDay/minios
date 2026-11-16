/* x86_64 minios kernel: low Multiboot boot block + high-half kernel. */
OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_low_start)

LOW_VMA = 0x00100000;
BOOT_BSS_VMA = 0x00200000;
BOOT_BSS_SIZE = 0x00290000;
KERNEL_LMA = 0x00500000;
KERNEL_VMA = 0xffffffff80000000;

SECTIONS {
. = LOW_VMA;
.boot ALIGN(4096) : {
    _boot_start = .;
    KEEP(*(.multiboot_header))
    *(.boot.text)
    *(.boot.rodata)
    . = ALIGN(4096);
    _boot_code_end = .;
    *(.boot.data)
    . = ALIGN(4096);
    _boot_end = .;
}

.boot_bss BOOT_BSS_VMA (NOLOAD) : {
    _boot_bss_start = .;
    *(.boot.bss)
    . = ALIGN(4096);
    _boot_bss_end = .;
}
ASSERT(_boot_bss_end <= BOOT_BSS_VMA + BOOT_BSS_SIZE, "boot bss overflow")


. = KERNEL_VMA;
.text : AT(KERNEL_LMA) {
    _kernel_text_start = .;
    *(.text .text.*)
    . = ALIGN(4096);
    _kernel_text_end = .;
}
__kernel_phys_text_start = LOADADDR(.text);
__kernel_phys_rodata_start = __kernel_phys_text_start + SIZEOF(.text);

.rodata : AT(__kernel_phys_rodata_start) {
    _kernel_rodata_start = .;
    *(.rodata .rodata.*)
    . = ALIGN(4096);
    _kernel_rodata_end = .;
}
__kernel_phys_data_start = __kernel_phys_rodata_start + SIZEOF(.rodata);

.data : AT(__kernel_phys_data_start) {
    _kernel_data_start = .;
    *(.data .data.*)
    . = ALIGN(4096);
    _kernel_data_end = .;
}
__kernel_phys_bss_start = __kernel_phys_data_start + SIZEOF(.data);

.bss : AT(__kernel_phys_bss_start) {
    _kernel_bss_start = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(4096);
    _kernel_bss_end = .;
}

/DISCARD/ : {
    *(.eh_frame .eh_frame_hdr)
    *(.comment)
    *(.note*)
}
}
