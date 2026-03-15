#!/bin/bash
# Corten vs Docker — MySQL (MariaDB) Benchmark
#
# Builds MariaDB in both runtimes, runs them side by side,
# and compares startup time, memory usage, and query throughput.
#
# Prerequisites: make install, docker, mysql client (mariadb)
# Usage: ./scripts/mysql-benchmark.sh

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

if ! command -v mysql &>/dev/null && ! command -v mariadb &>/dev/null; then
    red "MySQL/MariaDB client not found. Install: sudo dnf install mariadb"
    exit 1
fi

MYSQL_CMD=$(command -v mariadb || command -v mysql)
DOCKER_PORT=13306
CORTEN_PORT=13307
ROOT_PASS="benchpass123"
QUERIES=1000

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-mysql 2>/dev/null || true
    $CORTEN stop bench-corten-mysql 2>/dev/null || true
    $CORTEN rm bench-corten-mysql 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — MariaDB Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo "  MySQL:  $($MYSQL_CMD --version 2>&1 | head -1)"
echo "  Queries: $QUERIES"
echo ""

# ============================================================================
bold "[1] Build MariaDB images"
echo ""

# --- Docker ---
DOCKER_DIR=$(mktemp -d)
cat > "$DOCKER_DIR/Dockerfile" <<DOCKERFILE
FROM alpine:3.20
RUN apk add --no-cache mariadb mariadb-client && \
    mkdir -p /run/mysqld /var/lib/mysql && \
    chown -R mysql:mysql /run/mysqld /var/lib/mysql && \
    mysql_install_db --user=mysql --datadir=/var/lib/mysql && \
    mysqld --user=mysql --datadir=/var/lib/mysql --bootstrap <<'SQL'
USE mysql;
FLUSH PRIVILEGES;
ALTER USER 'root'@'localhost' IDENTIFIED BY '${ROOT_PASS}';
CREATE USER 'root'@'%' IDENTIFIED BY '${ROOT_PASS}';
GRANT ALL PRIVILEGES ON *.* TO 'root'@'%' WITH GRANT OPTION;
FLUSH PRIVILEGES;
SQL
EXPOSE 3306
CMD ["mysqld", "--user=mysql", "--datadir=/var/lib/mysql", "--bind-address=0.0.0.0", "--skip-networking=0"]
DOCKERFILE

echo "  Building Docker MariaDB..."
START=$(date +%s%N)
docker build -t bench-mysql "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_BUILD_MS=$(( (END - START) / 1000000 ))
echo "  Docker build: ${DOCKER_BUILD_MS}ms"
rm -rf "$DOCKER_DIR"

# --- Corten ---
echo "  Building Corten MariaDB..."

CORTEN_DIR=$(mktemp -d)
cat > "$CORTEN_DIR/Corten.toml" <<TOML
[image]
name = "bench-mysql"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["mariadb", "mariadb-client"]

[setup]
run = [
    "mkdir -p /run/mysqld /var/lib/mysql",
    "chown -R mysql:mysql /run/mysqld /var/lib/mysql",
    "mysql_install_db --user=mysql --datadir=/var/lib/mysql",
    "mysqld --user=mysql --datadir=/var/lib/mysql --bootstrap <<'SQL'\nUSE mysql;\nFLUSH PRIVILEGES;\nALTER USER 'root'@'localhost' IDENTIFIED BY '${ROOT_PASS}';\nCREATE USER 'root'@'%' IDENTIFIED BY '${ROOT_PASS}';\nGRANT ALL PRIVILEGES ON *.* TO 'root'@'%' WITH GRANT OPTION;\nFLUSH PRIVILEGES;\nSQL",
]

[container]
command = ["/usr/bin/mysqld", "--user=mysql", "--datadir=/var/lib/mysql", "--bind-address=0.0.0.0", "--skip-networking=0"]
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
docker run -d --name bench-docker-mysql -p $DOCKER_PORT:3306 bench-mysql >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_START_MS=$(( (END - START) / 1000000 ))
echo "  Docker start: ${DOCKER_START_MS}ms (port $DOCKER_PORT)"

# --- Corten ---
START=$(date +%s%N)
$CORTEN run -d --name bench-corten-mysql -p $CORTEN_PORT:3306 bench-mysql >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_START_MS=$(( (END - START) / 1000000 ))
echo "  Corten start: ${CORTEN_START_MS}ms (port $CORTEN_PORT)"

# Wait for MySQL to be ready
echo "  Waiting for MariaDB to accept connections..."
DOCKER_OK=false
CORTEN_OK=false
for i in $(seq 1 30); do
    if ! $DOCKER_OK; then
        $MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS -e "SELECT 1" >/dev/null 2>&1 && DOCKER_OK=true
    fi
    if ! $CORTEN_OK; then
        $MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS -e "SELECT 1" >/dev/null 2>&1 && CORTEN_OK=true
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
    $CORTEN logs bench-corten-mysql 2>/dev/null | tail -3 || echo "  (no logs)"
fi

if ! $DOCKER_OK || ! $CORTEN_OK; then
    red "  One or both databases failed to start. Aborting."
    exit 1
fi

echo ""

# ============================================================================
bold "[3] Memory usage"
echo ""

DOCKER_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-mysql 2>/dev/null || echo "0")
if [ "$DOCKER_PID" != "0" ] && [ -f "/proc/$DOCKER_PID/status" ]; then
    DOCKER_RSS=$(grep VmRSS "/proc/$DOCKER_PID/status" | awk '{print $2}')
    echo "  Docker mysqld RSS:   ${DOCKER_RSS} KB"
else
    DOCKER_RSS="?"
    echo "  Docker mysqld RSS:   (could not read)"
fi

CORTEN_STATE=$($CORTEN inspect bench-corten-mysql 2>/dev/null || echo "")
CORTEN_PID=$(echo "$CORTEN_STATE" | grep "^PID:" | awk '{print $2}')
if [ -n "$CORTEN_PID" ] && [ "$CORTEN_PID" != "-" ] && [ -f "/proc/$CORTEN_PID/status" ]; then
    CORTEN_RSS=$(grep VmRSS "/proc/$CORTEN_PID/status" | awk '{print $2}')
    echo "  Corten mysqld RSS:   ${CORTEN_RSS} KB"
else
    CORTEN_RSS="?"
    echo "  Corten mysqld RSS:   (could not read)"
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
bold "[4] Create test database and table"
echo ""

# Setup on Docker
$MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS <<'SQL'
CREATE DATABASE IF NOT EXISTS benchmark;
USE benchmark;
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(200) NOT NULL,
    score INT DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_email (email),
    INDEX idx_score (score)
) ENGINE=InnoDB;
SQL
echo "  Docker: benchmark database created"

# Setup on Corten
$MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS <<'SQL'
CREATE DATABASE IF NOT EXISTS benchmark;
USE benchmark;
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(200) NOT NULL,
    score INT DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_email (email),
    INDEX idx_score (score)
) ENGINE=InnoDB;
SQL
echo "  Corten: benchmark database created"

echo ""

# ============================================================================
bold "[5] Benchmark: INSERT $QUERIES rows"
echo ""

# Docker INSERT
START=$(date +%s%N)
for i in $(seq 1 $QUERIES); do
    echo "INSERT INTO users (name, email, score) VALUES ('user_$i', 'user_$i@test.com', $((RANDOM % 1000)));"
done | $MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS benchmark 2>/dev/null
END=$(date +%s%N)
DOCKER_INSERT_MS=$(( (END - START) / 1000000 ))
DOCKER_INSERT_RPS=$(echo "scale=0; $QUERIES * 1000 / $DOCKER_INSERT_MS" | bc)
echo "  Docker: ${DOCKER_INSERT_MS}ms (${DOCKER_INSERT_RPS} inserts/sec)"

# Corten INSERT
START=$(date +%s%N)
for i in $(seq 1 $QUERIES); do
    echo "INSERT INTO users (name, email, score) VALUES ('user_$i', 'user_$i@test.com', $((RANDOM % 1000)));"
done | $MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS benchmark 2>/dev/null
END=$(date +%s%N)
CORTEN_INSERT_MS=$(( (END - START) / 1000000 ))
CORTEN_INSERT_RPS=$(echo "scale=0; $QUERIES * 1000 / $CORTEN_INSERT_MS" | bc)
echo "  Corten: ${CORTEN_INSERT_MS}ms (${CORTEN_INSERT_RPS} inserts/sec)"

echo ""

# ============================================================================
bold "[6] Benchmark: SELECT $QUERIES queries (point lookups)"
echo ""

# Docker SELECT
START=$(date +%s%N)
for i in $(seq 1 $QUERIES); do
    echo "SELECT * FROM users WHERE id = $((RANDOM % QUERIES + 1));"
done | $MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS benchmark >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_SELECT_MS=$(( (END - START) / 1000000 ))
DOCKER_SELECT_RPS=$(echo "scale=0; $QUERIES * 1000 / $DOCKER_SELECT_MS" | bc)
echo "  Docker: ${DOCKER_SELECT_MS}ms (${DOCKER_SELECT_RPS} queries/sec)"

# Corten SELECT
START=$(date +%s%N)
for i in $(seq 1 $QUERIES); do
    echo "SELECT * FROM users WHERE id = $((RANDOM % QUERIES + 1));"
done | $MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS benchmark >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_SELECT_MS=$(( (END - START) / 1000000 ))
CORTEN_SELECT_RPS=$(echo "scale=0; $QUERIES * 1000 / $CORTEN_SELECT_MS" | bc)
echo "  Corten: ${CORTEN_SELECT_MS}ms (${CORTEN_SELECT_RPS} queries/sec)"

echo ""

# ============================================================================
bold "[7] Benchmark: Complex queries (JOIN + aggregate)"
echo ""

# Insert some related data first
$MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS benchmark <<'SQL' 2>/dev/null
CREATE TABLE IF NOT EXISTS orders (
    id INT AUTO_INCREMENT PRIMARY KEY,
    user_id INT NOT NULL,
    amount DECIMAL(10,2) NOT NULL,
    INDEX idx_user_id (user_id)
) ENGINE=InnoDB;
INSERT INTO orders (user_id, amount) SELECT id, RAND() * 1000 FROM users;
SQL

$MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS benchmark <<'SQL' 2>/dev/null
CREATE TABLE IF NOT EXISTS orders (
    id INT AUTO_INCREMENT PRIMARY KEY,
    user_id INT NOT NULL,
    amount DECIMAL(10,2) NOT NULL,
    INDEX idx_user_id (user_id)
) ENGINE=InnoDB;
INSERT INTO orders (user_id, amount) SELECT id, RAND() * 1000 FROM users;
SQL

COMPLEX_QUERIES=100

# Docker complex
START=$(date +%s%N)
for i in $(seq 1 $COMPLEX_QUERIES); do
    echo "SELECT u.name, COUNT(o.id) as order_count, SUM(o.amount) as total FROM users u JOIN orders o ON u.id = o.user_id WHERE u.score > $((RANDOM % 500)) GROUP BY u.id ORDER BY total DESC LIMIT 10;"
done | $MYSQL_CMD -h 127.0.0.1 -P $DOCKER_PORT -u root -p$ROOT_PASS benchmark >/dev/null 2>&1
END=$(date +%s%N)
DOCKER_COMPLEX_MS=$(( (END - START) / 1000000 ))
echo "  Docker: ${DOCKER_COMPLEX_MS}ms for $COMPLEX_QUERIES complex queries"

# Corten complex
START=$(date +%s%N)
for i in $(seq 1 $COMPLEX_QUERIES); do
    echo "SELECT u.name, COUNT(o.id) as order_count, SUM(o.amount) as total FROM users u JOIN orders o ON u.id = o.user_id WHERE u.score > $((RANDOM % 500)) GROUP BY u.id ORDER BY total DESC LIMIT 10;"
done | $MYSQL_CMD -h 127.0.0.1 -P $CORTEN_PORT -u root -p$ROOT_PASS benchmark >/dev/null 2>&1
END=$(date +%s%N)
CORTEN_COMPLEX_MS=$(( (END - START) / 1000000 ))
echo "  Corten: ${CORTEN_COMPLEX_MS}ms for $COMPLEX_QUERIES complex queries"

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
printf "  %-30s %12s KB %12s KB\n" "mysqld RSS memory" "$DOCKER_RSS" "$CORTEN_RSS"
printf "  %-30s %12s MB %13s\n" "Daemon overhead" "$DAEMON_MB" "0 MB"
printf "  %-30s %12s/s %12s/s\n" "INSERT throughput" "$DOCKER_INSERT_RPS" "$CORTEN_INSERT_RPS"
printf "  %-30s %12s/s %12s/s\n" "SELECT throughput" "$DOCKER_SELECT_RPS" "$CORTEN_SELECT_RPS"
printf "  %-30s %13sms %13sms\n" "Complex queries (${COMPLEX_QUERIES}x)" "$DOCKER_COMPLEX_MS" "$CORTEN_COMPLEX_MS"
echo ""
