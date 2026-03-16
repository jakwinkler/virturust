#!/bin/bash
# Corten vs Docker — Redis Benchmark
#
# Builds Redis in both runtimes, runs them side by side,
# and compares startup time, memory usage, and operations/sec.
#
# Prerequisites: make install, docker, redis-benchmark (redis)
# Usage: ./scripts/redis-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

if [ ! -x "$CORTEN" ] && [ ! -f "$CORTEN" ]; then
    red "Corten not found. Run: make install"
    exit 1
fi

if ! command -v docker &>/dev/null; then
    red "Docker not found"
    exit 1
fi

if ! command -v redis-benchmark &>/dev/null; then
    red "redis-benchmark not found. Install: sudo dnf install redis"
    exit 1
fi

DOCKER_PORT=16379
CORTEN_PORT=16380
CONTAINER_PORT=6379
BENCH_REQUESTS=10000
BENCH_CLIENTS=50

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-redis 2>/dev/null || true
    $CORTEN stop bench-corten-redis 2>/dev/null || true
    $CORTEN rm bench-corten-redis 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Redis Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  redis-benchmark: $(redis-benchmark --version 2>&1 | head -1)"
echo "  Requests: $BENCH_REQUESTS @ $BENCH_CLIENTS concurrent"
echo ""

# ============================================================================
bold "[1] Build Redis images"
echo ""

# --- Docker ---
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache redis && \
    sed -i 's/bind 127.0.0.1/bind 0.0.0.0/' /etc/redis.conf && \
    sed -i 's/protected-mode yes/protected-mode no/' /etc/redis.conf
CMD ["redis-server", "/etc/redis.conf"]
DOCKERFILE

echo "  Building Docker Redis..."
START=$(date +%s%N)
docker build -t bench-redis "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Docker build: ${DOCKER_BUILD_MS}ms"
rm -rf "$DOCKER_DIR"

# --- Corten ---
echo "  Building Corten Redis..."
$CORTEN pull alpine >/dev/null 2>&1 || true

CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-redis"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["redis"]

[setup]
run = [
    "sed -i 's/bind 127.0.0.1/bind 0.0.0.0/' /etc/redis.conf",
    "sed -i 's/protected-mode yes/protected-mode no/' /etc/redis.conf",
]

[container]
command = ["/usr/bin/redis-server", "/etc/redis.conf"]
TOML

START=$(date +%s%N)
$CORTEN build "$CORTEN_DIR" >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Corten build: ${CORTEN_BUILD_MS}ms"
rm -rf "$CORTEN_DIR"

echo ""

# ============================================================================
bold "[2] Start containers"
echo ""

# --- Docker ---
START=$(date +%s%N)
docker run -d --name bench-docker-redis -p $DOCKER_PORT:$CONTAINER_PORT bench-redis >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
echo "  Docker start: ${DOCKER_START_MS}ms (port $DOCKER_PORT)"

# --- Corten ---
START=$(date +%s%N)
$CORTEN run -d --name bench-corten-redis -p $CORTEN_PORT:$CONTAINER_PORT bench-redis >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
echo "  Corten start: ${CORTEN_START_MS}ms (port $CORTEN_PORT)"

# Wait for Redis to be ready
echo "  Waiting for Redis to accept connections..."
DOCKER_OK=false
CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        redis-cli -h 127.0.0.1 -p $DOCKER_PORT PING 2>/dev/null | grep -q PONG && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        redis-cli -h 127.0.0.1 -p $CORTEN_PORT PING 2>/dev/null | grep -q PONG && CORTEN_OK=true
    fi
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

echo -n "  Docker: "
if $DOCKER_OK; then
    green "READY"
else
    red "NOT RESPONDING on port $DOCKER_PORT"
fi
echo -n "  Corten: "
if $CORTEN_OK; then
    green "READY"
else
    red "NOT RESPONDING on port $CORTEN_PORT"
    $CORTEN logs bench-corten-redis 2>/dev/null | tail -3 || echo "  (no logs)"
fi

if ! $DOCKER_OK || ! $CORTEN_OK; then
    red "  One or both containers failed to start. Aborting."
    exit 1
fi

echo ""

# ============================================================================
bold "[3] Memory usage"
echo ""

DOCKER_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-redis 2>/dev/null || echo "0")
if [ "$DOCKER_PID" != "0" ] && [ -f "/proc/$DOCKER_PID/status" ]; then
    DOCKER_RSS=$(grep VmRSS "/proc/$DOCKER_PID/status" | awk '{print $2}')
    echo "  Docker redis RSS:   ${DOCKER_RSS} KB"
else
    DOCKER_RSS="?"
    echo "  Docker redis RSS:   (could not read)"
fi

CORTEN_STATE=$($CORTEN inspect bench-corten-redis 2>/dev/null || echo "")
CORTEN_PID=$(echo "$CORTEN_STATE" | grep "^PID:" | awk '{print $2}')
if [ -n "$CORTEN_PID" ] && [ "$CORTEN_PID" != "-" ] && [ -f "/proc/$CORTEN_PID/status" ]; then
    CORTEN_RSS=$(grep VmRSS "/proc/$CORTEN_PID/status" | awk '{print $2}')
    echo "  Corten redis RSS:   ${CORTEN_RSS} KB"
else
    CORTEN_RSS="?"
    echo "  Corten redis RSS:   (could not read)"
fi

DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")
DAEMON_RSS=0
if [ -n "$DOCKERD_PID" ] && [ -f "/proc/$DOCKERD_PID/status" ]; then
    D_RSS=$(grep VmRSS "/proc/$DOCKERD_PID/status" | awk '{print $2}')
    DAEMON_RSS=$((DAEMON_RSS + D_RSS))
fi
if [ -n "$CONTAINERD_PID" ] && [ -f "/proc/$CONTAINERD_PID/status" ]; then
    C_RSS=$(grep VmRSS "/proc/$CONTAINERD_PID/status" | awk '{print $2}')
    DAEMON_RSS=$((DAEMON_RSS + C_RSS))
fi
DAEMON_MB=$((DAEMON_RSS / 1024))
echo ""
echo "  Docker daemon overhead: ${DAEMON_MB} MB (dockerd + containerd)"
echo "  Corten daemon overhead: 0 MB (no daemon)"

echo ""

# ============================================================================
bold "[4] Benchmark: redis-benchmark ($BENCH_REQUESTS requests @ $BENCH_CLIENTS clients)"
echo ""

# --- Docker ---
echo "  Benchmarking Docker Redis..."
DOCKER_BENCH=$(redis-benchmark -h 127.0.0.1 -p $DOCKER_PORT -n $BENCH_REQUESTS -c $BENCH_CLIENTS -t set,get,incr,lpush --csv 2>/dev/null)
DOCKER_SET=$(echo "$DOCKER_BENCH" | grep '"SET"' | cut -d, -f2 | tr -d '"')
DOCKER_GET=$(echo "$DOCKER_BENCH" | grep '"GET"' | cut -d, -f2 | tr -d '"')
DOCKER_INCR=$(echo "$DOCKER_BENCH" | grep '"INCR"' | cut -d, -f2 | tr -d '"')
DOCKER_LPUSH=$(echo "$DOCKER_BENCH" | grep '"LPUSH"' | cut -d, -f2 | tr -d '"')
echo "    SET: ${DOCKER_SET}/s  GET: ${DOCKER_GET}/s  INCR: ${DOCKER_INCR}/s  LPUSH: ${DOCKER_LPUSH}/s"

echo ""

# --- Corten ---
echo "  Benchmarking Corten Redis..."
CORTEN_BENCH=$(redis-benchmark -h 127.0.0.1 -p $CORTEN_PORT -n $BENCH_REQUESTS -c $BENCH_CLIENTS -t set,get,incr,lpush --csv 2>/dev/null)
CORTEN_SET=$(echo "$CORTEN_BENCH" | grep '"SET"' | cut -d, -f2 | tr -d '"')
CORTEN_GET=$(echo "$CORTEN_BENCH" | grep '"GET"' | cut -d, -f2 | tr -d '"')
CORTEN_INCR=$(echo "$CORTEN_BENCH" | grep '"INCR"' | cut -d, -f2 | tr -d '"')
CORTEN_LPUSH=$(echo "$CORTEN_BENCH" | grep '"LPUSH"' | cut -d, -f2 | tr -d '"')
echo "    SET: ${CORTEN_SET}/s  GET: ${CORTEN_GET}/s  INCR: ${CORTEN_INCR}/s  LPUSH: ${CORTEN_LPUSH}/s"

echo ""

# ============================================================================
bold "[5] Image size"
echo ""

DOCKER_IMG_SIZE=$(docker image inspect bench-redis --format '{{.Size}}' 2>/dev/null || echo "0")
DOCKER_IMG_MB=$((DOCKER_IMG_SIZE / 1024 / 1024))
echo "  Docker image:  ${DOCKER_IMG_MB} MB"

CORTEN_ROOTFS=$(du -sb /var/lib/corten/images/bench-redis/latest/rootfs 2>/dev/null | cut -f1 || echo "0")
CORTEN_IMG_MB=$((CORTEN_ROOTFS / 1024 / 1024))
echo "  Corten rootfs: ${CORTEN_IMG_MB} MB"

echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary"
bold "========================================================"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %13sms %13sms\n" "Image build time" "$DOCKER_BUILD_MS" "$CORTEN_BUILD_MS"
printf "  %-30s %13sms %13sms\n" "Container start time" "$DOCKER_START_MS" "$CORTEN_START_MS"
printf "  %-30s %12s MB %12s MB\n" "Image size" "$DOCKER_IMG_MB" "$CORTEN_IMG_MB"
printf "  %-30s %12s KB %12s KB\n" "Redis RSS memory" "$DOCKER_RSS" "$CORTEN_RSS"
printf "  %-30s %12s MB %13s\n" "Daemon overhead" "$DAEMON_MB" "0 MB"
printf "  %-30s %12s/s %12s/s\n" "SET req/sec" "$DOCKER_SET" "$CORTEN_SET"
printf "  %-30s %12s/s %12s/s\n" "GET req/sec" "$DOCKER_GET" "$CORTEN_GET"
printf "  %-30s %12s/s %12s/s\n" "INCR req/sec" "$DOCKER_INCR" "$CORTEN_INCR"
printf "  %-30s %12s/s %12s/s\n" "LPUSH req/sec" "$DOCKER_LPUSH" "$CORTEN_LPUSH"
echo ""
