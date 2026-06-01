#!/usr/bin/env bash
# profile_bench.sh — CPU profiling for PrismGate benchmarks and daemon
#
# Usage:
#   ./profile_bench.sh bench [BENCH_NAME]     Profile a criterion benchmark
#   ./profile_bench.sh daemon [DURATION_SECS]  Profile the running daemon
#   ./profile_bench.sh flamegraph [PERF_DATA]  Generate flamegraph from perf.data
#
# Requirements:
#   Linux perf (linux-tools-generic or kernel-tools)
#   inferno-flamegraph (cargo install inferno)
#   Optional: cargo-flamegraph (cargo install flamegraph)
#
# Environment:
#   PERF_FREQ         Sampling frequency (default: 999 Hz)
#   PROFILING_OUTPUT  Output directory (default: ./target/profiling)

set -euo pipefail

PERF_FREQ="${PERF_FREQ:-999}"
PROFILING_OUTPUT="${PROFILING_OUTPUT:-./target/profiling}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Colors for terminal output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[prof]${NC} $*"; }
warn()  { echo -e "${YELLOW}[prof]${NC} $*"; }
error() { echo -e "${RED}[prof]${NC} $*" >&2; }

check_deps() {
    local missing=0
    if ! command -v perf &>/dev/null; then
        error "perf not found. Install: apt install linux-tools-generic || yum install perf"
        missing=1
    fi
    if ! command -v inferno-flamegraph &>/dev/null; then
        warn "inferno-flamegraph not found. Install: cargo install inferno"
        warn "Flamegraph generation will be skipped."
    fi
    return $missing
}

ensure_dir() {
    mkdir -p "$PROFILING_OUTPUT"
}

# --- Profile a criterion benchmark ---
profile_bench() {
    local bench_name="${1:-registry_search}"
    local timestamp=$(date +%Y%m%d_%H%M%S)
    local perf_data="${PROFILING_OUTPUT}/${bench_name}_${timestamp}.data"

    ensure_dir
    check_deps || exit 1

    info "Building benchmark '${bench_name}' with debug symbols..."
    (
        cd "$REPO_ROOT"
        CARGO_PROFILE_RELEASE_DEBUG=true \
        cargo build --release --bench "$bench_name"
    )

    # Find the actual benchmark binary
    local binary
    binary=$(find "${REPO_ROOT}/target/release/deps/" \
        -name "${bench_name}-*" -type f -executable -not -name '*.d' 2>/dev/null \
        | head -1)

    if [[ -z "$binary" ]]; then
        error "Could not find benchmark binary for '${bench_name}'"
        error "Expected pattern: target/release/deps/${bench_name}-<hash>"
        exit 1
    fi

    info "Profiling ${binary} at ${PERF_FREQ} Hz..."
    perf record \
        -F "$PERF_FREQ" \
        -g \
        --call-graph dwarf \
        -o "$perf_data" \
        -- "$binary" --profile-time 10

    info "Perf data written to: ${perf_data}"
    info "Generate flamegraph with: $0 flamegraph ${perf_data}"

    # Auto-generate flamegraph if inferno is available
    if command -v inferno-flamegraph &>/dev/null; then
        generate_flamegraph "$perf_data"
    fi
}

# --- Profile the running daemon ---
profile_daemon() {
    local duration="${1:-30}"
    local timestamp=$(date +%Y%m%d_%H%M%S)
    local perf_data="${PROFILING_OUTPUT}/daemon_${timestamp}.data"

    ensure_dir
    check_deps || exit 1

    # Find daemon PID
    local pid=$(pgrep -x gatemini 2>/dev/null || true)
    if [[ -z "$pid" ]]; then
        error "No running gatemini daemon found."
        error "Start with: gatemini serve"
        exit 1
    fi

    info "Profiling daemon (PID ${pid}) for ${duration}s at ${PERF_FREQ} Hz..."
    perf record \
        -F "$PERF_FREQ" \
        -g \
        --call-graph dwarf \
        -p "$pid" \
        -o "$perf_data" \
        sleep "$duration"

    info "Perf data written to: ${perf_data}"

    if command -v inferno-flamegraph &>/dev/null; then
        generate_flamegraph "$perf_data"
    fi
}

# --- Generate flamegraph from perf.data ---
generate_flamegraph() {
    local perf_data="${1:-${PROFILING_OUTPUT}/perf.data}"
    local output="${perf_data%.data}.svg"

    if [[ ! -f "$perf_data" ]]; then
        error "No perf data at: ${perf_data}"
        exit 1
    fi

    if ! command -v inferno-flamegraph &>/dev/null; then
        error "inferno-flamegraph not found. Install: cargo install inferno"
        exit 1
    fi

    info "Generating flamegraph: ${output}"
    perf script -i "$perf_data" \
        | inferno-flamegraph \
            --width 2400 \
            --min-width 0.01 \
            --title "PrismGate CPU Flamegraph" \
        > "$output"

    # Also generate a collapsed stack file for downstream tools
    local collapsed="${perf_data%.data}.collapsed"
    perf script -i "$perf_data" \
        | inferno-collapse-perf \
        > "$collapsed" 2>/dev/null || true

    info "Flamegraph: ${output}"
    info "Collapsed stacks: ${collapsed}"
    info "Open in browser: file://${output}"
}

# --- Summary report ---
show_report() {
    local perf_data="${1:-${PROFILING_OUTPUT}/perf.data}"
    if [[ ! -f "$perf_data" ]]; then
        error "No perf data at: ${perf_data}"
        exit 1
    fi

    info "=== Profiling Summary: ${perf_data} ==="
    perf report -i "$perf_data" --stdio --percent-limit 1 2>/dev/null \
        | head -80
}

usage() {
    cat <<EOF
PrismGate CPU Profiling Tool

Usage:
  $(basename "$0") <command> [args]

Commands:
  bench [NAME]       Profile a criterion benchmark (default: registry_search)
  daemon [SECS]      Profile running daemon for N seconds (default: 30)
  flamegraph [FILE]  Generate flamegraph SVG from perf.data
  report [FILE]      Show text summary of perf.data

Environment:
  PERF_FREQ          Sampling frequency in Hz (default: 999)
  PROFILING_OUTPUT   Output directory (default: ./target/profiling)

Examples:
  $(basename "$0") bench registry_search
  $(basename "$0") daemon 60
  $(basename "$0") flamegraph target/profiling/registry_search_20260525.svg
  PERF_FREQ=4999 $(basename "$0") bench

Requirements:
  perf       — apt install linux-tools-generic
  inferno    — cargo install inferno
EOF
}

# --- Main dispatch ---
case "${1:-help}" in
    bench)
        shift || true
        profile_bench "$@"
        ;;
    daemon)
        shift || true
        profile_daemon "$@"
        ;;
    flamegraph)
        shift || true
        generate_flamegraph "$@"
        ;;
    report)
        shift || true
        show_report "$@"
        ;;
    help|--help|-h)
        usage
        ;;
    *)
        error "Unknown command: $1"
        usage
        exit 1
        ;;
esac
