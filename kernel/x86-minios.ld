/* minios kernel linker script: i386 ELF loaded at 1 MiB by multiboot. */

OUTPUT_FORMAT(elf32-i386)
OUTPUT_ARCH(i386)
ENTRY(_start)

KERNEL_LOAD_ADDR = 0x100000;
BOOT_STACK_SIZE  = 16K;

SECTIONS
{
    . = KERNEL_LOAD_ADDR;

    /* Multiboot header must appear within the first 8 KiB of the image. */
    .multiboot : ALIGN(4096)
    {
        KEEP(*(.multiboot))
    }

    .text : ALIGN(4096)
    {
        *(.text .text.*)
    }

    .rodata : ALIGN(4096)
    {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096)
    {
        *(.data .data.*)
        *(.got .got.plt)
    }

    .bss : ALIGN(4096)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        boot_stack_bottom = .;
        . += BOOT_STACK_SIZE;
        boot_stack_top = .;
        __bss_end = .;
    }

    _kernel_end = ALIGN(., 4096);

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
    }

    ASSERT(ADDR(.text)   >= ADDR(.multiboot) + SIZEOF(.multiboot), "section overlap: .multiboot vs .text")
    ASSERT(ADDR(.rodata) >= ADDR(.text) + SIZEOF(.text), "section overlap: .text vs .rodata")
    ASSERT(ADDR(.data)   >= ADDR(.rodata) + SIZEOF(.rodata), "section overlap: .rodata vs .data")
    ASSERT(ADDR(.bss)    >= ADDR(.data) + SIZEOF(.data), "section overlap: .data vs .bss")
    ASSERT(boot_stack_top - boot_stack_bottom == BOOT_STACK_SIZE, "boot stack size mismatch")
    ASSERT(_kernel_end <= 0x10000000, "kernel image exceeds 256 MiB")
}
