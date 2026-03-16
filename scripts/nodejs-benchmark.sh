#!/bin/bash
# Corten vs Docker — Node.js HTTP Server Benchmark
#
# Raw HTTP server performance with a simple Node.js app.
#
# Usage: ./scripts/nodejs-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

if ! command -v ab &>/dev/null; then
    red "ab not found — install: sudo dnf install httpd-tools"
    exit 1
fi

DOCKER_PORT=19080
CORTEN_PORT=19081
AB_REQUESTS=10000
AB_CONCURRENCY=50

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-node 2>/dev/null || true
    $CORTEN stop bench-corten-node 2>/dev/null || true
    $CORTEN rm bench-corten-node 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Node.js HTTP Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  Requests: $AB_REQUESTS @ $AB_CONCURRENCY concurrent"
echo ""

# ============================================================================
bold "[1] Build images"
echo ""

# Docker
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache nodejs
RUN printf 'const http = require("http");\nconst server = http.createServer((req, res) => {\n  res.writeHead(200, {"Content-Type": "application/json"});\n  res.end(JSON.stringify({status: "ok", pid: process.pid, uptime: process.uptime()}));\n});\nserver.listen(8080, "0.0.0.0", () => console.log("listening on 8080"));\n' > /app.js
CMD ["node", "/app.js"]
DOCKERFILE

echo "  Building Docker Node.js..."
START=$(date +%s%N)
docker build -t bench-node "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Docker build: ${DOCKER_BUILD_MS}ms"
rm -rf "$DOCKER_DIR"

# Corten
echo "  Building Corten Node.js..."
CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-node"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["nodejs"]
[setup]
run = [
    "printf 'const http = require(\"http\");\\nconst server = http.createServer((req, res) => {\\n  res.writeHead(200, {\"Content-Type\": \"application/json\"});\\n  res.end(JSON.stringify({status: \"ok\", pid: process.pid, uptime: process.uptime()}));\\n});\\nserver.listen(8080, \"0.0.0.0\", () => console.log(\"listening on 8080\"));\\n' > /app.js",
]
[container]
command = ["/usr/bin/node", "/app.js"]
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

START=$(date +%s%N)
docker run -d --name bench-docker-node -p $DOCKER_PORT:8080 bench-node >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
echo "  Docker start: ${DOCKER_START_MS}ms (port $DOCKER_PORT)"

START=$(date +%s%N)
$CORTEN run -d --name bench-corten-node -p $CORTEN_PORT:8080 bench-node >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
echo "  Corten start: ${CORTEN_START_MS}ms (port $CORTEN_PORT)"

# Wait for ready
echo "  Waiting for Node.js..."
DOCKER_OK=false; CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$DOCKER_PORT/ 2>/dev/null || echo "000")
        [ "$CODE" = "200" ] && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$CORTEN_PORT/ 2>/dev/null || echo "000")
        [ "$CODE" = "200" ] && CORTEN_OK=true
    fi
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

echo -n "  Docker: "; if $DOCKER_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo -n "  Corten: "; if $CORTEN_OK; then green "READY"; else red "NOT RESPONDING"; fi

if ! $DOCKER_OK && ! $CORTEN_OK; then
    red "Both failed. Aborting."; exit 1
fi
echo ""

# ============================================================================
bold "[3] Memory usage"
echo ""

DOCKER_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-node 2>/dev/null || echo "0")
DOCKER_RSS="?"; [ -f "/proc/$DOCKER_PID/status" ] && DOCKER_RSS=$(grep VmRSS "/proc/$DOCKER_PID/status" | awk '{print $2}')
echo "  Docker node RSS:   ${DOCKER_RSS} KB"

CORTEN_PID=$($CORTEN inspect bench-corten-node 2>/dev/null | grep "^PID:" | awk '{print $2}')
CORTEN_RSS="?"; [ -n "$CORTEN_PID" ] && [ -f "/proc/$CORTEN_PID/status" ] && CORTEN_RSS=$(grep VmRSS "/proc/$CORTEN_PID/status" | awk '{print $2}')
echo "  Corten node RSS:   ${CORTEN_RSS} KB"
echo ""

# ============================================================================
if $DOCKER_OK; then
    bold "[4] Benchmark Docker: $AB_REQUESTS requests"
    echo ""
    DOCKER_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT/ 2>&1)
    DOCKER_RPS=$(echo "$DOCKER_AB" | grep "Requests per second" | awk '{print $4}')
    DOCKER_MEAN=$(echo "$DOCKER_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    DOCKER_P50=$(echo "$DOCKER_AB" | grep "50%" | awk '{print $2}')
    DOCKER_P99=$(echo "$DOCKER_AB" | grep "99%" | awk '{print $2}')
    DOCKER_FAILED=$(echo "$DOCKER_AB" | grep "Failed requests" | awk '{print $3}')
    echo "  Docker: ${DOCKER_RPS} req/s | mean ${DOCKER_MEAN}ms | P50 ${DOCKER_P50}ms | P99 ${DOCKER_P99}ms | failed: ${DOCKER_FAILED}"
else
    DOCKER_RPS="N/A"; DOCKER_MEAN="N/A"; DOCKER_P50="N/A"; DOCKER_P99="N/A"
fi
echo ""

if $CORTEN_OK; then
    bold "[5] Benchmark Corten: $AB_REQUESTS requests"
    echo ""
    CORTEN_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT/ 2>&1)
    CORTEN_RPS=$(echo "$CORTEN_AB" | grep "Requests per second" | awk '{print $4}')
    CORTEN_MEAN=$(echo "$CORTEN_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    CORTEN_P50=$(echo "$CORTEN_AB" | grep "50%" | awk '{print $2}')
    CORTEN_P99=$(echo "$CORTEN_AB" | grep "99%" | awk '{print $2}')
    CORTEN_FAILED=$(echo "$CORTEN_AB" | grep "Failed requests" | awk '{print $3}')
    echo "  Corten: ${CORTEN_RPS} req/s | mean ${CORTEN_MEAN}ms | P50 ${CORTEN_P50}ms | P99 ${CORTEN_P99}ms | failed: ${CORTEN_FAILED}"
else
    CORTEN_RPS="N/A"; CORTEN_MEAN="N/A"; CORTEN_P50="N/A"; CORTEN_P99="N/A"
fi
echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary — Node.js HTTP"
bold "========================================================"
echo ""
printf "  %-25s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-25s %15s %15s\n" "-------------------------" "---------------" "---------------"
printf "  %-25s %13sms %13sms\n" "Build time" "$DOCKER_BUILD_MS" "$CORTEN_BUILD_MS"
printf "  %-25s %13sms %13sms\n" "Container start" "$DOCKER_START_MS" "$CORTEN_START_MS"
printf "  %-25s %12s KB %12s KB\n" "Node RSS memory" "$DOCKER_RSS" "$CORTEN_RSS"
printf "  %-25s %12s/s %12s/s\n" "Requests/sec" "$DOCKER_RPS" "$CORTEN_RPS"
printf "  %-25s %13sms %13sms\n" "Latency (mean)" "$DOCKER_MEAN" "$CORTEN_MEAN"
printf "  %-25s %13sms %13sms\n" "Latency (P50)" "$DOCKER_P50" "$CORTEN_P50"
printf "  %-25s %13sms %13sms\n" "Latency (P99)" "$DOCKER_P99" "$CORTEN_P99"
echo ""
