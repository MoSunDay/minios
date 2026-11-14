/* minios kernel linker script (xv6-style layout, multiboot-loaded).
 * Low trampoline at VMA=LMA=0x100000; kernel proper at VMA 0x80100000,
 * loaded at physical 0x110000. */
OUTPUT_FORMAT(elf32-i386)
ENTRY(_low_start)

SECTIONS {
  . = 0x100000;
  .multiboot : { KEEP(*(.multiboot)) }   /* stays within first 8K of file */
  .lowentry  : {
    KEEP(*(.lowentry))
    . = ALIGN(4096);
    LOW_PDE = .;      . += 4096;          /* runtime-built boot page dir */
    LOW_STACK = .;    . += 4096;          /* boot stack */
    LOW_STACK_TOP = .;
    LOW_MB_MAGIC = .; . += 4;
    LOW_MB_INFO  = .; . += 4;
    LOW_END = .;
  }

  /* Kernel proper: VMA = LMA + KERNBASE (constant offset, so the boot
   * trampoline's single 4MB page at VMA 0x80000000 -> phys 0 covers it). */
  . = 0x80110000;
  KOFF = 0x80000000; /* VMA - LMA constant */
  _kernel_start = .;
  .text   : AT(ADDR(.text) - KOFF) { *(.text .text.*) }
  .rodata : AT(ADDR(.rodata) - KOFF) { *(.rodata .rodata.* .srodata*) }
  .stabs  : AT(ADDR(.stabs) - KOFF) { *(.stab .stabstr) }
  .data   : AT(ADDR(.data) - KOFF) {
              *(.data .data.*)
              *(.got .got.plt)
              . = ALIGN(4096);
            }
  .bss : {
    _bss_start = .;
    *(.bss .bss.* COMMON)
    . = ALIGN(4096);
    BOOT_STACK = .; . += 16384; BOOT_STACK_TOP = .;
    _bss_end = .;
  }
  . = ALIGN(4096);
  _kernel_end = .;

  /DISCARD/ : { *(.comment) *(.eh_frame) *(.eh_frame_hdr) *(.gcc_except_table .gcc_except_table.*) *(.note*) *(.interp) *(.gnu*) }

  ASSERT(LOW_END <= 0x110000, "low region overflows into kernel LMA")
  ASSERT(_kernel_end - _kernel_start < 0x10000000, "kernel image too large")
}
