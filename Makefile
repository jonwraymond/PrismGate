INSTALL_DIR := $(HOME)/.local/bin
BINARY := gatemini
CARGO_TOML := Cargo.toml

# Extract current version from Cargo.toml
VERSION := $(shell grep '^version' $(CARGO_TOML) | head -1 | sed 's/.*"\(.*\)"/\1/')
MAJOR := $(shell echo $(VERSION) | cut -d. -f1)
MINOR := $(shell echo $(VERSION) | cut -d. -f2)
PATCH := $(shell echo $(VERSION) | cut -d. -f3)

.PHONY: build install clean version bump-patch bump-minor bump-major release

build:
	cargo build --release

install: build
	@mkdir -p $(INSTALL_DIR)
	/usr/bin/install -m 755 target/release/$(BINARY) $(INSTALL_DIR)/$(BINARY)
	@echo "Installed $(INSTALL_DIR)/$(BINARY) v$(VERSION)"

clean:
	cargo clean

version:
	@echo $(VERSION)

bump-patch:
	@NEW_VERSION="$(MAJOR).$(MINOR).$(shell echo $$(($(PATCH) + 1)))"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

bump-minor:
	@NEW_VERSION="$(MAJOR).$(shell echo $$(($(MINOR) + 1))).0"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

bump-major:
	@NEW_VERSION="$(shell echo $$(($(MAJOR) + 1))).0.0"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

# Bump patch, build, and install in one step
release: bump-patch build install
	@echo "Released $(BINARY) v$$(grep '^version' $(CARGO_TOML) | head -1 | sed 's/.*"\(.*\)"/\1/')"
