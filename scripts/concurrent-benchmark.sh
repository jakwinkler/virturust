#!/bin/bash
# Corten vs Docker — Concurrent Container Stress Test
#
# Starts 20 containers simultaneously, measures total time,
# memory usage, then destroys them all.
#
# Usage: ./scripts/concurrent-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"
COUNT=20

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

cleanup() {
    bold "[cleanup]"
    for i in $(seq 1 $COUNT); do
        docker rm -f "stress-d-$i" 2>/dev/null &
        $CORTEN stop "stress-c-$i" 2>/dev/null &
    done
    wait
    for i in $(seq 1 $COUNT); do
        $CORTEN rm "stress-c-$i" 2>/dev/null &
    done
    wait
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — $COUNT Concurrent Containers"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  Containers: $COUNT each"
echo ""

# Ensure images exist
docker pull alpine:3.20 -q >/dev/null 2>&1 || true
$CORTEN pull alpine >/dev/null 2>&1 || true

# ============================================================================
bold "[1] Start $COUNT containers in parallel — each runs 'sleep 30'"
echo ""

# --- Docker ---
echo "  Docker: starting $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    docker run -d --name "stress-d-$i" alpine:3.20 sleep 30 >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
DOCKER_RUNNING=$(docker ps -q --filter "name=stress-d-" 2>/dev/null | wc -l)
echo "  Docker: ${DOCKER_START_MS}ms to start $DOCKER_RUNNING containers"

# --- Corten ---
echo "  Corten: starting $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    $CORTEN run -d --name "stress-c-$i" --network none alpine sleep 30 >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
CORTEN_RUNNING=$($CORTEN ps 2>/dev/null | grep "stress-c-.*running" | wc -l)
echo "  Corten: ${CORTEN_START_MS}ms to start $CORTEN_RUNNING containers"

echo ""

# ============================================================================
bold "[2] Memory usage with $COUNT containers running"
echo ""

# Docker per-container overhead
DOCKER_TOTAL_RSS=0
DOCKER_MEASURED=0
for i in $(seq 1 $COUNT); do
    PID=$(docker inspect --format '{{.State.Pid}}' "stress-d-$i" 2>/dev/null || echo "0")
    if [ "$PID" != "0" ] && [ -f "/proc/$PID/status" ]; then
        RSS=$(grep VmRSS "/proc/$PID/status" | awk '{print $2}')
        DOCKER_TOTAL_RSS=$((DOCKER_TOTAL_RSS + RSS))
        DOCKER_MEASURED=$((DOCKER_MEASURED + 1))
    fi
done

# Docker daemon
DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")
DAEMON_RSS=0
[ -n "$DOCKERD_PID" ] && [ -f "/proc/$DOCKERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$DOCKERD_PID/status" | awk '{print $2}')))
[ -n "$CONTAINERD_PID" ] && [ -f "/proc/$CONTAINERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$CONTAINERD_PID/status" | awk '{print $2}')))

# Count containerd-shim processes
SHIM_RSS=0
SHIM_COUNT=0
for PID in $(pgrep -f "containerd-shim" 2>/dev/null); do
    if [ -f "/proc/$PID/status" ]; then
        RSS=$(grep VmRSS "/proc/$PID/status" | awk '{print $2}')
        SHIM_RSS=$((SHIM_RSS + RSS))
        SHIM_COUNT=$((SHIM_COUNT + 1))
    fi
done

DOCKER_TOTAL_MB=$(( (DOCKER_TOTAL_RSS + DAEMON_RSS + SHIM_RSS) / 1024 ))
DOCKER_CONTAINERS_MB=$((DOCKER_TOTAL_RSS / 1024))
DOCKER_DAEMON_MB=$((DAEMON_RSS / 1024))
DOCKER_SHIM_MB=$((SHIM_RSS / 1024))

echo "  Docker ($DOCKER_MEASURED containers measured):"
echo "    Containers RSS:    ${DOCKER_CONTAINERS_MB} MB (${COUNT}x sleep)"
echo "    Daemon RSS:        ${DOCKER_DAEMON_MB} MB (dockerd + containerd)"
echo "    Shim RSS:          ${DOCKER_SHIM_MB} MB (${SHIM_COUNT} containerd-shim processes)"
echo "    TOTAL:             ${DOCKER_TOTAL_MB} MB"

# Corten per-container overhead
CORTEN_TOTAL_RSS=0
CORTEN_MEASURED=0
# Find monitor processes (corten run -d spawns a monitor)
for PID in $(pgrep -f "corten run -d --name stress-c-" 2>/dev/null); do
    if [ -f "/proc/$PID/status" ]; then
        RSS=$(grep VmRSS "/proc/$PID/status" | awk '{print $2}')
        CORTEN_TOTAL_RSS=$((CORTEN_TOTAL_RSS + RSS))
        CORTEN_MEASURED=$((CORTEN_MEASURED + 1))
    fi
done
# Add the actual sleep processes
for i in $(seq 1 $COUNT); do
    STATE=$($CORTEN inspect "stress-c-$i" 2>/dev/null || true)
    PID=$(echo "$STATE" | grep "^PID:" | awk '{print $2}')
    if [ -n "$PID" ] && [ "$PID" != "-" ] && [ -f "/proc/$PID/status" ]; then
        RSS=$(grep VmRSS "/proc/$PID/status" | awk '{print $2}')
        CORTEN_TOTAL_RSS=$((CORTEN_TOTAL_RSS + RSS))
    fi
done

CORTEN_TOTAL_MB=$((CORTEN_TOTAL_RSS / 1024))

echo ""
echo "  Corten ($CORTEN_RUNNING containers):"
echo "    Containers + monitors: ${CORTEN_TOTAL_MB} MB"
echo "    Daemon:                0 MB (no daemon)"
echo "    TOTAL:                 ${CORTEN_TOTAL_MB} MB"

echo ""

# ============================================================================
bold "[3] Stop all $COUNT containers"
echo ""

# Docker
echo "  Docker: stopping $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    docker stop -t 1 "stress-d-$i" >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
DOCKER_STOP_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_STOP_MS}ms"

# Corten
echo "  Corten: stopping $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    $CORTEN stop "stress-c-$i" >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
CORTEN_STOP_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_STOP_MS}ms"

echo ""

# ============================================================================
bold "[4] Remove all $COUNT containers"
echo ""

# Docker
echo "  Docker: removing $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    docker rm "stress-d-$i" >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
DOCKER_RM_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_RM_MS}ms"

# Corten
echo "  Corten: removing $COUNT containers..."
START=$(date +%s%N)
for i in $(seq 1 $COUNT); do
    $CORTEN rm "stress-c-$i" >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
CORTEN_RM_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_RM_MS}ms"

echo ""

# ============================================================================
bold "========================================================"
bold "  Results — $COUNT Concurrent Containers"
bold "========================================================"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %12s/%s %12s/%s\n" "Containers started" "$DOCKER_RUNNING" "$COUNT" "$CORTEN_RUNNING" "$COUNT"
printf "  %-30s %13sms %13sms\n" "Start ${COUNT}x (parallel)" "$DOCKER_START_MS" "$CORTEN_START_MS"
printf "  %-30s %12s MB %12s MB\n" "Total memory (all)" "$DOCKER_TOTAL_MB" "$CORTEN_TOTAL_MB"
printf "  %-30s %12s MB %13s\n" "  Daemon overhead" "$DOCKER_DAEMON_MB" "0 MB"
printf "  %-30s %12s MB %13s\n" "  Shim processes (${SHIM_COUNT}x)" "$DOCKER_SHIM_MB" "N/A"
printf "  %-30s %13sms %13sms\n" "Stop ${COUNT}x (parallel)" "$DOCKER_STOP_MS" "$CORTEN_STOP_MS"
printf "  %-30s %13sms %13sms\n" "Remove ${COUNT}x (parallel)" "$DOCKER_RM_MS" "$CORTEN_RM_MS"
echo ""
