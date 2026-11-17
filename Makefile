# minios top-level Makefile: x86_64 Core (M0R).

# Host triple for host-side crates (runner, host tests); the kernel target is
# pinned by .cargo/config.toml (x86_64-unknown-none).
HOST_TARGET := $(shell rustc -vV 2>/dev/null | sed -n 's/^host: //p')

# rust-lld ships inside the toolchain sysroot; keep it on PATH for cargo.
TOOLCHAIN_BIN := $(shell rustc --print sysroot 2>/dev/null)/lib/rustlib/$(HOST_TARGET)/bin
export PATH := $(TOOLCHAIN_BIN):$(PATH)

TARGET_DIR := $(shell cargo metadata --format-version 1 --no-deps 2>/dev/null \
               | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
KERNEL   := $(TARGET_DIR)/x86_64-unknown-none/release/kernel
RUNNER   := cargo run --offline -q -p minios-runner --target $(HOST_TARGET) --

# CORE-UP-GRUB image pieces (ADR-0014; frozen in plans/linux26/40).
# M1R (S2) adds two cmdline variants + the frozen raw-IDE root drive
# (CORE-UP-GRUB-R1, 40 §1.2): root.raw is a placeholder zero disk until the
# S3 Rust mkfs takes it over.
GRUB_CFG := scripts/m0r-grub-early.cfg
M1R_SUITE_CFG := scripts/m1r-suite-grub.cfg
M1R_FAULT_CFG := scripts/m1r-fault-grub.cfg
ISO_DIR  := $(TARGET_DIR)/m0r-iso
M1R_SUITE_DIR := $(TARGET_DIR)/m1r-suite-iso
M1R_FAULT_DIR := $(TARGET_DIR)/m1r-fault-iso
ISO      := $(ISO_DIR)/minios-m0r.iso
M1R_SUITE_ISO := $(M1R_SUITE_DIR)/minios-m1r-suite.iso
M1R_FAULT_ISO := $(M1R_FAULT_DIR)/minios-m1r-fault.iso
RUN_DIR  := $(TARGET_DIR)/m0r-run
M1R_SUITE_RUN := $(TARGET_DIR)/m1r-suite-run
M1R_FAULT_RUN := $(TARGET_DIR)/m1r-fault-run
ROOT_RAW := $(TARGET_DIR)/root.raw
GRUB_MODULES := biosdisk iso9660 multiboot normal configfile terminal serial echo boot search memdisk

.PHONY: all build check-layout m0r-iso m1r-iso m1r-fault-iso test-qemu-smoke run-smoke \
        test-qemu test-expected-fault test test-host fmt fmt-check clean legacy-user

all: build check-layout

# ksymtab convergence (kernel/build.rs): the generated table is size-fixed,
# so the kernel hash stabilizes after the ELF exists; loop <= 4 passes.
build:
	@prev=""; for i in 1 2 3 4; do \
	  cargo build --locked --release -p minios-kernel || exit 1; \
	  cur=$$(sha256sum $(KERNEL) | cut -d' ' -f1); \
	  if [ "$$cur" = "$$prev" ]; then exit 0; fi; \
	  prev="$$cur"; \
	done; echo "kernel build did not converge (ksymtab)"; exit 1

# Static Multiboot1/ELF64 layout gate (Rust verdict, ADR-0013).
check-layout: build
	$(RUNNER) check-elf $(KERNEL)

# Reproducible GRUB Multiboot ISO: kernel + embedded early config.
m0r-iso: build
	@rm -rf $(ISO_DIR)
	@mkdir -p $(ISO_DIR)/iso/boot/grub
	@cp $(KERNEL) $(ISO_DIR)/iso/boot/minios-x86_64.elf
	@cp $(GRUB_CFG) $(ISO_DIR)/iso/boot/grub/grub.cfg
	grub-mkimage --format=i386-pc-eltorito --output=$(ISO_DIR)/iso/boot/grub/eltorito.img \
	  --prefix=/boot/grub --config=$(GRUB_CFG) $(GRUB_MODULES)
	mkisofs -R -J -b boot/grub/eltorito.img -c boot.catalog -no-emul-boot \
	  -boot-load-size 4 -boot-info-table --output=$(ISO) $(ISO_DIR)/iso
	@sha256sum $(KERNEL) $(ISO)

# One M1R ISO variant from the kernel + its cmdline early config.
define make-m1r-iso
	@rm -rf $(1)
	@mkdir -p $(1)/iso/boot/grub
	@cp $(KERNEL) $(1)/iso/boot/minios-x86_64.elf
	@cp $(2) $(1)/iso/boot/grub/grub.cfg
	grub-mkimage --format=i386-pc-eltorito --output=$(1)/iso/boot/grub/eltorito.img \
	  --prefix=/boot/grub --config=$(2) $(GRUB_MODULES)
	mkisofs -R -J -b boot/grub/eltorito.img -c boot.catalog -no-emul-boot \
	  -boot-load-size 4 -boot-info-table --output=$(3) $(1)/iso
	@sha256sum $(KERNEL) $(3)
endef

m1r-iso: build
	$(call make-m1r-iso,$(M1R_SUITE_DIR),$(M1R_SUITE_CFG),$(M1R_SUITE_ISO))

m1r-fault-iso: build
	$(call make-m1r-iso,$(M1R_FAULT_DIR),$(M1R_FAULT_CFG),$(M1R_FAULT_ISO))

# Placeholder root disk (1 MiB zero image; S3 Rust mkfs takes it over).
$(ROOT_RAW):
	dd if=/dev/zero of=$@ bs=1M count=1 status=none

# Real QEMU boot + strict verdict (exit 33, protocol line, all cases).
# Evidence (serial.log, qemu.stderr) is kept under $(RUN_DIR) for hashing.
test-qemu-smoke: m0r-iso
	@rm -rf $(RUN_DIR)
	$(RUNNER) run-smoke --iso $(ISO) --work-dir $(RUN_DIR)
	@sha256sum $(RUN_DIR)/serial.log

run-smoke: test-qemu-smoke

# M1R kernel-mode suite (CORE-UP-GRUB-R1: + frozen raw IDE root disk).
test-qemu: m1r-iso $(ROOT_RAW)
	@rm -rf $(M1R_SUITE_RUN)
	$(RUNNER) run-m1r-suite --iso $(M1R_SUITE_ISO) --root $(ROOT_RAW) --work-dir $(M1R_SUITE_RUN)
	@sha256sum $(M1R_SUITE_RUN)/serial.log

# M1R expected-fault suite: dedicated log namespace + verdict (40 §2.1.6);
# argv identical to test-qemu (byte-pinned by the runner self-tests).
test-expected-fault: m1r-fault-iso $(ROOT_RAW)
	@rm -rf $(M1R_FAULT_RUN)
	$(RUNNER) run-m1r-fault --iso $(M1R_FAULT_ISO) --root $(ROOT_RAW) --work-dir $(M1R_FAULT_RUN)
	@sha256sum $(M1R_FAULT_RUN)/serial.log

# Host-side Rust tests: verdict fault self-tests + portable kernel modules.
test: test-host

# Host tests run with explicit packages/targets and --locked (plans/linux26/40 §4);
# -D warnings applies to host triples only and cannot affect kernel rustflags.
test-host:
	RUSTFLAGS='-D warnings' cargo test --locked --offline -p minios-host-tests --target $(HOST_TARGET)
	RUSTFLAGS='-D warnings' cargo test --locked --offline -p minios-runner --target $(HOST_TARGET)

fmt:
	cargo fmt

fmt-check:
	cargo fmt --all -- --check

clean:
	cargo clean
	@rm -rf $(ISO_DIR) $(M1R_SUITE_DIR) $(M1R_FAULT_DIR)

# Legacy i686 userspace (not part of the x86_64 Core path; see kernel/src/legacy).
legacy-user:
	$(MAKE) -C user
