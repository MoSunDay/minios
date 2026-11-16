.section .multiboot_header, "a"
.align 4
.long 0x1BADB002
.long 0x00000003
.long -(0x1BADB002 + 0x00000003)

.section .boot.text, "ax"
.code32
.globl _low_start
_low_start:
    movl $_boot_stack_top, %esp
    movl %eax, _saved_magic
    movl %ebx, _saved_mbi

    /* Clear the low-memory boot page structures. */
    leal _boot_bss_start, %edi
    xorl %eax, %eax
    movl $_boot_bss_end, %ecx
    subl %edi, %ecx
    shrl $2, %ecx
    rep stosl

    leal _pml4, %edi
    leal _low_pdpt, %eax
    orl $3, %eax
    movl %eax, 0(%edi)
    leal _high_pdpt, %eax
    orl $3, %eax
    movl %eax, 511*8(%edi)

    leal _low_pdpt, %edi
    leal _page_dir, %eax
    orl $3, %eax
    movl %eax, 0(%edi)

    leal _high_pdpt, %edi
    leal _high_page_dir, %eax
    orl $3, %eax
    movl %eax, 510*8(%edi)

    /* High half: VMA 0xffffffff80000000 + X maps phys __kernel_phys_text_start + X. */
    leal _high_page_dir, %edi
    leal _high_page_table, %eax
    orl $3, %eax
    movl %eax, 0(%edi)
    movl $0, 4(%edi)

    leal _high_page_table, %esi
    xorl %ecx, %ecx
8:
    movl $__kernel_phys_text_start, %eax
    movl %ecx, %edx
    shll $12, %edx
    addl %edx, %eax
    movl %eax, %ebp
    call _pte_flags
    orl %ebp, %eax
    movl %eax, (%esi,%ecx,8)
    movl %edx, 4(%esi,%ecx,8)
    incl %ecx
    cmpl $512, %ecx
    jl 8b

    /* Point 128 page directories at 128 page tables (256 MiB, 4 KiB pages). */
    leal _page_dir, %edi
    leal _page_tables, %esi
    xorl %ebx, %ebx
1:
    movl %esi, %eax
    orl $3, %eax
    movl %eax, (%edi,%ebx,8)
    movl $0, 4(%edi,%ebx,8)

    xorl %ecx, %ecx
2:
    movl %ebx, %eax
    shll $21, %eax
    movl %ecx, %edx
    shll $12, %edx
    addl %edx, %eax
    movl %eax, %ebp
    call _pte_flags
    orl %ebp, %eax
    movl %eax, (%esi,%ecx,8)
    movl %edx, 4(%esi,%ecx,8)
    incl %ecx
    cmpl $512, %ecx
    jl 2b

    addl $4096, %esi
    incl %ebx
    cmpl $128, %ebx
    jl 1b

    movl %cr4, %eax
    orl $0x00000020, %eax
    movl %eax, %cr4
    movl $_pml4, %eax
    movl %eax, %cr3
    movl $0xc0000080, %ecx
    rdmsr
    orl $0x00000900, %eax
    wrmsr
    lgdt _boot_gdt_descriptor
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0
    ljmp $0x08, $_long64_start

.code64
_long64_start:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    movl _saved_magic, %edi
    movl _saved_mbi, %esi
    movabsq $rust_main, %rax
    callq *%rax
    cli
1:  hlt
    jmp 1b

/* Input: EAX = physical address. Output: EAX = low PTE flags, EDX = high flags.
 * Must touch only EAX/EDX: callers keep loop indices in ECX/EBX/ESI/EDI. */
_pte_flags:
    xorl %edx, %edx
    cmpl $_boot_code_end, %eax
    jl 3f
    cmpl $__kernel_phys_text_start, %eax
    jl 4f
    cmpl $__kernel_phys_rodata_start, %eax
    jge 5f
    jmp 3f
3:  /* Executable, read-only. */
    movl $1, %eax
    ret
4:  /* Boot data/page tables/stack: writable, non-executable. */
    movl $3, %eax
    movl $0x80000000, %edx
    ret
5:  cmpl $__kernel_phys_data_start, %eax
    jl 6f
    movl $3, %eax
    movl $0x80000000, %edx
    ret
6:  /* Kernel rodata: read-only, non-executable. */
    movl $1, %eax
    movl $0x80000000, %edx
    ret

.section .boot.data, "aw"
.align 4
_saved_magic:
    .long 0
_saved_mbi:
    .long 0

.section .boot.rodata, "a"
.align 8
_boot_gdt:
    .quad 0
    .quad 0x00209A0000000000
    .quad 0x0000920000000000
_boot_gdt_end:
_boot_gdt_descriptor:
    .word _boot_gdt_end - _boot_gdt - 1
    .long _boot_gdt

.section .boot.bss, "aw", @nobits
.align 4096
_pml4:
    .space 4096
_low_pdpt:
    .space 4096
_high_pdpt:
    .space 4096
_high_page_dir:
    .space 4096
_high_page_table:
    .space 4096
_page_dir:
    .space 4096
_page_tables:
    .space 4096 * 128
.align 16
_boot_stack:
    .space 0x10000
_boot_stack_top:
