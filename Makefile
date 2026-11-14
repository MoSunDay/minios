# minios top-level Makefile.

TARGET   := i686-unknown-linux-gnu
# Honor CARGO_TARGET_DIR / global cargo config instead of assuming ./target.
TARGET_DIR := $(shell cargo metadata --format-version 1 --no-deps 2>/dev/null \
               | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
KERNEL   := $(TARGET_DIR)/$(TARGET)/release/kernel
QEMU     := qemu-system-i386
RUN_SECS := 10
RUN_LOG  := /tmp/minios-run.log

.PHONY: all build run check fmt clean

all: build

build:
	cargo build --target $(TARGET) --release

# Boot the kernel under QEMU for a few seconds and verify the serial log.
run: build
	@rm -f $(RUN_LOG)
	@timeout $(RUN_SECS) $(QEMU) -kernel $(KERNEL) -serial stdio -display none \
	  -no-reboot > $(RUN_LOG) 2>&1 || [ $$? -eq 124 ]
	@cat $(RUN_LOG)
	@grep -q 'minios v0.1.0 (x86, rust)' $(RUN_LOG) \
	  || { echo 'FAIL: banner line missing'; exit 1; }
	@grep -q 'boot ok' $(RUN_LOG) \
	  || { echo 'FAIL: boot ok line missing'; exit 1; }
	@echo 'run: serial output verified'

check:
	cargo build --target $(TARGET)

fmt:
	cargo fmt

clean:
	cargo clean
	rm -f $(RUN_LOG)
