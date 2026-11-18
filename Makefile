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
# (CORE-UP-GRUB-R1, 40 §1.2): a pristine 1 MiB zero template is copied into
# each run dir per round (40 §2.2 per-round images); the S3 Rust mkfs takes
# the disk over later.
GRUB_CFG := scripts/m0r-grub-early.cfg
M1R_SUITE_CFG := scripts/m1r-suite-grub.cfg
M1R_USER_CFG := scripts/m1r-user-grub.cfg
M1R_FAULT_CFG := scripts/m1r-fault-grub.cfg
ISO_DIR  := $(TARGET_DIR)/m0r-iso
M1R_SUITE_DIR := $(TARGET_DIR)/m1r-suite-iso
M1R_USER_DIR := $(TARGET_DIR)/m1r-user-iso
M1R_FAULT_DIR := $(TARGET_DIR)/m1r-fault-iso
ISO      := $(ISO_DIR)/minios-m0r.iso
M1R_SUITE_ISO := $(M1R_SUITE_DIR)/minios-m1r-suite.iso
M1R_USER_ISO := $(M1R_USER_DIR)/minios-m1r-user.iso
M1R_FAULT_ISO := $(M1R_FAULT_DIR)/minios-m1r-fault.iso
# M1R S3 user chain: the user crate builds through its own target dir and
# RUSTFLAGS (rust-lld + own link script + static non-PIE ET_EXEC); these
# are Makefile-scoped env, deliberately never .cargo/config.toml, which is
# kernel-scoped (04 §2). MINIOS_USER_IMG_DIR feeds kernel/build.rs.
USER_TARGET := $(CURDIR)/target-user
USER_RELEASE := $(USER_TARGET)/x86_64-unknown-linux-gnu/release
# S4e: the xv6 program set rides the root image (dirent order = this
# list; +usertests since S4e-2 — 19 dirents = 304 bytes, one block).
USER_ELFS := $(USER_RELEASE)/m1r-suite $(USER_RELEASE)/m1r-exec-child \
  $(USER_RELEASE)/cat $(USER_RELEASE)/echo $(USER_RELEASE)/forktest \
  $(USER_RELEASE)/grep $(USER_RELEASE)/init $(USER_RELEASE)/kill \
  $(USER_RELEASE)/ln $(USER_RELEASE)/ls $(USER_RELEASE)/mkdir \
  $(USER_RELEASE)/rm $(USER_RELEASE)/sh $(USER_RELEASE)/stressfs \
  $(USER_RELEASE)/usertests $(USER_RELEASE)/wc $(USER_RELEASE)/zombie
export MINIOS_USER_IMG_DIR := $(USER_RELEASE)
# rust-lld speaks GNU-ld when invoked as ld.lld (argv[0] flavor); the
# symlink below is pure invocation plumbing, still the toolchain lld.
USER_LLD_DIR := $(USER_TARGET)/bin
USER_RUSTFLAGS := -C linker=ld.lld -C linker-flavor=ld \
                  -C relocation-model=static \
                  -C link-arg=-T$(CURDIR)/user/x86_64-minios-user.ld \
                  -C link-arg=--build-id=none

RUN_DIR  := $(TARGET_DIR)/m0r-run
M1R_SUITE_RUN := $(TARGET_DIR)/m1r-suite-run
M1R_USER_RUN := $(TARGET_DIR)/m1r-user-run
M1R_FAULT_RUN := $(TARGET_DIR)/m1r-fault-run
ROOT_TEMPLATE := $(TARGET_DIR)/root-template.raw
GRUB_MODULES := biosdisk iso9660 multiboot normal configfile terminal serial echo boot search memdisk

.PHONY: all build user-build check-user-elf check-layout m0r-iso m1r-iso m1r-user-iso m1r-fault-iso \
        test-qemu-smoke run-smoke test-qemu test-qemu-user test-expected-fault test test-host fmt fmt-check \
        clean legacy-user

all: build check-user-elf check-layout

# M1R S3 user binaries: static non-PIE ET_EXEC, rust-lld, own script.
user-build:
	@mkdir -p $(USER_LLD_DIR)
	@ln -sf $(TOOLCHAIN_BIN)/rust-lld $(USER_LLD_DIR)/ld.lld
	PATH=$(USER_LLD_DIR):$(TOOLCHAIN_BIN):$$PATH RUSTFLAGS='$(USER_RUSTFLAGS)' \
	  CARGO_TARGET_DIR=$(USER_TARGET) \
	  cargo build --locked --release -p minios-user --target x86_64-unknown-linux-gnu

# Internal ELF layout gate for the user binaries (04 §2: ET_EXEC, static,
# no PT_INTERP). Host-side Rust verdict, like check-layout.
check-user-elf: user-build
	$(RUNNER) check-user-elf $(USER_ELFS)

# ksymtab convergence (kernel/build.rs): the generated table is size-fixed,
# so the kernel hash stabilizes after the ELF exists; loop <= 4 passes.
# S3: the kernel embeds the user ELFs (MINIOS_USER_IMG_DIR), so the user
# build runs first and build.rs rerun-if-changed covers image updates.
build: user-build
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

m1r-user-iso: build
	$(call make-m1r-iso,$(M1R_USER_DIR),$(M1R_USER_CFG),$(M1R_USER_ISO))

# Root-disk template: the xv6 root image built by the S4 Rust mkfs
# (minios-tools, plans/linux26/04 §5; byte-parity with tools/mkfs.c
# proven at port time). Never attached to QEMU directly: every round
# copies it fresh into its own run dir (40 §2.2).
MKFS := cargo run --offline -q -p minios-tools --target $(HOST_TARGET) --bin minios-mkfs --
$(ROOT_TEMPLATE): $(USER_ELFS) tools/src/lib.rs tools/src/bin/mkfs.rs tools/Cargo.toml
	@rm -f $@
	$(MKFS) $@ $(USER_ELFS)
	@sha256sum $@

# Real QEMU boot + strict verdict (exit 33, protocol line, all cases).
# Evidence (serial.log, qemu.stderr) is kept under $(RUN_DIR) for hashing.
test-qemu-smoke: m0r-iso
	@rm -rf $(RUN_DIR)
	$(RUNNER) run-smoke --iso $(ISO) --work-dir $(RUN_DIR)
	@sha256sum $(RUN_DIR)/serial.log

run-smoke: test-qemu-smoke

# M1R kernel-mode suite (CORE-UP-GRUB-R1: + frozen raw IDE root disk).
test-qemu: m1r-iso $(ROOT_TEMPLATE)
	@rm -rf $(M1R_SUITE_RUN)
	@mkdir -p $(M1R_SUITE_RUN)
	@cp $(ROOT_TEMPLATE) $(M1R_SUITE_RUN)/root.raw
	$(RUNNER) run-m1r-suite --iso $(M1R_SUITE_ISO) --root $(M1R_SUITE_RUN)/root.raw --work-dir $(M1R_SUITE_RUN)
	@sha256sum $(M1R_SUITE_RUN)/serial.log $(M1R_SUITE_RUN)/root.raw

# M1R user-mode suite (S4e-2): the xv6 usertests port as pid 1.
test-qemu-user: m1r-user-iso $(ROOT_TEMPLATE)
	@rm -rf $(M1R_USER_RUN) results/m1r-user
	@mkdir -p $(M1R_USER_RUN)
	@cp $(ROOT_TEMPLATE) $(M1R_USER_RUN)/root.raw
	$(RUNNER) run-m1r-user --iso $(M1R_USER_ISO) --root $(M1R_USER_RUN)/root.raw --work-dir $(M1R_USER_RUN)
	@sha256sum $(M1R_USER_RUN)/serial.log $(M1R_USER_RUN)/root.raw
	$(RUNNER) coverage --manifest tests/user/coverage.toml --serial $(M1R_USER_RUN)/serial.log \
	  --suite m1r-user --run m1r-user --config CORE-UP-GRUB-R1 --milestone M1R

# M1R expected-fault suite: dedicated log namespace + verdict (40 §2.1.6);
# argv identical to test-qemu except the -cdrom image (construction
# byte-pinned by the runner self-tests).
test-expected-fault: m1r-fault-iso $(ROOT_TEMPLATE)
	@rm -rf $(M1R_FAULT_RUN)
	@mkdir -p $(M1R_FAULT_RUN)
	@cp $(ROOT_TEMPLATE) $(M1R_FAULT_RUN)/root.raw
	$(RUNNER) run-m1r-fault --iso $(M1R_FAULT_ISO) --root $(M1R_FAULT_RUN)/root.raw --work-dir $(M1R_FAULT_RUN)
	@sha256sum $(M1R_FAULT_RUN)/serial.log $(M1R_FAULT_RUN)/root.raw

# Host-side Rust tests: verdict fault self-tests + portable kernel modules.
test: test-host

# Host tests run with explicit packages/targets and --locked (plans/linux26/40 §4);
# -D warnings applies to host triples only and cannot affect kernel rustflags.
test-host:
	RUSTFLAGS='-D warnings' cargo test --locked --offline -p minios-host-tests --target $(HOST_TARGET)
	RUSTFLAGS='-D warnings' cargo test --locked --offline -p minios-runner --target $(HOST_TARGET)
	RUSTFLAGS='-D warnings' cargo test --locked --offline -p minios-tools --target $(HOST_TARGET)

fmt:
	cargo fmt

fmt-check:
	cargo fmt --all -- --check

clean:
	cargo clean
	@rm -rf $(ISO_DIR) $(M1R_SUITE_DIR) $(M1R_FAULT_DIR) $(USER_TARGET)

# Legacy i686 userspace (not part of the x86_64 Core path; see kernel/src/legacy).
legacy-user:
	$(MAKE) -C user
