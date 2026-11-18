/*
 * minios user link script (M1R): static non-PIE ET_EXEC at the classic
 * Linux non-PIE base. Segments stay page-separated so the kernel exec
 * loader can apply per-segment NX without page conflicts.
 */
OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
  . = 0x400000;

  .text : ALIGN(0x1000)
  {
    KEEP(*(.text.start))
    *(.text .text.*)
  }

  .rodata : ALIGN(0x1000)
  {
    *(.rodata .rodata.*)
    . = ALIGN(0x8);
  }

  .data : ALIGN(0x1000)
  {
    /* The static image still carries a tiny .got; merging it here keeps
     * every PT_LOAD page-separated (the kernel loader maps per page). */
    *(.data .data.*)
    *(.got .got.*)
    . = ALIGN(0x8);
  }

  .bss : ALIGN(0x1000)
  {
    __bss_start = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(0x8);
    __bss_end = .;
  }

  /DISCARD/ :
  {
    *(.eh_frame .eh_frame_hdr .eh_frame.*)
    *(.note .note.*)
    *(.comment)
  }
}
