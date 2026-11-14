# minios user/ Makefile — xv6 C user programs on the minios syscall ABI.
#
# Original xv6 rules kept: user-program linking (%_), ULIB, _forktest
# special case, UPROGS, mkfs invocation, fs.img target. Adapted: paths
# (mkfs lives in ../tools), the mkfs build line, and the qemu run line
# (minios boots the multiboot Rust kernel directly and attaches fs.img as
# the raw IDE disk instead of xv6's -hdb boot-disk combo).

CC = gcc
# -fno-stack-protector: modern distro gccs default to -fstack-protector-strong,
# which drags in __stack_chk_fail from libc that a -nostdlib static user
# program cannot resolve (classic xv6 assumed older gcc defaults).
CFLAGS = -fno-pic -O -nostdinc -I. -m32 -fno-builtin -static -fno-stack-protector
LD = ld
LDFLAGS = -m elf_i386

# minios kernel image (Rust side), for the qemu run line. Honor the
# workspace's cargo target directory (it may live outside ./target).
TARGET_DIR ?= $(shell cargo metadata --format-version 1 --no-deps 2>/dev/null \
               | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
KERNEL ?= $(TARGET_DIR)/i686-unknown-linux-gnu/release/kernel

ULIB = ulib.o usys.o printf.o umalloc.o

all: $(UPROGS) fs.img
.PHONY: all

# xv6: every user program is statically linked at 0 with ulib+usys+printf+umalloc.
_%: %.o $(ULIB)
	$(LD) $(LDFLAGS) -N -e main -Ttext 0 -o $@ $^

_forktest: forktest.o ulib.o usys.o
	# forktest has less library code linked in - needs to be small
	# in order to be able to max out the proc table.
	$(LD) $(LDFLAGS) -N -e main -Ttext 0 -o _forktest forktest.o ulib.o usys.o

UPROGS=\
	_cat\
	_echo\
	_forktest\
	_grep\
	_init\
	_kill\
	_ln\
	_ls\
	_mkdir\
	_rm\
	_sh\
	_stressfs\
	_usertests\
	_wc\
	_zombie\

# xv6 ships README on the image; drop it from prerequisites here if minios
# does not keep a README file in this directory.
../tools/mkfs: ../tools/mkfs.c ../tools/fs.h
	gcc -m32 -Werror -Wall -o ../tools/mkfs ../tools/mkfs.c

# xv6: `fs.img: mkfs README $(UPROGS); ./mkfs fs.img README $(UPROGS)`
fs.img: ../tools/mkfs $(UPROGS)
	../tools/mkfs fs.img $(UPROGS)

# Prevent deletion of intermediate files, e.g. cat.o, after first build, so
# that disk image changes after first build are persistent until clean.
# http://www.gnu.org/software/make/manual/html_node/Chained-Rules.html
.PRECIOUS: %.o

# xv6 compiles user .c/.S with $(CC) $(CFLAGS) -c; kept for completeness.
%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

%.o: %.S
	$(CC) $(CFLAGS) -c -o $@ $<

# minios run line: multiboot Rust kernel + raw IDE fs.img (xv6 used
# `$(QEMU) $(QEMUOPTS)` with `-hdb fs.img xv6.img -m 512`).
qemu: fs.img
	qemu-system-i386 -kernel $(KERNEL) \
	  -drive file=fs.img,index=1,if=ide,format=raw \
	  -m 256M -serial stdio -display none -no-reboot
