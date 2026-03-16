#!/bin/bash
# Corten vs Docker — PostgreSQL Benchmark
#
# Builds PostgreSQL in both runtimes, runs them side by side,
# and compares startup time, memory usage, and transaction throughput.
#
# Prerequisites: make install, docker, pgbench (postgresql)
# Usage: ./scripts/postgres-benchmark.sh

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

if ! command -v pgbench &>/dev/null; then
    red "pgbench not found. Install: sudo dnf install postgresql"
    exit 1
fi

DOCKER_PORT=15432
CORTEN_PORT=15433
CONTAINER_PORT=5432
PGBENCH_CLIENTS=10
PGBENCH_JOBS=2
PGBENCH_TIME=10

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-postgres 2>/dev/null || true
    $CORTEN stop bench-corten-postgres 2>/dev/null || true
    $CORTEN rm bench-corten-postgres 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — PostgreSQL Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  pgbench: $(pgbench --version 2>&1 | head -1)"
echo "  Benchmark: ${PGBENCH_CLIENTS} clients, ${PGBENCH_JOBS} threads, ${PGBENCH_TIME}s"
echo ""

# ============================================================================
bold "[1] Build PostgreSQL images"
echo ""

# --- Docker ---
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache postgresql16 && \
    mkdir -p /run/postgresql /var/lib/postgresql/data && \
    chown -R postgres:postgres /run/postgresql /var/lib/postgresql && \
    su postgres -c 'initdb -D /var/lib/postgresql/data' && \
    su postgres -c 'pg_ctl start -D /var/lib/postgresql/data -l /tmp/pg.log -w' && \
    su postgres -c 'createuser -s root' && \
    su postgres -c 'createdb benchdb' && \
    su postgres -c 'pg_ctl stop -D /var/lib/postgresql/data -w' && \
    echo 'host all all 0.0.0.0/0 trust' >> /var/lib/postgresql/data/pg_hba.conf && \
    sed -i "s/#listen_addresses = 'localhost'/listen_addresses = '*'/" /var/lib/postgresql/data/postgresql.conf
CMD ["su", "postgres", "-c", "postgres -D /var/lib/postgresql/data -h 0.0.0.0"]
DOCKERFILE

echo "  Building Docker PostgreSQL..."
START=$(date +%s%N)
docker build -t bench-postgres "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Docker build: ${DOCKER_BUILD_MS}ms"
rm -rf "$DOCKER_DIR"

# --- Corten ---
echo "  Building Corten PostgreSQL..."
$CORTEN pull alpine >/dev/null 2>&1 || true

CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-postgres"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["postgresql16"]

[setup]
run = [
    "mkdir -p /run/postgresql /var/lib/postgresql/data",
    "chown -R postgres:postgres /run/postgresql /var/lib/postgresql",
    "su postgres -c 'initdb -D /var/lib/postgresql/data'",
    "su postgres -c 'pg_ctl start -D /var/lib/postgresql/data -l /tmp/pg.log -w'",
    "su postgres -c 'createuser -s root'",
    "su postgres -c 'createdb benchdb'",
    "su postgres -c 'pg_ctl stop -D /var/lib/postgresql/data -w'",
    "echo 'host all all 0.0.0.0/0 trust' >> /var/lib/postgresql/data/pg_hba.conf",
    "sed -i \"s/#listen_addresses = 'localhost'/listen_addresses = '*'/\" /var/lib/postgresql/data/postgresql.conf",
]

[container]
command = ["/bin/sh", "-c", "chown -R postgres:postgres /run/postgresql /var/lib/postgresql && exec su postgres -c 'postgres -D /var/lib/postgresql/data -h 0.0.0.0'"]
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
docker run -d --name bench-docker-postgres -p $DOCKER_PORT:$CONTAINER_PORT bench-postgres >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
echo "  Docker start: ${DOCKER_START_MS}ms (port $DOCKER_PORT)"

# --- Corten ---
START=$(date +%s%N)
$CORTEN run -d --name bench-corten-postgres -p $CORTEN_PORT:$CONTAINER_PORT bench-postgres >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
echo "  Corten start: ${CORTEN_START_MS}ms (port $CORTEN_PORT)"

# Wait for PostgreSQL to be ready
echo "  Waiting for PostgreSQL to accept connections..."
DOCKER_OK=false
CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        pgbench -h 127.0.0.1 -p $DOCKER_PORT -U postgres -i benchdb >/dev/null 2>&1 && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        pgbench -h 127.0.0.1 -p $CORTEN_PORT -U postgres -i benchdb >/dev/null 2>&1 && CORTEN_OK=true
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
    $CORTEN logs bench-corten-postgres 2>/dev/null | tail -3 || echo "  (no logs)"
fi

if ! $DOCKER_OK || ! $CORTEN_OK; then
    red "  One or both databases failed to start. Aborting."
    exit 1
fi

echo ""

# ============================================================================
bold "[3] Memory usage"
echo ""

DOCKER_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-postgres 2>/dev/null || echo "0")
if [ "$DOCKER_PID" != "0" ] && [ -f "/proc/$DOCKER_PID/status" ]; then
    DOCKER_RSS=$(grep VmRSS "/proc/$DOCKER_PID/status" | awk '{print $2}')
    echo "  Docker postgres RSS:   ${DOCKER_RSS} KB"
else
    DOCKER_RSS="?"
    echo "  Docker postgres RSS:   (could not read)"
fi

CORTEN_STATE=$($CORTEN inspect bench-corten-postgres 2>/dev/null || echo "")
CORTEN_PID=$(echo "$CORTEN_STATE" | grep "^PID:" | awk '{print $2}')
if [ -n "$CORTEN_PID" ] && [ "$CORTEN_PID" != "-" ] && [ -f "/proc/$CORTEN_PID/status" ]; then
    CORTEN_RSS=$(grep VmRSS "/proc/$CORTEN_PID/status" | awk '{print $2}')
    echo "  Corten postgres RSS:   ${CORTEN_RSS} KB"
else
    CORTEN_RSS="?"
    echo "  Corten postgres RSS:   (could not read)"
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
bold "[4] Benchmark: pgbench (${PGBENCH_CLIENTS} clients, ${PGBENCH_JOBS} threads, ${PGBENCH_TIME}s)"
echo ""

# --- Docker ---
echo "  Benchmarking Docker PostgreSQL..."
DOCKER_PGBENCH=$(pgbench -h 127.0.0.1 -p $DOCKER_PORT -U postgres -c $PGBENCH_CLIENTS -j $PGBENCH_JOBS -T $PGBENCH_TIME benchdb 2>&1)
DOCKER_TPS=$(echo "$DOCKER_PGBENCH" | grep "^tps" | head -1 | awk '{print $3}')
DOCKER_LATENCY=$(echo "$DOCKER_PGBENCH" | grep "latency average" | awk '{print $4}')
echo "    TPS:     $DOCKER_TPS"
echo "    Latency: ${DOCKER_LATENCY}ms"

echo ""

# --- Corten ---
echo "  Benchmarking Corten PostgreSQL..."
CORTEN_PGBENCH=$(pgbench -h 127.0.0.1 -p $CORTEN_PORT -U postgres -c $PGBENCH_CLIENTS -j $PGBENCH_JOBS -T $PGBENCH_TIME benchdb 2>&1)
CORTEN_TPS=$(echo "$CORTEN_PGBENCH" | grep "^tps" | head -1 | awk '{print $3}')
CORTEN_LATENCY=$(echo "$CORTEN_PGBENCH" | grep "latency average" | awk '{print $4}')
echo "    TPS:     $CORTEN_TPS"
echo "    Latency: ${CORTEN_LATENCY}ms"

echo ""

# ============================================================================
bold "[5] Image size"
echo ""

DOCKER_IMG_SIZE=$(docker image inspect bench-postgres --format '{{.Size}}' 2>/dev/null || echo "0")
DOCKER_IMG_MB=$((DOCKER_IMG_SIZE / 1024 / 1024))
echo "  Docker image:  ${DOCKER_IMG_MB} MB"

CORTEN_ROOTFS=$(du -sb /var/lib/corten/images/bench-postgres/latest/rootfs 2>/dev/null | cut -f1 || echo "0")
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
printf "  %-30s %12s KB %12s KB\n" "Postgres RSS memory" "$DOCKER_RSS" "$CORTEN_RSS"
printf "  %-30s %12s MB %13s\n" "Daemon overhead" "$DAEMON_MB" "0 MB"
printf "  %-30s %12s/s %12s/s\n" "Transactions/sec (TPS)" "$DOCKER_TPS" "$CORTEN_TPS"
printf "  %-30s %13sms %13sms\n" "Latency (avg)" "$DOCKER_LATENCY" "$CORTEN_LATENCY"
echo ""
