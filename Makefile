INSTALL_DIR := $(HOME)/.local/bin
BINARY := gatemini

.PHONY: build install clean

build:
	cargo build --release

install: build
	@mkdir -p $(INSTALL_DIR)
	cp target/release/$(BINARY) $(INSTALL_DIR)/$(BINARY)
	@echo "Installed $(INSTALL_DIR)/$(BINARY)"

clean:
	cargo clean
