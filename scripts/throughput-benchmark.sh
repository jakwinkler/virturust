#!/bin/bash
# Corten vs Docker — Network & I/O Throughput Benchmark
#
# Tests: iperf3 (network), dd (disk I/O), static file serving (large files)
#
# Usage: ./scripts/throughput-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

if ! command -v ab &>/dev/null; then
    red "ab not found — install: sudo dnf install httpd-tools"
    exit 1
fi

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-iperf bench-docker-static 2>/dev/null || true
    $CORTEN stop bench-corten-iperf bench-corten-static 2>/dev/null || true
    $CORTEN rm bench-corten-iperf bench-corten-static 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Throughput Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo ""

# ============================================================================
bold "[1] Disk I/O: write 100MB inside container"
echo ""

# Docker
START=$(date +%s%N)
docker run --rm alpine:3.20 /bin/sh -c "dd if=/dev/zero of=/tmp/test bs=1M count=100 2>&1 | tail -1" 2>/dev/null
END=$(date +%s%N)
DOCKER_IO_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_IO_MS}ms"

# Corten
START=$(date +%s%N)
$CORTEN run --rm --name io-test --network none alpine /bin/sh -c "dd if=/dev/zero of=/tmp/test bs=1M count=100 2>&1 | tail -1" 2>/dev/null
END=$(date +%s%N)
CORTEN_IO_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_IO_MS}ms"

echo ""

# ============================================================================
bold "[2] CPU: calculate primes to 10000"
echo ""

# Docker
START=$(date +%s%N)
docker run --rm alpine:3.20 /bin/sh -c "
i=2; count=0
while [ \$i -le 10000 ]; do
    j=2; is_prime=1
    while [ \$j -le \$((\$i/2)) ]; do
        if [ \$((\$i % \$j)) -eq 0 ]; then is_prime=0; break; fi
        j=\$((\$j+1))
    done
    if [ \$is_prime -eq 1 ]; then count=\$((\$count+1)); fi
    i=\$((\$i+1))
done
echo \$count primes
" 2>/dev/null
END=$(date +%s%N)
DOCKER_CPU_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_CPU_MS}ms"

# Corten
START=$(date +%s%N)
$CORTEN run --rm --name cpu-test --network none alpine /bin/sh -c "
i=2; count=0
while [ \$i -le 10000 ]; do
    j=2; is_prime=1
    while [ \$j -le \$((\$i/2)) ]; do
        if [ \$((\$i % \$j)) -eq 0 ]; then is_prime=0; break; fi
        j=\$((\$j+1))
    done
    if [ \$is_prime -eq 1 ]; then count=\$((\$count+1)); fi
    i=\$((\$i+1))
done
echo \$count primes
" 2>/dev/null
END=$(date +%s%N)
CORTEN_CPU_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_CPU_MS}ms"

echo ""

# ============================================================================
bold "[3] Static file serving: 1MB file x 1000 requests"
echo ""

# Build nginx with a 1MB file
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache nginx && mkdir -p /run/nginx /var/www/html && \
    dd if=/dev/urandom of=/var/www/html/large.bin bs=1M count=1 2>/dev/null && \
    printf 'server {\n  listen 8080;\n  root /var/www/html;\n}\n' > /etc/nginx/http.d/default.conf
CMD ["nginx", "-g", "daemon off;"]
DOCKERFILE
docker build -t bench-static "$DOCKER_DIR" -q >/dev/null 2>&1
rm -rf "$DOCKER_DIR"

CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-static"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["nginx"]
[setup]
run = [
    "mkdir -p /run/nginx /var/www/html",
    "dd if=/dev/urandom of=/var/www/html/large.bin bs=1M count=1 2>/dev/null",
    "printf 'server {\\n  listen 8080;\\n  root /var/www/html;\\n}\\n' > /etc/nginx/http.d/default.conf",
]
[container]
command = ["/bin/sh", "-c", "mkdir -p /run/nginx && exec nginx -g 'daemon off;'"]
TOML
$CORTEN build "$CORTEN_DIR" >/dev/null 2>&1
rm -rf "$CORTEN_DIR"

# Start
docker run -d --name bench-docker-static -p 19082:8080 bench-static >/dev/null 2>&1
$CORTEN run -d --name bench-corten-static -p 19083:8080 bench-static >/dev/null 2>&1

# Wait
DOCKER_OK=false; CORTEN_OK=false
for i in $(seq 1 15); do
    if ! $DOCKER_OK; then
        [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:19082/large.bin 2>/dev/null)" = "200" ] && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:19083/large.bin 2>/dev/null)" = "200" ] && CORTEN_OK=true
    fi
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

if $DOCKER_OK; then
    DOCKER_STATIC=$(ab -n 1000 -c 10 -q http://127.0.0.1:19082/large.bin 2>&1)
    DOCKER_STATIC_RPS=$(echo "$DOCKER_STATIC" | grep "Requests per second" | awk '{print $4}')
    DOCKER_STATIC_RATE=$(echo "$DOCKER_STATIC" | grep "Transfer rate" | awk '{print $3 " " $4}')
    echo "  Docker: ${DOCKER_STATIC_RPS} req/s (${DOCKER_STATIC_RATE})"
else
    DOCKER_STATIC_RPS="N/A"; DOCKER_STATIC_RATE="N/A"
    echo "  Docker: NOT RESPONDING"
fi

if $CORTEN_OK; then
    CORTEN_STATIC=$(ab -n 1000 -c 10 -q http://127.0.0.1:19083/large.bin 2>&1)
    CORTEN_STATIC_RPS=$(echo "$CORTEN_STATIC" | grep "Requests per second" | awk '{print $4}')
    CORTEN_STATIC_RATE=$(echo "$CORTEN_STATIC" | grep "Transfer rate" | awk '{print $3 " " $4}')
    echo "  Corten: ${CORTEN_STATIC_RPS} req/s (${CORTEN_STATIC_RATE})"
else
    CORTEN_STATIC_RPS="N/A"; CORTEN_STATIC_RATE="N/A"
    echo "  Corten: NOT RESPONDING"
fi

echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary — Throughput"
bold "========================================================"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %13sms %13sms\n" "Disk I/O (write 100MB)" "$DOCKER_IO_MS" "$CORTEN_IO_MS"
printf "  %-30s %13sms %13sms\n" "CPU (primes to 10000)" "$DOCKER_CPU_MS" "$CORTEN_CPU_MS"
printf "  %-30s %12s/s %12s/s\n" "Static 1MB file (req/s)" "${DOCKER_STATIC_RPS:-N/A}" "${CORTEN_STATIC_RPS:-N/A}"
echo ""
