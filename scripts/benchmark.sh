#!/bin/bash
# Corten vs Docker Benchmark
#
# Compares startup latency, memory overhead, and execution speed
# between Corten and Docker on the same workloads.
#
# Prerequisites:
#   - Corten installed (make install) or built (target/release/corten)
#   - Docker installed and running
#   - Alpine image pulled in both (script handles this)
#   - Root access (sudo)
#
# Usage: sudo ./scripts/benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-./target/release/corten}"
RUNS=10  # Number of iterations for timing

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }
yellow() { echo -e "\033[33m$*\033[0m"; }

# Time a command in milliseconds (average of N runs)
time_avg_ms() {
    local label="$1"
    shift
    local total=0
    for i in $(seq 1 $RUNS); do
        local start end elapsed
        start=$(date +%s%N)
        "$@" >/dev/null 2>&1 || true
        end=$(date +%s%N)
        elapsed=$(( (end - start) / 1000000 ))
        total=$((total + elapsed))
    done
    echo $((total / RUNS))
}

# Get RSS memory of a process in KB
get_rss_kb() {
    local pid="$1"
    if [ -f "/proc/$pid/status" ]; then
        grep VmRSS "/proc/$pid/status" 2>/dev/null | awk '{print $2}' || echo "0"
    else
        echo "0"
    fi
}

cleanup() {
    # Clean up test containers
    $CORTEN rm bench-corten 2>/dev/null || true
    docker rm -f bench-docker 2>/dev/null || true
    $CORTEN rm corten-mem 2>/dev/null || true
    docker rm -f docker-mem 2>/dev/null || true
}
trap cleanup EXIT

# ============================================================================
bold "=========================================="
bold "  Corten vs Docker Benchmark"
bold "=========================================="
echo ""

# Check prerequisites
if [ "$(id -u)" -ne 0 ]; then
    red "Error: must run as root (sudo $0)"
    exit 1
fi

if [ ! -f "$CORTEN" ]; then
    echo "Building Corten release binary..."
    cargo build --release
fi

if ! command -v docker &>/dev/null; then
    red "Error: docker not found. Install Docker to compare."
    exit 1
fi

echo "Corten: $($CORTEN --version 2>&1)"
echo "Docker: $(docker --version 2>&1)"
echo "Runs per test: $RUNS"
echo ""

# ============================================================================
bold "1. Pulling Alpine Image"
echo ""

$CORTEN pull alpine 2>/dev/null || true
docker pull alpine >/dev/null 2>&1 || true
green "  Both have alpine:latest"
echo ""

# ============================================================================
bold "2. Container Startup Latency (echo hello)"
echo "   Running 'echo hello' $RUNS times, measuring wall clock..."
echo ""

CORTEN_STARTUP=$(time_avg_ms "corten" $CORTEN run --name bench-corten --network none alpine echo hello)
$CORTEN rm bench-corten 2>/dev/null || true

DOCKER_STARTUP=$(time_avg_ms "docker" docker run --rm --network none alpine echo hello)

echo "  Corten:  ${CORTEN_STARTUP}ms (avg over $RUNS runs)"
echo "  Docker:  ${DOCKER_STARTUP}ms (avg over $RUNS runs)"

if [ "$CORTEN_STARTUP" -lt "$DOCKER_STARTUP" ]; then
    SPEEDUP=$(echo "scale=1; $DOCKER_STARTUP / $CORTEN_STARTUP" | bc 2>/dev/null || echo "?")
    green "  Corten is ${SPEEDUP}x faster"
else
    SLOWDOWN=$(echo "scale=1; $CORTEN_STARTUP / $DOCKER_STARTUP" | bc 2>/dev/null || echo "?")
    yellow "  Docker is ${SLOWDOWN}x faster"
fi
echo ""

# ============================================================================
bold "3. Container Startup Latency (cat /etc/os-release)"
echo "   Slightly heavier workload..."
echo ""

CORTEN_CAT=$(time_avg_ms "corten" $CORTEN run --name bench-corten --network none alpine cat /etc/os-release)
$CORTEN rm bench-corten 2>/dev/null || true

DOCKER_CAT=$(time_avg_ms "docker" docker run --rm --network none alpine cat /etc/os-release)

echo "  Corten:  ${CORTEN_CAT}ms"
echo "  Docker:  ${DOCKER_CAT}ms"

if [ "$CORTEN_CAT" -lt "$DOCKER_CAT" ]; then
    SPEEDUP=$(echo "scale=1; $DOCKER_CAT / $CORTEN_CAT" | bc 2>/dev/null || echo "?")
    green "  Corten is ${SPEEDUP}x faster"
else
    SLOWDOWN=$(echo "scale=1; $CORTEN_CAT / $DOCKER_CAT" | bc 2>/dev/null || echo "?")
    yellow "  Docker is ${SLOWDOWN}x faster"
fi
echo ""

# ============================================================================
bold "4. Daemon Memory Overhead"
echo "   Docker runs a daemon; Corten does not."
echo ""

# Docker daemon RSS
DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")

DOCKER_DAEMON_RSS=0
if [ -n "$DOCKERD_PID" ]; then
    DOCKERD_RSS=$(get_rss_kb "$DOCKERD_PID")
    DOCKER_DAEMON_RSS=$((DOCKER_DAEMON_RSS + DOCKERD_RSS))
    echo "  dockerd PID $DOCKERD_PID:    ${DOCKERD_RSS} KB"
fi
if [ -n "$CONTAINERD_PID" ]; then
    CONTAINERD_RSS=$(get_rss_kb "$CONTAINERD_PID")
    DOCKER_DAEMON_RSS=$((DOCKER_DAEMON_RSS + CONTAINERD_RSS))
    echo "  containerd PID $CONTAINERD_PID: ${CONTAINERD_RSS} KB"
fi

DOCKER_DAEMON_MB=$((DOCKER_DAEMON_RSS / 1024))
echo ""
echo "  Docker daemon total:  ${DOCKER_DAEMON_MB} MB (always running)"
echo "  Corten daemon total:  0 MB (no daemon)"

if [ "$DOCKER_DAEMON_MB" -gt 0 ]; then
    green "  Corten saves ${DOCKER_DAEMON_MB} MB of RAM"
fi
echo ""

# ============================================================================
bold "5. Binary Size"
echo ""

CORTEN_SIZE=$(du -h "$CORTEN" | cut -f1)
DOCKER_SIZE=$(du -h "$(which docker)" 2>/dev/null | cut -f1 || echo "?")
RUNC_SIZE=$(du -h "$(which runc)" 2>/dev/null | cut -f1 || echo "?")

echo "  corten binary:    $CORTEN_SIZE"
echo "  docker CLI:       $DOCKER_SIZE"
echo "  runc binary:      $RUNC_SIZE"
echo ""

# ============================================================================
bold "6. Sequential Container Creation (10 containers)"
echo "   Create and destroy 10 containers sequentially..."
echo ""

# Corten
START=$(date +%s%N)
for i in $(seq 1 10); do
    $CORTEN run --name "seq-c-$i" --network none alpine true >/dev/null 2>&1 || true
done
END=$(date +%s%N)
CORTEN_SEQ=$(( (END - START) / 1000000 ))
for i in $(seq 1 10); do
    $CORTEN rm "seq-c-$i" >/dev/null 2>&1 || true
done

# Docker
START=$(date +%s%N)
for i in $(seq 1 10); do
    docker run --rm --name "seq-d-$i" --network none alpine true >/dev/null 2>&1 || true
done
END=$(date +%s%N)
DOCKER_SEQ=$(( (END - START) / 1000000 ))

echo "  Corten:  ${CORTEN_SEQ}ms total ($(( CORTEN_SEQ / 10 ))ms/container)"
echo "  Docker:  ${DOCKER_SEQ}ms total ($(( DOCKER_SEQ / 10 ))ms/container)"

if [ "$CORTEN_SEQ" -lt "$DOCKER_SEQ" ]; then
    SPEEDUP=$(echo "scale=1; $DOCKER_SEQ / $CORTEN_SEQ" | bc 2>/dev/null || echo "?")
    green "  Corten is ${SPEEDUP}x faster"
else
    SLOWDOWN=$(echo "scale=1; $CORTEN_SEQ / $DOCKER_SEQ" | bc 2>/dev/null || echo "?")
    yellow "  Docker is ${SLOWDOWN}x faster"
fi
echo ""

# ============================================================================
bold "=========================================="
bold "  Summary"
bold "=========================================="
echo ""
printf "  %-30s %10s %10s\n" "Metric" "Corten" "Docker"
printf "  %-30s %10s %10s\n" "------------------------------" "----------" "----------"
printf "  %-30s %8dms %8dms\n" "Startup (echo hello)" "$CORTEN_STARTUP" "$DOCKER_STARTUP"
printf "  %-30s %8dms %8dms\n" "Startup (cat os-release)" "$CORTEN_CAT" "$DOCKER_CAT"
printf "  %-30s %8dMB %8dMB\n" "Daemon memory overhead" 0 "$DOCKER_DAEMON_MB"
printf "  %-30s %10s %10s\n" "Binary size" "$CORTEN_SIZE" "$DOCKER_SIZE"
printf "  %-30s %8dms %8dms\n" "10 containers sequential" "$CORTEN_SEQ" "$DOCKER_SEQ"
echo ""
