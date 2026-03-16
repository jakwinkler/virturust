#!/bin/bash
# Corten vs Docker — Container Startup Benchmark
#
# Measures how fast each runtime can start and stop containers.
# Tests: single start, 20 sequential, 20 parallel (Docker only for parallel).
#
# Usage: ./scripts/startup-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"
RUNS=20

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

cleanup() {
    bold "[cleanup]"
    for i in $(seq 1 $RUNS); do
        docker rm -f "bench-d-$i" 2>/dev/null || true
        $CORTEN rm "bench-c-$i" 2>/dev/null || true
    done
    docker rm -f bench-docker-single 2>/dev/null || true
    $CORTEN rm bench-corten-single 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Container Startup Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  Runs: $RUNS"
echo ""

# Ensure alpine is available
$CORTEN pull alpine >/dev/null 2>&1 || true
docker pull alpine:3.20 -q >/dev/null 2>&1 || true

# ============================================================================
bold "[1] Single container: start + echo + exit"
echo ""

# Docker
START=$(date +%s%N)
docker run --rm --name bench-docker-single alpine:3.20 echo "hello" >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_SINGLE=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_SINGLE}ms"

# Corten
START=$(date +%s%N)
$CORTEN run --rm --name bench-corten-single --network none alpine echo "hello" >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_SINGLE=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_SINGLE}ms"

echo ""

# ============================================================================
bold "[2] Sequential: $RUNS containers start + echo + exit"
echo ""

# Docker
START=$(date +%s%N)
for i in $(seq 1 $RUNS); do
    docker run --rm --name "bench-d-$i" alpine:3.20 echo "hello" >/dev/null 2>&1
done
END=$(date +%s%N)
DOCKER_SEQ_MS=$(( (END - START) / 1000000 ))
DOCKER_SEQ_PER=$((DOCKER_SEQ_MS / RUNS))
DOCKER_SEQ_RPS=$(echo "scale=1; $RUNS * 1000 / $DOCKER_SEQ_MS" | bc)
echo "  Docker: ${DOCKER_SEQ_MS}ms total, ${DOCKER_SEQ_PER}ms/container, ${DOCKER_SEQ_RPS}/sec"

# Corten
START=$(date +%s%N)
for i in $(seq 1 $RUNS); do
    $CORTEN run --rm --name "bench-c-$i" --network none alpine echo "hello" >/dev/null 2>&1
done
END=$(date +%s%N)
CORTEN_SEQ_MS=$(( (END - START) / 1000000 ))
CORTEN_SEQ_PER=$((CORTEN_SEQ_MS / RUNS))
CORTEN_SEQ_RPS=$(echo "scale=1; $RUNS * 1000 / $CORTEN_SEQ_MS" | bc)
echo "  Corten: ${CORTEN_SEQ_MS}ms total, ${CORTEN_SEQ_PER}ms/container, ${CORTEN_SEQ_RPS}/sec"

echo ""

# ============================================================================
bold "[3] Binary size"
echo ""

CORTEN_SIZE=$(du -h "$(command -v corten || echo ./target/release/corten)" 2>/dev/null | cut -f1)
DOCKER_SIZE=$(du -h "$(which docker)" 2>/dev/null | cut -f1)
DOCKERD_SIZE=$(du -h "$(which dockerd 2>/dev/null || echo /usr/bin/dockerd)" 2>/dev/null | cut -f1)
CONTAINERD_SIZE=$(du -h "$(which containerd 2>/dev/null || echo /usr/bin/containerd)" 2>/dev/null | cut -f1)

echo "  Corten binary:     $CORTEN_SIZE"
echo "  Docker CLI:        $DOCKER_SIZE"
echo "  dockerd:           $DOCKERD_SIZE"
echo "  containerd:        $CONTAINERD_SIZE"

echo ""

# ============================================================================
bold "[4] Memory overhead (idle)"
echo ""

DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")
DOCKERD_RSS=0; CONTAINERD_RSS=0
[ -n "$DOCKERD_PID" ] && [ -f "/proc/$DOCKERD_PID/status" ] && DOCKERD_RSS=$(grep VmRSS "/proc/$DOCKERD_PID/status" | awk '{print $2}')
[ -n "$CONTAINERD_PID" ] && [ -f "/proc/$CONTAINERD_PID/status" ] && CONTAINERD_RSS=$(grep VmRSS "/proc/$CONTAINERD_PID/status" | awk '{print $2}')
DOCKER_DAEMON_MB=$(( (DOCKERD_RSS + CONTAINERD_RSS) / 1024 ))

echo "  Docker daemon RSS: ${DOCKER_DAEMON_MB} MB (dockerd + containerd, always running)"
echo "  Corten daemon RSS: 0 MB (no daemon)"

echo ""

# ============================================================================
bold "[5] Alpine rootfs size"
echo ""

DOCKER_IMG=$(docker image inspect alpine:3.20 --format '{{.Size}}' 2>/dev/null || echo "0")
DOCKER_IMG_MB=$((DOCKER_IMG / 1024 / 1024))
CORTEN_IMG=$(du -sb /var/lib/corten/images/alpine/latest/rootfs 2>/dev/null | cut -f1 || echo "0")
CORTEN_IMG_MB=$((CORTEN_IMG / 1024 / 1024))

echo "  Docker Alpine:  ${DOCKER_IMG_MB} MB"
echo "  Corten Alpine:  ${CORTEN_IMG_MB} MB"

echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary"
bold "========================================================"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %13sms %13sms\n" "Single start+echo+exit" "$DOCKER_SINGLE" "$CORTEN_SINGLE"
printf "  %-30s %13sms %13sms\n" "Sequential ${RUNS}x (total)" "$DOCKER_SEQ_MS" "$CORTEN_SEQ_MS"
printf "  %-30s %13sms %13sms\n" "Per container" "$DOCKER_SEQ_PER" "$CORTEN_SEQ_PER"
printf "  %-30s %12s/s %12s/s\n" "Containers/sec" "$DOCKER_SEQ_RPS" "$CORTEN_SEQ_RPS"
printf "  %-30s %12s MB %12s MB\n" "Daemon overhead" "$DOCKER_DAEMON_MB" "0"
printf "  %-30s %12s MB %12s MB\n" "Alpine image size" "$DOCKER_IMG_MB" "$CORTEN_IMG_MB"
echo ""
