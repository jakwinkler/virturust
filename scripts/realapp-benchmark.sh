#!/bin/bash
# Corten vs Docker — Real Application Benchmark
#
# Tests real-world application patterns:
#   1. Python REST API with SQLite (CRUD operations)
#   2. PHP app processing requests with session/file I/O
#
# Usage: ./scripts/realapp-benchmark.sh

set -euo pipefail

CORTEN="${CORTEN:-$(command -v corten || echo ./target/release/corten)}"

red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

if ! command -v ab &>/dev/null; then
    red "ab not found — install: sudo dnf install httpd-tools"
    exit 1
fi

DOCKER_PORT_PY=28080
CORTEN_PORT_PY=28081
DOCKER_PORT_PHP=28082
CORTEN_PORT_PHP=28083
AB_REQUESTS=5000
AB_CONCURRENCY=50

cleanup() {
    bold "[cleanup]"
    docker rm -f bench-docker-pyapi bench-docker-phpapp 2>/dev/null || true
    $CORTEN stop bench-corten-pyapi bench-corten-phpapp 2>/dev/null || true
    $CORTEN rm bench-corten-pyapi bench-corten-phpapp 2>/dev/null || true
}
trap cleanup EXIT

bold ""
bold "========================================================"
bold "  Corten vs Docker — Real Application Benchmark"
bold "========================================================"
echo ""
echo "  Corten: $($CORTEN --version 2>&1)"
echo "  Docker: $(docker --version 2>&1)"
echo ""

# ============================================================================
# APP 1: Python REST API with SQLite
# ============================================================================
bold "============================================"
bold "  App 1: Python REST API + SQLite"
bold "============================================"
echo ""

# Write Python app to a temp file (avoids quoting hell in heredocs)
APP_PY=$(mktemp)
cat > "$APP_PY" <<'PYEOF'
import http.server, json, sqlite3, time, os

db = sqlite3.connect("/tmp/app.db")
db.execute("CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT, price REAL, created_at TEXT)")
db.execute("INSERT OR IGNORE INTO items VALUES (1, 'Widget', 9.99, datetime('now'))")
db.execute("INSERT OR IGNORE INTO items VALUES (2, 'Gadget', 24.99, datetime('now'))")
db.execute("INSERT OR IGNORE INTO items VALUES (3, 'Doohickey', 4.99, datetime('now'))")
db.commit()

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        if self.path == "/api/items":
            rows = db.execute("SELECT * FROM items ORDER BY id").fetchall()
            data = [{"id":r[0],"name":r[1],"price":r[2],"created_at":r[3]} for r in rows]
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"items": data, "count": len(data)}).encode())
        elif self.path.startswith("/api/items/"):
            try:
                item_id = int(self.path.split("/")[-1])
                row = db.execute("SELECT * FROM items WHERE id=?", (item_id,)).fetchone()
                if row:
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(json.dumps({"id":row[0],"name":row[1],"price":row[2]}).encode())
                else:
                    self.send_response(404); self.end_headers()
            except: self.send_response(400); self.end_headers()
        elif self.path == "/health":
            self.send_response(200); self.end_headers(); self.wfile.write(b"ok")
        else:
            self.send_response(404); self.end_headers()
    def do_POST(self):
        if self.path == "/api/items":
            length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(length)) if length else {}
            db.execute("INSERT INTO items (name, price, created_at) VALUES (?, ?, datetime('now'))",
                       (body.get("name","item"), body.get("price",0)))
            db.commit()
            self.send_response(201); self.send_header("Content-Type","application/json")
            self.end_headers(); self.wfile.write(b'{"status":"created"}')

http.server.HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PYEOF

# Docker
DOCKER_DIR=$(mktemp -d)
cp "$APP_PY" "$DOCKER_DIR/app.py"
cat > "$DOCKER_DIR/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache python3
COPY app.py /app.py
CMD ["python3", "/app.py"]
DOCKERFILE

echo "  Building Docker Python API..."
START=$(date +%s%N)
docker build -t bench-pyapi "$DOCKER_DIR" -q >/dev/null 2>&1
END=$(date +%s%N)
echo "  Docker build: $(( (END - START) / 1000000 ))ms"
rm -rf "$DOCKER_DIR"

# Corten
echo "  Building Corten Python API..."
CORTEN_DIR=$(mktemp -d)
cp "$APP_PY" "$CORTEN_DIR/app.py"
cat > "$CORTEN_DIR/Corten.toml" <<'TOML'
[image]
name = "bench-pyapi"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["python3"]
[files]
copy = [
    { src = "app.py", dest = "/app.py" },
]
[container]
command = ["/usr/bin/python3", "/app.py"]
TOML

START=$(date +%s%N)
$CORTEN build "$CORTEN_DIR" >/dev/null 2>&1
END=$(date +%s%N)
echo "  Corten build: $(( (END - START) / 1000000 ))ms"
rm -rf "$CORTEN_DIR" "$APP_PY"

# Start
docker run -d --name bench-docker-pyapi -p $DOCKER_PORT_PY:8080 bench-pyapi >/dev/null 2>&1
$CORTEN run -d --name bench-corten-pyapi -p $CORTEN_PORT_PY:8080 bench-pyapi >/dev/null 2>&1

# Wait
DOCKER_OK=false; CORTEN_OK=false
for i in $(seq 1 15); do
    $DOCKER_OK || { [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$DOCKER_PORT_PY/health 2>/dev/null)" = "200" ] && DOCKER_OK=true; }
    $CORTEN_OK || { [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$CORTEN_PORT_PY/health 2>/dev/null)" = "200" ] && CORTEN_OK=true; }
    $DOCKER_OK && $CORTEN_OK && break
    sleep 1
done

echo -n "  Docker: "; if $DOCKER_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo -n "  Corten: "; if $CORTEN_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo ""

# Verify API works
if $DOCKER_OK; then
    echo "  Docker API: $(curl -s http://127.0.0.1:$DOCKER_PORT_PY/api/items | head -c 80)..."
fi
if $CORTEN_OK; then
    echo "  Corten API: $(curl -s http://127.0.0.1:$CORTEN_PORT_PY/api/items | head -c 80)..."
fi
echo ""

# Benchmark: GET /api/items (list all — DB read)
bold "  Benchmark: GET /api/items ($AB_REQUESTS requests)"
echo ""

if $DOCKER_OK; then
    D_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT_PY/api/items 2>&1)
    D_RPS=$(echo "$D_AB" | grep "Requests per second" | awk '{print $4}')
    D_MEAN=$(echo "$D_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    D_P99=$(echo "$D_AB" | grep "99%" | awk '{print $2}')
    echo "  Docker: ${D_RPS} req/s | mean ${D_MEAN}ms | P99 ${D_P99}ms"
else
    D_RPS="N/A"; D_MEAN="N/A"; D_P99="N/A"
fi

if $CORTEN_OK; then
    C_AB=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT_PY/api/items 2>&1)
    C_RPS=$(echo "$C_AB" | grep "Requests per second" | awk '{print $4}')
    C_MEAN=$(echo "$C_AB" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    C_P99=$(echo "$C_AB" | grep "99%" | awk '{print $2}')
    echo "  Corten: ${C_RPS} req/s | mean ${C_MEAN}ms | P99 ${C_P99}ms"
else
    C_RPS="N/A"; C_MEAN="N/A"; C_P99="N/A"
fi
echo ""

# Benchmark: GET /api/items/1 (single item — point query)
bold "  Benchmark: GET /api/items/1 ($AB_REQUESTS requests)"
echo ""

if $DOCKER_OK; then
    D_AB2=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT_PY/api/items/1 2>&1)
    D_RPS2=$(echo "$D_AB2" | grep "Requests per second" | awk '{print $4}')
    echo "  Docker: ${D_RPS2} req/s"
else
    D_RPS2="N/A"
fi

if $CORTEN_OK; then
    C_AB2=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT_PY/api/items/1 2>&1)
    C_RPS2=$(echo "$C_AB2" | grep "Requests per second" | awk '{print $4}')
    echo "  Corten: ${C_RPS2} req/s"
else
    C_RPS2="N/A"
fi
echo ""

# Benchmark: POST /api/items (write — DB insert)
bold "  Benchmark: POST /api/items ($((AB_REQUESTS / 2)) write requests)"
echo ""

POSTDATA='{"name":"bench-item","price":12.34}'
if $DOCKER_OK; then
    D_AB3=$(ab -n $((AB_REQUESTS / 2)) -c $AB_CONCURRENCY -p /dev/stdin -T "application/json" -q http://127.0.0.1:$DOCKER_PORT_PY/api/items <<< "$POSTDATA" 2>&1)
    D_RPS3=$(echo "$D_AB3" | grep "Requests per second" | awk '{print $4}')
    echo "  Docker: ${D_RPS3} req/s (writes)"
else
    D_RPS3="N/A"
fi

if $CORTEN_OK; then
    C_AB3=$(ab -n $((AB_REQUESTS / 2)) -c $AB_CONCURRENCY -p /dev/stdin -T "application/json" -q http://127.0.0.1:$CORTEN_PORT_PY/api/items <<< "$POSTDATA" 2>&1)
    C_RPS3=$(echo "$C_AB3" | grep "Requests per second" | awk '{print $4}')
    echo "  Corten: ${C_RPS3} req/s (writes)"
else
    C_RPS3="N/A"
fi
echo ""

# Memory
DOCKER_PY_PID=$(docker inspect --format '{{.State.Pid}}' bench-docker-pyapi 2>/dev/null || echo "0")
DOCKER_PY_RSS="?"; [ -f "/proc/$DOCKER_PY_PID/status" ] && DOCKER_PY_RSS=$(grep VmRSS "/proc/$DOCKER_PY_PID/status" | awk '{print $2}')
CORTEN_PY_PID=$($CORTEN inspect bench-corten-pyapi 2>/dev/null | grep "^PID:" | awk '{print $2}')
CORTEN_PY_RSS="?"; [ -n "$CORTEN_PY_PID" ] && [ -f "/proc/$CORTEN_PY_PID/status" ] && CORTEN_PY_RSS=$(grep VmRSS "/proc/$CORTEN_PY_PID/status" | awk '{print $2}')

echo "  Memory: Docker=${DOCKER_PY_RSS}KB  Corten=${CORTEN_PY_RSS}KB"
echo ""

# ============================================================================
# APP 2: PHP Application (computation + file I/O)
# ============================================================================
bold "============================================"
bold "  App 2: PHP Application (compute + I/O)"
bold "============================================"
echo ""

# Docker
DOCKER_DIR2=$(mktemp -d)
cat > "$DOCKER_DIR2/Dockerfile" <<'DOCKERFILE'
FROM alpine:3.20
RUN apk add --no-cache php83 php83-fpm php83-json php83-session nginx && \
    mkdir -p /run/nginx /run/php /var/www/html && \
    sed -i 's|listen = 127.0.0.1:9000|listen = /run/php/php-fpm.sock|' /etc/php83/php-fpm.d/www.conf && \
    printf 'server {\n  listen 8080;\n  root /var/www/html;\n  index index.php;\n  location ~ \\.php$ {\n    fastcgi_pass unix:/run/php/php-fpm.sock;\n    fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n    include fastcgi_params;\n  }\n}\n' > /etc/nginx/http.d/default.conf
RUN printf '<?php\n$start = microtime(true);\n\n// CPU: fibonacci\nfunction fib($n) { return $n <= 1 ? $n : fib($n-1) + fib($n-2); }\n$fib_result = fib(20);\n\n// I/O: write + read temp file\n$tmp = tempnam("/tmp", "bench_");\nfile_put_contents($tmp, str_repeat("x", 1024));\n$data = file_get_contents($tmp);\nunlink($tmp);\n\n// String processing\n$hash = hash("sha256", str_repeat("benchmark", 100));\n\n$elapsed = (microtime(true) - $start) * 1000;\n\nheader("Content-Type: application/json");\necho json_encode([\n    "fibonacci_20" => $fib_result,\n    "hash" => substr($hash, 0, 16),\n    "file_bytes" => strlen($data),\n    "elapsed_ms" => round($elapsed, 3),\n    "pid" => getmypid(),\n    "php" => phpversion()\n]);\n' > /var/www/html/index.php
CMD ["/bin/sh", "-c", "php-fpm83 && nginx -g 'daemon off;'"]
DOCKERFILE

echo "  Building Docker PHP app..."
START=$(date +%s%N)
docker build -t bench-phpapp "$DOCKER_DIR2" -q >/dev/null 2>&1
END=$(date +%s%N)
echo "  Docker build: $(( (END - START) / 1000000 ))ms"
rm -rf "$DOCKER_DIR2"

# Corten
echo "  Building Corten PHP app..."
CORTEN_DIR2=$(mktemp -d)
cat > "$CORTEN_DIR2/Corten.toml" <<'TOML'
[image]
name = "bench-phpapp"
tag = "latest"
[base]
system = "alpine"
version = "3.20"
[packages]
install = ["php83", "php83-fpm", "php83-json", "php83-session", "nginx"]
[setup]
run = [
    "mkdir -p /run/nginx /run/php /var/www/html",
    "sed -i 's|listen = 127.0.0.1:9000|listen = /run/php/php-fpm.sock|' /etc/php83/php-fpm.d/www.conf",
    "printf 'server {\\n  listen 8080;\\n  root /var/www/html;\\n  index index.php;\\n  location ~ \\.php$ {\\n    fastcgi_pass unix:/run/php/php-fpm.sock;\\n    fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\\n    include fastcgi_params;\\n  }\\n}\\n' > /etc/nginx/http.d/default.conf",
    "printf '<?php\\n$start = microtime(true);\\nfunction fib($n) { return $n <= 1 ? $n : fib($n-1) + fib($n-2); }\\n$fib_result = fib(20);\\n$tmp = tempnam(\"/tmp\", \"bench_\");\\nfile_put_contents($tmp, str_repeat(\"x\", 1024));\\n$data = file_get_contents($tmp);\\nunlink($tmp);\\n$hash = hash(\"sha256\", str_repeat(\"benchmark\", 100));\\n$elapsed = (microtime(true) - $start) * 1000;\\nheader(\"Content-Type: application/json\");\\necho json_encode([\"fibonacci_20\" => $fib_result, \"hash\" => substr($hash, 0, 16), \"file_bytes\" => strlen($data), \"elapsed_ms\" => round($elapsed, 3), \"pid\" => getmypid(), \"php\" => phpversion()]);\\n' > /var/www/html/index.php",
]
[container]
command = ["/bin/sh", "-c", "mkdir -p /run/nginx /run/php && php-fpm83 && exec nginx -g 'daemon off;'"]
TOML

START=$(date +%s%N)
$CORTEN build "$CORTEN_DIR2" >/dev/null 2>&1
END=$(date +%s%N)
echo "  Corten build: $(( (END - START) / 1000000 ))ms"
rm -rf "$CORTEN_DIR2"

# Start
docker run -d --name bench-docker-phpapp -p $DOCKER_PORT_PHP:8080 bench-phpapp >/dev/null 2>&1
$CORTEN run -d --name bench-corten-phpapp -p $CORTEN_PORT_PHP:8080 bench-phpapp >/dev/null 2>&1

# Wait
DOCKER_PHP_OK=false; CORTEN_PHP_OK=false
for i in $(seq 1 15); do
    $DOCKER_PHP_OK || { [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$DOCKER_PORT_PHP/ 2>/dev/null)" = "200" ] && DOCKER_PHP_OK=true; }
    $CORTEN_PHP_OK || { [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$CORTEN_PORT_PHP/ 2>/dev/null)" = "200" ] && CORTEN_PHP_OK=true; }
    $DOCKER_PHP_OK && $CORTEN_PHP_OK && break
    sleep 1
done

echo -n "  Docker: "; if $DOCKER_PHP_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo -n "  Corten: "; if $CORTEN_PHP_OK; then green "READY"; else red "NOT RESPONDING"; fi
echo ""

# Show response
if $DOCKER_PHP_OK; then
    echo "  Docker: $(curl -s http://127.0.0.1:$DOCKER_PORT_PHP/ | head -c 100)"
fi
if $CORTEN_PHP_OK; then
    echo "  Corten: $(curl -s http://127.0.0.1:$CORTEN_PORT_PHP/ | head -c 100)"
fi
echo ""

# Benchmark
bold "  Benchmark: PHP app ($AB_REQUESTS requests — fib + file I/O + hash per request)"
echo ""

if $DOCKER_PHP_OK; then
    D_PHP=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$DOCKER_PORT_PHP/ 2>&1)
    D_PHP_RPS=$(echo "$D_PHP" | grep "Requests per second" | awk '{print $4}')
    D_PHP_MEAN=$(echo "$D_PHP" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    D_PHP_P99=$(echo "$D_PHP" | grep "99%" | awk '{print $2}')
    D_PHP_FAIL=$(echo "$D_PHP" | grep "Failed requests" | awk '{print $3}')
    echo "  Docker: ${D_PHP_RPS} req/s | mean ${D_PHP_MEAN}ms | P99 ${D_PHP_P99}ms | failed: ${D_PHP_FAIL}"
else
    D_PHP_RPS="N/A"; D_PHP_MEAN="N/A"; D_PHP_P99="N/A"
fi

if $CORTEN_PHP_OK; then
    C_PHP=$(ab -n $AB_REQUESTS -c $AB_CONCURRENCY -q http://127.0.0.1:$CORTEN_PORT_PHP/ 2>&1)
    C_PHP_RPS=$(echo "$C_PHP" | grep "Requests per second" | awk '{print $4}')
    C_PHP_MEAN=$(echo "$C_PHP" | grep "Time per request.*mean\b" | head -1 | awk '{print $4}')
    C_PHP_P99=$(echo "$C_PHP" | grep "99%" | awk '{print $2}')
    C_PHP_FAIL=$(echo "$C_PHP" | grep "Failed requests" | awk '{print $3}')
    echo "  Corten: ${C_PHP_RPS} req/s | mean ${C_PHP_MEAN}ms | P99 ${C_PHP_P99}ms | failed: ${C_PHP_FAIL}"
else
    C_PHP_RPS="N/A"; C_PHP_MEAN="N/A"; C_PHP_P99="N/A"
fi

echo ""

# ============================================================================
bold "========================================================"
bold "  Final Results — Real Applications"
bold "========================================================"
echo ""
printf "  %-35s %12s %12s\n" "Metric" "Docker" "Corten"
printf "  %-35s %12s %12s\n" "-----------------------------------" "------------" "------------"
bold "  Python REST API + SQLite"
printf "  %-35s %10s/s %10s/s\n" "  GET /api/items (list)" "$D_RPS" "$C_RPS"
printf "  %-35s %10s/s %10s/s\n" "  GET /api/items/1 (point query)" "$D_RPS2" "$C_RPS2"
printf "  %-35s %10s/s %10s/s\n" "  POST /api/items (write)" "$D_RPS3" "$C_RPS3"
printf "  %-35s %10s KB %10s KB\n" "  Memory" "$DOCKER_PY_RSS" "$CORTEN_PY_RSS"
echo ""
bold "  PHP App (fib + hash + file I/O)"
printf "  %-35s %10s/s %10s/s\n" "  Requests/sec" "${D_PHP_RPS:-N/A}" "${C_PHP_RPS:-N/A}"
printf "  %-35s %10sms %10sms\n" "  Latency (mean)" "${D_PHP_MEAN:-N/A}" "${C_PHP_MEAN:-N/A}"
printf "  %-35s %10sms %10sms\n" "  Latency (P99)" "${D_PHP_P99:-N/A}" "${C_PHP_P99:-N/A}"
echo ""
