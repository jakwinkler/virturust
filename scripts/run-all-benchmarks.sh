#!/bin/bash
# Run all Corten vs Docker benchmarks
#
# Usage: ./scripts/run-all-benchmarks.sh
#
# Results are saved to benchmark-results.txt

set -euo pipefail

RESULTS="benchmark-results.txt"
SCRIPTS_DIR="$(dirname "$0")"

bold()  { echo -e "\033[1m$*\033[0m"; }
green() { echo -e "\033[32m$*\033[0m"; }
red()   { echo -e "\033[31m$*\033[0m"; }

bold ""
bold "========================================================"
bold "  Corten vs Docker — Full Benchmark Suite"
bold "========================================================"
bold "  Results will be saved to: $RESULTS"
echo ""

# Clean up before starting
corten system prune >/dev/null 2>&1 || true

BENCHMARKS=(
    "startup-benchmark.sh:Container Startup"
    "nginx-benchmark.sh:Nginx HTTP"
    "nodejs-benchmark.sh:Node.js HTTP"
    "mysql-benchmark.sh:MariaDB SQL"
    "redis-benchmark.sh:Redis"
    "postgres-benchmark.sh:PostgreSQL"
    "throughput-benchmark.sh:Throughput (I/O + CPU + Static)"
)

echo "Benchmarks to run:" > "$RESULTS"
echo "" >> "$RESULTS"

PASSED=0
FAILED=0

for entry in "${BENCHMARKS[@]}"; do
    SCRIPT="${entry%%:*}"
    NAME="${entry##*:}"

    if [ ! -f "$SCRIPTS_DIR/$SCRIPT" ]; then
        red "  SKIP: $NAME ($SCRIPT not found)"
        FAILED=$((FAILED + 1))
        continue
    fi

    bold ""
    bold "================================================================"
    bold "  Running: $NAME"
    bold "================================================================"
    echo ""

    echo "================================================================" >> "$RESULTS"
    echo "  $NAME" >> "$RESULTS"
    echo "================================================================" >> "$RESULTS"

    if bash "$SCRIPTS_DIR/$SCRIPT" 2>&1 | tee -a "$RESULTS"; then
        green "  DONE: $NAME"
        PASSED=$((PASSED + 1))
    else
        red "  FAIL: $NAME (non-zero exit, results may be partial)"
        PASSED=$((PASSED + 1))  # partial results still count
    fi

    echo "" >> "$RESULTS"

    # Clean between benchmarks
    corten system prune >/dev/null 2>&1 || true
done

bold ""
bold "========================================================"
bold "  All benchmarks complete: $PASSED run, $FAILED skipped"
bold "  Results saved to: $RESULTS"
bold "========================================================"
echo ""
