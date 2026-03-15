#!/bin/bash
# Corten vs Docker — PHP + Nginx Stack Benchmark
#
# Two-container stack: Nginx → PHP-FPM (FastCGI)
# Tests real-world web app performance.
#
# Prerequisites: make install, docker, ab
# Usage: ./scripts/php-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

DOCKER_PORT=9080
CORTEN_PORT=9081
AB_REQUESTS=5000
AB_CONCURRENCY=50

cleanup() {
    bold "[cleanup]"
    docker rm -f php bench-docker-nginx 2>/dev/null || true
    docker network rm bench-docker-net 2>/dev/null || true
    $CORTEN stop php bench-corten-nginx 2>/dev/null || true
    $CORTEN rm php bench-corten-nginx 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — PHP + Nginx Stack Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  Requests: $AB_REQUESTS @ $AB_CONCURRENCY concurrent"
echo ""

# ============================================================================
bold "[1] Build images"
echo ""

# Shared PHP app code
PHP_APP='<?php echo "<h1>Hello from " . ($_SERVER["SERVER_SOFTWARE"] ?? "PHP") . "</h1>"; echo "<p>PHP " . phpversion() . "</p>"; echo "<p>Time: " . date("Y-m-d H:i:s") . "</p>"; echo "<p>PID: " . getmypid() . "</p>";'

# --- Docker ---
DOCKER_DIR=$(mktemp -d)

# PHP-FPM Dockerfile
cat > "$DOCKER_DIR/Dockerfile.php" <<DOCKERFILE
FROM alpine:3.20
RUN apk add --no-cache php83 php83-fpm php83-json php83-mbstring && \
    mkdir -p /var/www/html /run/php && \
    sed -i 's|listen = 127.0.0.1:9000|listen = 0.0.0.0:9000|' /etc/php83/php-fpm.d/www.conf
RUN echo '${PHP_APP}' > /var/www/html/index.php
CMD ["/usr/sbin/php-fpm83", "--nodaemonize"]
DOCKERFILE

# Nginx Dockerfile
cat > "$DOCKER_DIR/Dockerfile.nginx" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache nginx && mkdir -p /run/nginx /var/www/html
RUN printf 'server {\n  listen 8080;\n  root /var/www/html;\n  index index.php;\n  location ~ \\.php$ {\n    fastcgi_pass php:9000;\n    fastcgi_param SCRIPT_FILENAME /var/www/html$fastcgi_script_name;\n    include fastcgi_params;\n  }\n}\n' > /etc/nginx/http.d/default.conf
RUN echo '<h1>Static OK</h1>' > /var/www/html/index.html
CMD ["nginx", "-g", "daemon off;"]
DOCKERFILE

echo "  Building Docker PHP-FPM..."
START=$(date +%s%N)
docker build -t bench-php -f "$DOCKER_DIR/Dockerfile.php" "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_PHP_BUILD=$(( (END - START) / 1000000 ))

echo "  Building Docker Nginx..."
START=$(date +%s%N)
docker build -t bench-nginx-proxy -f "$DOCKER_DIR/Dockerfile.nginx" "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_NGINX_BUILD=$(( (END - START) / 1000000 ))
echo "  Docker total: $((DOCKER_PHP_BUILD + DOCKER_NGINX_BUILD))ms"
rm -rf "$DOCKER_DIR"

# --- Corten ---
echo "  Building Corten PHP-FPM..."
CORTEN_PHP_DIR=$(mktemp -d)
cat > "$CORTEN_PHP_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-php"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["php83", "php83-fpm", "php83-json", "php83-mbstring"]
[setup]
run = [
    "mkdir -p /var/www/html /run/php",
    "sed -i 's|listen = 127.0.0.1:9000|listen = 0.0.0.0:9000|' /etc/php83/php-fpm.d/www.conf",
    "printf '<?php\\necho \"<h1>Hello from Corten!</h1>\";\\necho \"<p>PHP \" . phpversion() . \"</p>\";\\necho \"<p>Time: \" . date(\"H:i:s\") . \"</p>\";\\n' > /var/www/html/index.php",
]
[container]
command = ["/usr/sbin/php-fpm83", "--nodaemonize"]
TOML

START=$(date +%s%N)
$CORTEN build "$CORTEN_PHP_DIR" >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_PHP_BUILD=$(( (END - START) / 1000000 ))
rm -rf "$CORTEN_PHP_DIR"

echo "  Building Corten Nginx..."
CORTEN_NGINX_DIR=$(mktemp -d)
cat > "$CORTEN_NGINX_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-nginx-proxy"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["nginx"]
[setup]
run = [
    "mkdir -p /run/nginx /var/www/html",
    "printf 'server {\\n  listen 8080;\\n  root /var/www/html;\\n  index index.php;\\n  location ~ \\.php$ {\\n    fastcgi_pass php:9000;\\n    fastcgi_param SCRIPT_FILENAME /var/www/html$fastcgi_script_name;\\n    include fastcgi_params;\\n  }\\n}\\n' > /etc/nginx/http.d/default.conf",
    "echo '<h1>Static OK</h1>' > /var/www/html/index.html",
]
[container]
command = ["/bin/sh", "-c", "mkdir -p /run/nginx && exec nginx -g 'daemon off;'"]
TOML

START=$(date +%s%N)
$CORTEN build "$CORTEN_NGINX_DIR" >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_NGINX_BUILD=$(( (END - START) / 1000000 ))
echo "  Corten total: $((CORTEN_PHP_BUILD + CORTEN_NGINX_BUILD))ms"
rm -rf "$CORTEN_NGINX_DIR"

echo ""

# ============================================================================
bold "[2] Start stacks (2 containers each)"
echo ""

# --- Docker: create network + start containers ---
START=$(date +%s%N)
docker network create bench-docker-net >/dev/null 2>&1
docker run -d --name php --network-alias php --network bench-docker-net bench-php >/dev/null 2>&1
docker run -d --name bench-docker-nginx --network bench-docker-net -p $DOCKER_PORT:8080 bench-nginx-proxy >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_STACK_MS=$(( (END - START) / 1000000 ))
echo "  Docker stack start: ${DOCKER_STACK_MS}ms"

# --- Corten: create network + start containers ---
START=$(date +%s%N)
$CORTEN network create bench-net >/dev/null 2>&1 || true
$CORTEN run -d --name php --network bench-net bench-php >/dev/null 2>&1
$CORTEN run -d --name bench-corten-nginx --network bench-net -p $CORTEN_PORT:8080 bench-nginx-proxy >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_STACK_MS=$(( (END - START) / 1000000 ))
echo "  Corten stack start: ${CORTEN_STACK_MS}ms"

# Wait for ready
echo "  Waiting for stacks..."
DOCKER_OK=false
CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$DOCKER_PORT/index.php 2>/dev/null || echo "000")
        [ "$CODE" = "200" ] && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$CORTEN_PORT/index.php 2>/dev/null || echo "000")
        [ "$CODE" = "200" ] && CORTEN_OK=true
    fi
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

echo -n "  Docker: "
if $DOCKER_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo -n "  Corten: "
if $CORTEN_OK; then green "READY"; else red "NOT RESPONDING"; fi

if ! $DOCKER_OK && ! $CORTEN_OK; then
    red "  Both stacks failed. Aborting."
    exit 1
fi

echo ""

# ============================================================================
bold "[3] Memory usage (total stack)"
echo ""

# Docker
DOCKER_PHP_PID=$(docker inspect --format '{{.State.Pid}}' php 2>/dev/null || echo "0")
DOCKER_NGINX_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-nginx 2>/dev/null || echo "0")
DOCKER_PHP_RSS=0; DOCKER_NGINX_RSS=0
[ -f "/proc/$DOCKER_PHP_PID/status" ] && DOCKER_PHP_RSS=$(grep VmRSS "/proc/$DOCKER_PHP_PID/status" | awk '{print $2}')
[ -f "/proc/$DOCKER_NGINX_PID/status" ] && DOCKER_NGINX_RSS=$(grep VmRSS "/proc/$DOCKER_NGINX_PID/status" | awk '{print $2}')
DOCKER_TOTAL_RSS=$((DOCKER_PHP_RSS + DOCKER_NGINX_RSS))
echo "  Docker: php-fpm=${DOCKER_PHP_RSS}KB + nginx=${DOCKER_NGINX_RSS}KB = ${DOCKER_TOTAL_RSS}KB"

# Corten
CORTEN_PHP_STATE=$($CORTEN inspect php 2>/dev/null)
CORTEN_NGINX_STATE=$($CORTEN inspect bench-corten-nginx 2>/dev/null)
CORTEN_PHP_PID=$(echo "$CORTEN_PHP_STATE" | grep "^PID:" | awk '{print $2}')
CORTEN_NGINX_PID=$(echo "$CORTEN_NGINX_STATE" | grep "^PID:" | awk '{print $2}')
CORTEN_PHP_RSS=0; CORTEN_NGINX_RSS=0
[ -n "$CORTEN_PHP_PID" ] && [ -f "/proc/$CORTEN_PHP_PID/status" ] && CORTEN_PHP_RSS=$(grep VmRSS "/proc/$CORTEN_PHP_PID/status" | awk '{print $2}')
[ -n "$CORTEN_NGINX_PID" ] && [ -f "/proc/$CORTEN_NGINX_PID/status" ] && CORTEN_NGINX_RSS=$(grep VmRSS "/proc/$CORTEN_NGINX_PID/status" | awk '{print $2}')
CORTEN_TOTAL_RSS=$((CORTEN_PHP_RSS + CORTEN_NGINX_RSS))
echo "  Corten: php-fpm=${CORTEN_PHP_RSS}KB + nginx=${CORTEN_NGINX_RSS}KB = ${CORTEN_TOTAL_RSS}KB"

DAEMON_RSS=0
DOCKERD_PID=$(pgrep -x dockerd 2>/dev/null || echo "")
CONTAINERD_PID=$(pgrep -x containerd 2>/dev/null | head -1 || echo "")
[ -n "$DOCKERD_PID" ] && [ -f "/proc/$DOCKERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$DOCKERD_PID/status" | awk '{print $2}')))
[ -n "$CONTAINERD_PID" ] && [ -f "/proc/$CONTAINERD_PID/status" ] && DAEMON_RSS=$((DAEMON_RSS + $(grep VmRSS "/proc/$CONTAINERD_PID/status" | awk '{print $2}')))
echo "  Docker daemon overhead: $((DAEMON_RSS / 1024)) MB"
echo "  Corten daemon overhead: 0 MB"

echo ""

# ============================================================================
if $DOCKER_OK; then
    bold "[4] Benchmark Docker: PHP page — $AB_REQUESTS requests @ $AB_CONCURRENCY concurrent"
    echo ""
    DOCKER_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT/index.php 2>&1)
    DOCKER_RPS=$(echo "$DOCKER_AB" | grep "Requests per second" | awk '{print $4}')
    DOCKER_MEAN=$(echo "$DOCKER_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    DOCKER_P50=$(echo "$DOCKER_AB" | grep "50%" | awk '{print $2}')
    DOCKER_P99=$(echo "$DOCKER_AB" | grep "99%" | awk '{print $2}')
    DOCKER_FAILED=$(echo "$DOCKER_AB" | grep "Failed requests" | awk '{print $3}')
    echo "  Docker:  ${DOCKER_RPS} req/s | mean ${DOCKER_MEAN}ms | P50 ${DOCKER_P50}ms | P99 ${DOCKER_P99}ms | failed: ${DOCKER_FAILED}"
else
    DOCKER_RPS="N/A"; DOCKER_MEAN="N/A"; DOCKER_P50="N/A"; DOCKER_P99="N/A"; DOCKER_FAILED="N/A"
    echo "  Docker: SKIPPED (not responding)"
fi

echo ""

if $CORTEN_OK; then
    bold "[5] Benchmark Corten: PHP page — $AB_REQUESTS requests @ $AB_CONCURRENCY concurrent"
    echo ""
    CORTEN_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT/index.php 2>&1)
    CORTEN_RPS=$(echo "$CORTEN_AB" | grep "Requests per second" | awk '{print $4}')
    CORTEN_MEAN=$(echo "$CORTEN_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    CORTEN_P50=$(echo "$CORTEN_AB" | grep "50%" | awk '{print $2}')
    CORTEN_P99=$(echo "$CORTEN_AB" | grep "99%" | awk '{print $2}')
    CORTEN_FAILED=$(echo "$CORTEN_AB" | grep "Failed requests" | awk '{print $3}')
    echo "  Corten: ${CORTEN_RPS} req/s | mean ${CORTEN_MEAN}ms | P50 ${CORTEN_P50}ms | P99 ${CORTEN_P99}ms | failed: ${CORTEN_FAILED}"
else
    CORTEN_RPS="N/A"; CORTEN_MEAN="N/A"; CORTEN_P50="N/A"; CORTEN_P99="N/A"; CORTEN_FAILED="N/A"
    echo "  Corten: SKIPPED (not responding)"
fi

echo ""

# ============================================================================
bold "========================================================"
bold "  Results Summary — PHP + Nginx Stack"
bold "========================================================"
echo ""
printf "  %-30s %15s %15s\n" "Metric" "Docker" "Corten"
printf "  %-30s %15s %15s\n" "------------------------------" "---------------" "---------------"
printf "  %-30s %13sms %13sms\n" "Stack start (2 containers)" "$DOCKER_STACK_MS" "$CORTEN_STACK_MS"
printf "  %-30s %12s KB %12s KB\n" "Stack RSS (php+nginx)" "$DOCKER_TOTAL_RSS" "$CORTEN_TOTAL_RSS"
printf "  %-30s %12s/s %12s/s\n" "PHP Requests/sec" "$DOCKER_RPS" "$CORTEN_RPS"
printf "  %-30s %13sms %13sms\n" "Latency (mean)" "$DOCKER_MEAN" "$CORTEN_MEAN"
printf "  %-30s %13sms %13sms\n" "Latency (P50)" "$DOCKER_P50" "$CORTEN_P50"
printf "  %-30s %13sms %13sms\n" "Latency (P99)" "$DOCKER_P99" "$CORTEN_P99"
echo ""
