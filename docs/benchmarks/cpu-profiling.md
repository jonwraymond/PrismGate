# CPU Profiling with perf/Flamegraph

This guide covers CPU profiling PrismGate using Linux `perf` and generating
flamegraphs with `inferno`.

## Prerequisites

| Tool | Install | Purpose |
|------|---------|---------|
| `perf` | `apt install linux-tools-generic` | CPU sampling |
| `inferno` | `cargo install inferno` | Flamegraph SVG generation |
| debug symbols | automatic (via `CARGO_PROFILE_RELEASE_DEBUG=true`) | Symbol resolution |

One-command setup:

```bash
make profile-deps
```

## Quick Start

### Profile a benchmark

```bash
# Default: profile the registry_search benchmark
make profile

# Specific benchmark
make profile-bench PROFILE_BENCH=registry_search
```

### Profile the running daemon

```bash
# Start daemon first
gatemini serve &

# Profile for 30 seconds (default)
make profile-daemon

# Profile for 60 seconds
make profile-daemon PROFILE_DURATION=60
```

### Generate flamegraph from existing data

```bash
make flamegraph PERF_DATA=target/profiling/registry_search_20260525_143000.data
```

### View text report

```bash
make profile-report PERF_DATA=target/profiling/registry_search_20260525_143000.data
```

## Using the Script Directly

The `.github/scripts/profile_bench.sh` script provides more control:

```bash
# Profile a benchmark
./.github/scripts/profile_bench.sh bench registry_search

# Profile the daemon for 60 seconds
./.github/scripts/profile_bench.sh daemon 60

# Generate flamegraph from perf.data
./.github/scripts/profile_bench.sh flamegraph target/profiling/foo.data

# Show text report
./.github/scripts/profile_bench.sh report target/profiling/foo.data
```

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `PERF_FREQ` | `999` | Sampling frequency in Hz |
| `PROFILING_OUTPUT` | `./target/profiling` | Output directory |

Higher frequency = more detail but larger files and more overhead.
`999` is a good default; use `4999` for hot-path analysis.

## Output Files

All profiling artifacts go to `target/profiling/`:

```
target/profiling/
├── registry_search_20260525_143000.data       # Raw perf data
├── registry_search_20260525_143000.svg        # Flamegraph SVG
├── registry_search_20260525_143000.collapsed  # Collapsed stacks
├── daemon_20260525_150000.data
├── daemon_20260525_150000.svg
└── ...
```

Clean with: `make profile-clean`

## Interpreting Flamegraphs

1. Open the `.svg` file in a browser.
2. **Width** = proportion of CPU time in that stack frame.
3. **Stack depth** (y-axis) = call chain. Top = leaf function.
4. **Click** any frame to zoom in. Search with the search box.
5. Look for wide blocks at the top — those are your hot functions.

Key PrismGate hot spots to watch:

| Area | Expected behavior |
|------|-------------------|
| `registry::search` | BM25 scoring should dominate; fuzzy fallback only on miss |
| `server::handle_tool_call` | Dispatch overhead; watch for serialization hotspots |
| `ipc::proxy` | Socket I/O; should be thin if proxy is just forwarding |
| `backend::health_check` | Periodic; shouldn't dominate unless backends are unhealthy |

## perf_event_paranoid

If you get permission errors:

```bash
# Check current value (4 = most restricted)
cat /proc/sys/kernel/perf_event_paranoid

# Allow non-root profiling (requires root to set)
sudo sysctl -w kernel.perf_event_paranoid=1

# Persistent:
echo 'kernel.perf_event_paranoid=1' | sudo tee /etc/sysctl.d/99-perf.conf
```

Values:
- `-1`: No restriction
- `0`: Allow per-thread profiling
- `1`: Allow per-thread + per-cpu (recommended for dev)
- `2+:` Restricted (default on many distros)

## Integration with CI

The profiling script is CI-friendly. Example GitHub Actions step:

```yaml
- name: Profile registry_search
  run: |
    make profile-bench PROFILE_BENCH=registry_search
  env:
    PERF_FREQ: 999

- name: Upload flamegraph
  uses: actions/upload-artifact@v4
  with:
    name: flamegraphs
    path: target/profiling/*.svg
```

## Criterion Profiling Output

The existing criterion benchmarks also support profiling output:

```bash
# Generate criterion HTML reports
cargo bench --bench registry_search

# Profile-specific iteration (criterion's built-in profiling)
cargo bench --bench registry_search -- --profile-time 10
```

This generates criterion's own profiling data in `target/criterion/`.
The `make profile-bench` target complements this with full perf/flamegraph output.

## Advanced: Call Graph Options

The default uses DWARF call graphs (`--call-graph dwarf`), which provide
accurate stacks but can be slow. Alternatives:

```bash
# Frame pointers (faster but requires frame-pointer compilation)
# Add to Cargo.toml: [profile.release] opt-level = 3, debug = 1
PERF_FREQ=999 .github/scripts/profile_bench.sh bench

# LBR (Intel only, lowest overhead)
perf record -F 999 -g --call-graph lbr -o perf.data -- <binary>
```
