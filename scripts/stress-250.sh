#!/bin/bash
# Corten vs Docker — 250 Working Containers
#
# Not just sleep — each container runs a real workload:
# - 50 containers: HTTP servers (busybox httpd)
# - 50 containers: CPU work (calculating checksums)
# - 50 containers: I/O work (writing to disk in a loop)
# - 50 containers: Memory work (allocating and touching pages)
# - 50 containers: Network listeners (nc -l)
#
# Usage: ./scripts/stress-250.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"
PER_TYPE=50
TOTAL=$((PER_TYPE * 5))

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

cleanup() {
    bold "[cleanup]"
    for prefix in http cpu io mem net; do
        for i in $(seq 1 $PER_TYPE); do
            docker rm -f "d-${prefix}-${i}" 2>/dev/null &
            $CORTEN stop "c-${prefix}-${i}" 2>/dev/null &
        done
    done
    wait
    for prefix in http cpu io mem net; do
        for i in $(seq 1 $PER_TYPE); do
            $CORTEN rm "c-${prefix}-${i}" 2>/dev/null &
        done
    done
    wait
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — $TOTAL Working Containers"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  Workloads: ${PER_TYPE}x HTTP, ${PER_TYPE}x CPU, ${PER_TYPE}x I/O, ${PER_TYPE}x Memory, ${PER_TYPE}x Network"
echo ""

# Ensure alpine
docker pull alpine:3.20 -q >/dev/null 2>&1 || true
$CORTEN pull alpine >/dev/null 2>&1 || true

# Workload commands
HTTP_CMD="/bin/sh -c 'mkdir -p /www && echo ok > /www/index.html && httpd -f -p 8080 -h /www'"
CPU_CMD="/bin/sh -c 'while true; do md5sum /etc/hostname > /dev/null 2>&1; done'"
IO_CMD="/bin/sh -c 'while true; do dd if=/dev/zero of=/tmp/out bs=4k count=256 2>/dev/null; rm -f /tmp/out; sleep 0.1; done'"
MEM_CMD="/bin/sh -c 'dd if=/dev/zero of=/tmp/mem bs=1M count=4 2>/dev/null; while true; do cat /tmp/mem > /dev/null; sleep 0.5; done'"
NET_CMD="/bin/sh -c 'while true; do echo ok | nc -l -p 9999 2>/dev/null || sleep 0.1; done'"

# ============================================================================
bold "[1] Start $TOTAL containers in parallel"
echo ""

# --- Docker ---
echo "  Docker: starting $TOTAL containers..."
START=$(date +%s%N)
for i in $(seq 1 $PER_TYPE); do
    docker run -d --name "d-http-$i" alpine:3.20 /bin/sh -c 'mkdir -p /www && echo ok > /www/index.html && httpd -f -p 8080 -h /www' >/dev/null 2>&1 &
    docker run -d --name "d-cpu-$i" alpine:3.20 /bin/sh -c 'while true; do md5sum /etc/hostname > /dev/null 2>&1; done' >/dev/null 2>&1 &
    docker run -d --name "d-io-$i" alpine:3.20 /bin/sh -c 'while true; do dd if=/dev/zero of=/tmp/out bs=4k count=256 2>/dev/null; rm -f /tmp/out; sleep 0.1; done' >/dev/null 2>&1 &
    docker run -d --name "d-mem-$i" alpine:3.20 /bin/sh -c 'dd if=/dev/zero of=/tmp/mem bs=1M count=4 2>/dev/null; while true; do cat /tmp/mem > /dev/null; sleep 0.5; done' >/dev/null 2>&1 &
    docker run -d --name "d-net-$i" alpine:3.20 /bin/sh -c 'while true; do echo ok | nc -l -p 9999 2>/dev/null || sleep 0.1; done' >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
DOCKER_RUNNING=$(docker ps -q --filter "name=d-" 2>/dev/null | wc -l)
echo "  Docker: ${DOCKER_START_MS}ms — $DOCKER_RUNNING/$TOTAL running"

# --- Corten ---
echo "  Corten: starting $TOTAL containers..."
START=$(date +%s%N)
for i in $(seq 1 $PER_TYPE); do
    $CORTEN run -d --name "c-http-$i" --network none alpine /bin/sh -c 'mkdir -p /www && echo ok > /www/index.html && httpd -f -p 8080 -h /www' >/dev/null 2>&1 &
    $CORTEN run -d --name "c-cpu-$i" --network none alpine /bin/sh -c 'while true; do md5sum /etc/hostname > /dev/null 2>&1; done' >/dev/null 2>&1 &
    $CORTEN run -d --name "c-io-$i" --network none alpine /bin/sh -c 'while true; do dd if=/dev/zero of=/tmp/out bs=4k count=256 2>/dev/null; rm -f /tmp/out; sleep 0.1; done' >/dev/null 2>&1 &
    $CORTEN run -d --name "c-mem-$i" --network none alpine /bin/sh -c 'dd if=/dev/zero of=/tmp/mem bs=1M count=4 2>/dev/null; while true; do cat /tmp/mem > /dev/null; sleep 0.5; done' >/dev/null 2>&1 &
    $CORTEN run -d --name "c-net-$i" --network none alpine /bin/sh -c 'while true; do echo ok | nc -l -p 9999 2>/dev/null || sleep 0.1; done' >/dev/null 2>&1 &
done
wait
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
CORTEN_RUNNING=$($CORTEN ps 2>/dev/null | grep -c "running" || echo "0")
echo "  Corten: ${CORTEN_START_MS}ms — $CORTEN_RUNNING/$TOTAL running"

echo ""

# Let them cook for 5 seconds
bold "[2] Letting containers work for 5 seconds..."
sleep 5
echo "  Done."
echo ""

# ============================================================================
bold "[3] Memory snapshot with $TOTAL containers under load"
echo ""

# Docker
DOCKER_CONTAINER_RSS=0
for PID in $(docker ps -q --filter "name=d-" 2>/dev/null | xargs -I{} docker inspect --format '{{.State.Pid}}' {} 2>/dev/null); do
    [ -f "/proc/$PID/status" ] && DOCKER_CONTAINER_RSS=$((DOCKER_CONTAINER_RSS + $(grep VmRSS "/proc/$PID/status" 2>/dev/null | awk '{print $2}' || echo 0)))
done

DAEMON_RSS=0
DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")
[ -n "$DOCKERD_PID" ] && [ -f "/proc/$DOCKERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$DOCKERD_PID/status" | awk '{print $2}')))
[ -n "$CONTAINERD_PID" ] && [ -f "/proc/$CONTAINERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$CONTAINERD_PID/status" | awk '{print $2}')))

SHIM_RSS=0; SHIM_COUNT=0
for PID in $(pgrep -f "containerd-shim" 2>/dev/null); do
    [ -f "/proc/$PID/status" ] && SHIM_RSS=$((SHIM_RSS + $(grep VmRSS "/proc/$PID/status" | awk '{print $2}'))) && SHIM_COUNT=$((SHIM_COUNT + 1))
done

DOCKER_TOTAL=$((DOCKER_CONTAINER_RSS + DAEMON_RSS + SHIM_RSS))
echo "  Docker:"
echo "    Workload processes:  $((DOCKER_CONTAINER_RSS / 1024)) MB"
echo "    Daemon (dockerd+cd): $((DAEMON_RSS / 1024)) MB"
echo "    Shims (${SHIM_COUNT}x):        $((SHIM_RSS / 1024)) MB"
echo "    TOTAL:               $((DOCKER_TOTAL / 1024)) MB"

# Corten
CORTEN_TOTAL_RSS=0
for PID in $(pgrep -f "corten run -d" 2>/dev/null); do
    [ -f "/proc/$PID/status" ] && CORTEN_TOTAL_RSS=$((CORTEN_TOTAL_RSS + $(grep VmRSS "/proc/$PID/status" 2>/dev/null | awk '{print $2}' || echo 0)))
done
# Add actual workload processes
for prefix in http cpu io mem net; do
    for i in $(seq 1 $PER_TYPE); do
        STATE=$($CORTEN inspect "c-${prefix}-${i}" 2>/dev/null || true)
        PID=$(echo "$STATE" | grep "^PID:" | awk '{print $2}')
        if [ -n "$PID" ] && [ "$PID" != "-" ] && [ -f "/proc/$PID/status" ]; then
            CORTEN_TOTAL_RSS=$((CORTEN_TOTAL_RSS + $(grep VmRSS "/proc/$PID/status" 2>/dev/null | awk '{print $2}' || echo 0)))
        fi
    done
done

echo ""
echo "  Corten:"
echo "    Everything:          $((CORTEN_TOTAL_RSS / 1024)) MB"
echo "    Daemon:              0 MB"
echo "    TOTAL:               $((CORTEN_TOTAL_RSS / 1024)) MB"

echo ""

# ============================================================================
bold "[4] CPU usage snapshot"
echo ""

# Quick CPU check via /proc/stat
DOCKER_CPU_PROCS=0
for PID in $(docker ps -q --filter "name=d-cpu" 2>/dev/null | head -5 | xargs -I{} docker inspect --format '{{.State.Pid}}' {} 2>/dev/null); do
    [ -f "/proc/$PID/stat" ] && DOCKER_CPU_PROCS=$((DOCKER_CPU_PROCS + 1))
done
echo "  Docker: $DOCKER_RUNNING containers running, ${DOCKER_CPU_PROCS} CPU-bound checked"

CORTEN_CPU_PROCS=0
for i in $(seq 1 5); do
    STATE=$($CORTEN inspect "c-cpu-$i" 2>/dev/null || true)
    PID=$(echo "$STATE" | grep "^PID:" | awk '{print $2}')
    [ -n "$PID" ] && [ "$PID" != "-" ] && [ -f "/proc/$PID/stat" ] && CORTEN_CPU_PROCS=$((CORTEN_CPU_PROCS + 1))
done
echo "  Corten: $CORTEN_RUNNING containers running, ${CORTEN_CPU_PROCS} CPU-bound checked"

echo ""

# ============================================================================
bold "[5] Destroy everything"
echo ""

echo "  Docker: stopping + removing $TOTAL containers..."
START=$(date +%s%N)
for prefix in http cpu io mem net; do
    for i in $(seq 1 $PER_TYPE); do
        docker rm -f "d-${prefix}-${i}" >/dev/null 2>&1 &
    done
done
wait
END=$(date +%s%N)
DOCKER_DESTROY_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_DESTROY_MS}ms"

echo "  Corten: stopping + removing $TOTAL containers..."
START=$(date +%s%N)
for prefix in http cpu io mem net; do
    for i in $(seq 1 $PER_TYPE); do
        $CORTEN stop "c-${prefix}-${i}" >/dev/null 2>&1 &
    done
done
wait
for prefix in http cpu io mem net; do
    for i in $(seq 1 $PER_TYPE); do
        $CORTEN rm "c-${prefix}-${i}" >/dev/null 2>&1 &
    done
done
wait
END=$(date +%s%N)
CORTEN_DESTROY_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_DESTROY_MS}ms"

echo ""

# ============================================================================
bold "========================================================"
bold "  Final Results — $TOTAL Working Containers"
bold "========================================================"
echo ""
echo "  Workloads: HTTP servers, CPU hashing, disk I/O, memory, network listeners"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %12s/%s %12s/%s\n" "Containers running" "$DOCKER_RUNNING" "$TOTAL" "$CORTEN_RUNNING" "$TOTAL"
printf "  %-30s %13sms %13sms\n" "Start ${TOTAL}x (parallel)" "$DOCKER_START_MS" "$CORTEN_START_MS"
printf "  %-30s %12s MB %12s MB\n" "Total memory under load" "$((DOCKER_TOTAL / 1024))" "$((CORTEN_TOTAL_RSS / 1024))"
printf "  %-30s %12s MB %13s\n" "  Daemon+shim overhead" "$(( (DAEMON_RSS + SHIM_RSS) / 1024 ))" "0 MB"
printf "  %-30s %13sms %13sms\n" "Destroy ${TOTAL}x" "$DOCKER_DESTROY_MS" "$CORTEN_DESTROY_MS"
echo ""
