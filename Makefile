INSTALL_DIR := $(HOME)/.local/bin
CONFIG_DIR := $(HOME)/.prismgate
CONFIG_FILE := $(CONFIG_DIR)/gatemini.yaml
BINARY := gatemini
CARGO_TOML := Cargo.toml

# Extract current version from Cargo.toml
VERSION := $(shell grep '^version' $(CARGO_TOML) | head -1 | sed 's/.*\"\(.*\)"/\1/')
MAJOR := $(shell echo $(VERSION) | cut -d. -f1)
MINOR := $(shell echo $(VERSION) | cut -d. -f2)
PATCH := $(shell echo $(VERSION) | cut -d. -f3)

# Profiling defaults
PROFILING_DIR := target/profiling
PERF_FREQ ?= 999
PROFILE_BENCH ?= registry_search
PROFILE_DURATION ?= 30

.PHONY: build install clean version bump-patch bump-minor bump-major release \
        profile profile-bench profile-daemon flamegraph profile-report \
        profile-clean profile-deps

build:
	cargo build --release

install: build
	@mkdir -p $(INSTALL_DIR)
	/usr/bin/install -m 755 target/release/$(BINARY) $(INSTALL_DIR)/$(BINARY)
	@if [ "$$(uname)" = "Darwin" ]; then \
		codesign -fs - $(INSTALL_DIR)/$(BINARY) 2>/dev/null || true; \
	fi
	@# Seed starter config if none exists
	@mkdir -p $(CONFIG_DIR)
	@if [ ! -f $(CONFIG_FILE) ]; then \
		cp config/starter.yaml $(CONFIG_FILE); \
		echo "Created $(CONFIG_FILE) (starter config)"; \
	fi
	@echo "Installed $(INSTALL_DIR)/$(BINARY) v$(VERSION)"
	@# Graceful hot restart: drain in-flight calls, proxies auto-reconnect
	@# with handshake replay — client sessions are preserved transparently.
	@$(INSTALL_DIR)/$(BINARY) restart 2>/dev/null && \
		echo "Daemon restarted (clients reconnecting)" || \
		echo "No daemon running (will start on next connection)"

clean:
	cargo clean

version:
	@echo $(VERSION)

bump-patch:
	@NEW_VERSION="$(MAJOR).$(MINOR).$$(($$((PATCH)) + 1)))"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

bump-minor:
	@NEW_VERSION="$(MAJOR).$$(($$((MINOR)) + 1))).0"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

bump-major:
	@NEW_VERSION="$$(($$((MAJOR)) + 1))).0.0"; \
	sed -i '' "s/^version = \"$(VERSION)\"/version = \"$$NEW_VERSION\"/" $(CARGO_TOML); \
	echo "$(VERSION) → $$NEW_VERSION"

# Bump patch, build, and install in one step
release: bump-patch build install
	@echo "Released $(BINARY) v$$(grep '^version' $(CARGO_TOML) | head -1 | sed 's/.*\"\(.*\)"/\1/')"

# ─── CPU Profiling Targets ───────────────────────────────────────────────

# Install profiling dependencies (perf + inferno)
profile-deps:
	@echo "Checking profiling tools..."
	@command -v perf >/dev/null 2>&1 || \
		(echo "Installing perf..." && apt-get install -y linux-tools-generic 2>/dev/null || \
		yum install -y perf 2>/dev/null || \
		echo "WARNING: Could not install perf automatically. Install manually.")
	@command -v inferno-flamegraph >/dev/null 2>&1 || \
		(echo "Installing inferno..." && cargo install inferno)
	@echo "Profiling dependencies ready."

# Profile a criterion benchmark and generate flamegraph
# Usage: make profile-bench PROFILE_BENCH=registry_search
profile-bench: profile-deps
	@mkdir -p $(PROFILING_DIR)
	@echo "[prof] Building $(PROFILE_BENCH) with debug symbols..."
	CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --bench $(PROFILE_BENCH)
	@BINARY=$$(find target/release/deps/ -name '$(PROFILE_BENCH)-*' -type f -executable -not -name '*.d' | head -1); \
	if [ -z "$$BINARY" ]; then echo "ERROR: benchmark binary not found"; exit 1; fi; \
	echo "[prof] Profiling $$BINARY at $(PERF_FREQ) Hz..."; \
	perf record -F $(PERF_FREQ) -g --call-graph dwarf \
		-o $(PROFILING_DIR)/$(PROFILE_BENCH)_$$(date +%Y%m%d_%H%M%S).data \
		-- "$$BINARY" --profile-time 10
	@LATEST=$$(ls -t $(PROFILING_DIR)/$(PROFILE_BENCH)_*.data 2>/dev/null | head -1); \
	if [ -n "$$LATEST" ]; then \
		echo "[prof] Generating flamegraph..."; \
		perf script -i "$$LATEST" | inferno-flamegraph \
			--width 2400 --min-width 0.01 \
			--title "PrismGate: $(PROFILE_BENCH)" \
			> "$${LATEST%.data}.svg"; \
		echo "[prof] Flamegraph: $${LATEST%.data}.svg"; \
	fi

# Profile the running daemon for N seconds
# Usage: make profile-daemon PROFILE_DURATION=60
profile-daemon: profile-deps
	@mkdir -p $(PROFILING_DIR)
	@PID=$$(pgrep -x $(BINARY) 2>/dev/null); \
	if [ -z "$$PID" ]; then echo "ERROR: No running $(BINARY) daemon. Start with: $(BINARY) serve"; exit 1; fi; \
	echo "[prof] Profiling daemon (PID $$PID) for $(PROFILE_DURATION)s at $(PERF_FREQ) Hz..."; \
	perf record -F $(PERF_FREQ) -g --call-graph dwarf \
		-p $$PID \
		-o $(PROFILING_DIR)/daemon_$$(date +%Y%m%d_%H%M%S).data \
		sleep $(PROFILE_DURATION)
	@LATEST=$$(ls -t $(PROFILING_DIR)/daemon_*.data 2>/dev/null | head -1); \
	if [ -n "$$LATEST" ]; then \
		echo "[prof] Generating flamegraph..."; \
		perf script -i "$$LATEST" | inferno-flamegraph \
			--width 2400 --min-width 0.01 \
			--title "PrismGate: daemon ($$PROFILE_DURATION)" \
			> "$${LATEST%.data}.svg"; \
		echo "[prof] Flamegraph: $${LATEST%.data}.svg"; \
	fi

# Generate flamegraph from existing perf.data
# Usage: make flamegraph PERF_DATA=target/profiling/foo.data
flamegraph:
	@test -n "$(PERF_DATA)" || (echo "Usage: make flamegraph PERF_DATA=<path>"; exit 1)
	@test -f "$(PERF_DATA)" || (echo "ERROR: $(PERF_DATA) not found"; exit 1)
	@echo "[prof] Generating flamegraph from $(PERF_DATA)..."; \
	perf script -i "$(PERF_DATA)" | inferno-flamegraph \
		--width 2400 --min-width 0.01 \
		--title "PrismGate CPU Flamegraph" \
		> "$(PERF_DATA:.data=.svg)"; \
	echo "[prof] Flamegraph: $(PERF_DATA:.data=.svg)"

# Show text profiling report
# Usage: make profile-report PERF_DATA=target/profiling/foo.data
profile-report:
	@test -n "$(PERF_DATA)" || (echo "Usage: make profile-report PERF_DATA=<path>"; exit 1)
	@test -f "$(PERF_DATA)" || (echo "ERROR: $(PERF_DATA) not found"; exit 1)
	perf report -i "$(PERF_DATA)" --stdio --percent-limit 1

# Clean profiling artifacts
profile-clean:
	rm -rf $(PROFILING_DIR)

# Convenience: profile the default benchmark end-to-end
profile: profile-bench
