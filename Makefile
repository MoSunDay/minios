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
GRUB_CFG := scripts/m0r-grub-early.cfg
ISO_DIR  := $(TARGET_DIR)/m0r-iso
ISO      := $(ISO_DIR)/minios-m0r.iso
RUN_DIR  := $(TARGET_DIR)/m0r-run
GRUB_MODULES := biosdisk iso9660 multiboot normal configfile terminal serial echo boot search memdisk

.PHONY: all build check-layout m0r-iso test-qemu-smoke run-smoke test test-host fmt fmt-check \
        clean legacy-user

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

# Real QEMU boot + strict verdict (exit 33, protocol line, all cases).
# Evidence (serial.log, qemu.stderr) is kept under $(RUN_DIR) for hashing.
test-qemu-smoke: m0r-iso
	@rm -rf $(RUN_DIR)
	$(RUNNER) run-smoke --iso $(ISO) --work-dir $(RUN_DIR)
	@sha256sum $(RUN_DIR)/serial.log

run-smoke: test-qemu-smoke

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
	@rm -rf $(ISO_DIR)

# Legacy i686 userspace (not part of the x86_64 Core path; see kernel/src/legacy).
legacy-user:
	$(MAKE) -C user
