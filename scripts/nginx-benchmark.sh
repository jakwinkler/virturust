#!/bin/bash
# Corten vs Docker — Nginx Benchmark
#
# Builds nginx in both runtimes, runs them side by side,
# and compares startup time, memory usage, and requests/sec.
#
# Prerequisites: sudo, ab (httpd-tools), docker running
# Usage: sudo ./scripts/nginx-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }
yellow() { echo -e "\033[33m$*\033[0m"; }

if [ ! -f "$CORTEN" ]; then
    red "Build first: make build"
    exit 1
fi

if ! command -v docker &>/dev/null; then
    red "Docker not found — install Docker to compare"
    exit 1
fi

if ! command -v ab &>/dev/null; then
    red "ab not found — install with: sudo dnf install httpd-tools"
    exit 1
fi

DOCKER_PORT=9080
CORTEN_PORT=9081
AB_REQUESTS=10000
AB_CONCURRENCY=50

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-nginx 2>/dev/null || true
    $CORTEN stop bench-corten-nginx 2>/dev/null || true
    $CORTEN rm bench-corten-nginx 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Nginx Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  ab:     $(ab -V 2>&1 | head -1)"
echo "  Requests: $AB_REQUESTS @ $AB_CONCURRENCY concurrent"
echo ""

# ============================================================================
bold "[1] Build nginx images"
echo ""

# --- Docker ---
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache nginx && \
    mkdir -p /run/nginx /var/www/html && \
    echo '<h1>Docker Nginx</h1>' > /var/www/html/index.html
CMD ["nginx", "-g", "daemon off;"]
DOCKERFILE

echo "  Building Docker nginx..."
START=$(date +%s%N)
docker build -t bench-nginx "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Docker build: ${DOCKER_BUILD_MS}ms"
rm -rf "$DOCKER_DIR"

# --- Corten ---
echo "  Building Corten nginx..."
# Ensure alpine is pulled
$CORTEN pull alpine >/dev/null 2>&1 || true

CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-nginx"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["nginx"]

[setup]
run = [
    "mkdir -p /run/nginx /var/www/html /var/log/nginx /var/lib/nginx/tmp",
    "echo '<h1>Corten Nginx</h1>' > /var/www/html/index.html",
    "chown -R nginx:nginx /run/nginx /var/log/nginx /var/lib/nginx",
    "printf 'server {\\n  listen 80 default_server;\\n  root /var/www/html;\\n  location / { try_files $uri $uri/ =404; }\\n}\\n' > /etc/nginx/http.d/default.conf",
    "sed -i 's/^user nginx;/user root;/' /etc/nginx/nginx.conf",
]

[container]
command = ["/bin/sh", "-c", "mkdir -p /run/nginx && exec nginx -g 'daemon off;'"]
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
docker run -d --name bench-docker-nginx -p $DOCKER_PORT:80 bench-nginx >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
echo "  Docker start: ${DOCKER_START_MS}ms (port $DOCKER_PORT)"

# --- Corten ---
START=$(date +%s%N)
$CORTEN run -d --name bench-corten-nginx -p $CORTEN_PORT:80 bench-nginx >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
echo "  Corten start: ${CORTEN_START_MS}ms (port $CORTEN_PORT)"

# Wait for both to be ready
echo "  Waiting for nginx..."
DOCKER_OK=false
CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        DOCKER_UP=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$DOCKER_PORT/ 2>/dev/null || echo "000")
        [ "$DOCKER_UP" = "200" ] && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        CORTEN_UP=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$CORTEN_PORT/ 2>/dev/null || echo "000")
        [ "$CORTEN_UP" = "200" ] && CORTEN_OK=true
    fi
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

# Verify both are serving
echo -n "  Docker: "
if $DOCKER_OK; then
    curl -s http://127.0.0.1:$DOCKER_PORT/ | head -1
else
    red "NOT RESPONDING on port $DOCKER_PORT"
fi
echo -n "  Corten: "
if $CORTEN_OK; then
    curl -s http://127.0.0.1:$CORTEN_PORT/ | head -1
else
    red "NOT RESPONDING on port $CORTEN_PORT"
    echo "  Checking logs..."
    $CORTEN logs bench-corten-nginx 2>/dev/null || echo "  (no logs)"
fi

if ! $DOCKER_OK || ! $CORTEN_OK; then
    red "  One or both containers failed to start. Aborting benchmark."
    exit 1
fi

echo ""

# ============================================================================
bold "[3] Memory usage"
echo ""

# Docker container PID and memory
DOCKER_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-nginx 2>/dev/null || echo "0")
if [ "$DOCKER_PID" != "0" ] && [ -f "/proc/$DOCKER_PID/status" ]; then
    DOCKER_RSS=$(grep VmRSS "/proc/$DOCKER_PID/status" | awk '{print $2}')
    echo "  Docker nginx RSS:  ${DOCKER_RSS} KB"
else
    DOCKER_RSS="?"
    echo "  Docker nginx RSS:  (could not read)"
fi

# Corten container PID and memory
CORTEN_STATE=$($CORTEN inspect bench-corten-nginx 2>/dev/null || echo "")
CORTEN_PID=$(echo "$CORTEN_STATE" | grep "^PID:" | awk '{print $2}')
if [ -n "$CORTEN_PID" ] && [ "$CORTEN_PID" != "-" ] && [ -f "/proc/$CORTEN_PID/status" ]; then
    CORTEN_RSS=$(grep VmRSS "/proc/$CORTEN_PID/status" | awk '{print $2}')
    echo "  Corten nginx RSS:  ${CORTEN_RSS} KB"
else
    CORTEN_RSS="?"
    echo "  Corten nginx RSS:  (could not read)"
fi

# Docker daemon overhead
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
bold "[4] Benchmark: $AB_REQUESTS requests @ $AB_CONCURRENCY concurrent"
echo ""

# --- Docker ---
echo "  Benchmarking Docker nginx..."
DOCKER_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT/ 2>&1)
DOCKER_RPS=$(echo "$DOCKER_AB" | grep "Requests per second" | awk '{print $4}')
DOCKER_MEAN=$(echo "$DOCKER_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
DOCKER_P50=$(echo "$DOCKER_AB" | grep "50%" | awk '{print $2}')
DOCKER_P99=$(echo "$DOCKER_AB" | grep "99%" | awk '{print $2}')
DOCKER_FAILED=$(echo "$DOCKER_AB" | grep "Failed requests" | awk '{print $3}')

echo "    Req/s:    $DOCKER_RPS"
echo "    Mean:     ${DOCKER_MEAN}ms"
echo "    P50:      ${DOCKER_P50}ms"
echo "    P99:      ${DOCKER_P99}ms"
echo "    Failed:   $DOCKER_FAILED"
echo ""

# --- Corten ---
echo "  Benchmarking Corten nginx..."
CORTEN_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT/ 2>&1)
CORTEN_RPS=$(echo "$CORTEN_AB" | grep "Requests per second" | awk '{print $4}')
CORTEN_MEAN=$(echo "$CORTEN_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
CORTEN_P50=$(echo "$CORTEN_AB" | grep "50%" | awk '{print $2}')
CORTEN_P99=$(echo "$CORTEN_AB" | grep "99%" | awk '{print $2}')
CORTEN_FAILED=$(echo "$CORTEN_AB" | grep "Failed requests" | awk '{print $3}')

echo "    Req/s:    $CORTEN_RPS"
echo "    Mean:     ${CORTEN_MEAN}ms"
echo "    P50:      ${CORTEN_P50}ms"
echo "    P99:      ${CORTEN_P99}ms"
echo "    Failed:   $CORTEN_FAILED"
echo ""

# ============================================================================
bold "[5] Image size"
echo ""

DOCKER_IMG_SIZE=$(docker image inspect bench-nginx --format '{{.Size}}' 2>/dev/null || echo "0")
DOCKER_IMG_MB=$((DOCKER_IMG_SIZE / 1024 / 1024))
echo "  Docker image:  ${DOCKER_IMG_MB} MB"

CORTEN_ROOTFS=$(du -s /var/lib/corten/images/bench-nginx/latest/rootfs 2>/dev/null | cut -f1 || echo "0")
CORTEN_IMG_MB=$((CORTEN_ROOTFS / 1024))
echo "  Corten rootfs: ${CORTEN_IMG_MB} MB"

echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary"
bold "========================================================"
echo ""
printf "  %-25s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-25s %15s %15s\n" "-------------------------" "---------------" "---------------"
printf "  %-25s %13sms %13sms\n" "Image build time" "$DOCKER_BUILD_MS" "$CORTEN_BUILD_MS"
printf "  %-25s %13sms %13sms\n" "Container start time" "$DOCKER_START_MS" "$CORTEN_START_MS"
printf "  %-25s %12s MB %12s MB\n" "Image size" "$DOCKER_IMG_MB" "$CORTEN_IMG_MB"
printf "  %-25s %12s KB %12s KB\n" "Nginx RSS memory" "$DOCKER_RSS" "$CORTEN_RSS"
printf "  %-25s %12s MB %13s\n" "Daemon overhead" "$DAEMON_MB" "0 MB"
printf "  %-25s %12s/s %12s/s\n" "Requests/sec" "$DOCKER_RPS" "$CORTEN_RPS"
printf "  %-25s %13sms %13sms\n" "Latency (mean)" "$DOCKER_MEAN" "$CORTEN_MEAN"
printf "  %-25s %13sms %13sms\n" "Latency (P50)" "$DOCKER_P50" "$CORTEN_P50"
printf "  %-25s %13sms %13sms\n" "Latency (P99)" "$DOCKER_P99" "$CORTEN_P99"
echo ""
