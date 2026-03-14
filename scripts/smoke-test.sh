#!/bin/bash
# Corten v1.1 Smoke Test
#
# Tests all major features by running real containers.
# Requires: root (or sudo), internet access, ~100MB disk for alpine image.
#
# Usage: sudo ./scripts/smoke-test.sh

set -euo pipefail

CORTEN="${CORTEN:-./target/release/corten}"
PASS=0
FAIL=0
TOTAL=0

red()   { echo -e "\033[31m$*\033[0m"; }
green() { echo -e "\033[32m$*\033[0m"; }
bold()  { echo -e "\033[1m$*\033[0m"; }

check() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    if "$@" >/dev/null 2>&1; then
        green "  PASS: $name"
        PASS=$((PASS + 1))
    else
        red "  FAIL: $name"
        FAIL=$((FAIL + 1))
    fi
}

check_output() {
    local name="$1"
    local expected="$2"
    shift 2
    TOTAL=$((TOTAL + 1))
    local output
    output=$("$@" 2>/dev/null) || true
    if echo "$output" | grep -qE "$expected"; then
        green "  PASS: $name"
        PASS=$((PASS + 1))
    else
        red "  FAIL: $name (expected '$expected', got: $(echo "$output" | head -3))"
        FAIL=$((FAIL + 1))
    fi
}

check_fail() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    if "$@" >/dev/null 2>&1; then
        red "  FAIL: $name (should have failed)"
        FAIL=$((FAIL + 1))
    else
        green "  PASS: $name"
        PASS=$((PASS + 1))
    fi
}

cleanup() {
    # Clean up any test containers
    $CORTEN system prune 2>/dev/null || true
}
trap cleanup EXIT

# ============================================================================
bold "Corten v1.1 Smoke Test"
echo ""

# Check prerequisites
if [ "$(id -u)" -ne 0 ]; then
    red "Error: must run as root (sudo $0)"
    exit 1
fi

if [ ! -f "$CORTEN" ]; then
    echo "Building release binary..."
    cargo build --release
fi

echo "Binary: $CORTEN"
$CORTEN --version
echo ""

# ============================================================================
bold "1. Image Management"
echo ""

check "pull alpine" $CORTEN pull alpine
check "images lists alpine" bash -c "$CORTEN images | grep -q alpine"
check "pull ubuntu:22.04" $CORTEN pull ubuntu:22.04

# ============================================================================
bold "2. Basic Container Execution"
echo ""

check_output "run echo" "hello-corten" \
    $CORTEN run --name echo-test --network none alpine echo hello-corten

check_output "run cat /etc/os-release" "Alpine" \
    $CORTEN run --name osrel-test --network none alpine cat /etc/os-release

check_output "run sh -c with pipes" "HELLO" \
    $CORTEN run --name sh-test --network none alpine sh -c "echo hello | tr a-z A-Z"

# Exit code propagation
TOTAL=$((TOTAL + 1))
$CORTEN run --name exit-test --network none alpine sh -c "exit 42" || CODE=$?
if [ "${CODE:-0}" -eq 42 ]; then
    green "  PASS: exit code propagation (42)"
    PASS=$((PASS + 1))
else
    red "  FAIL: exit code propagation (expected 42, got ${CODE:-0})"
    FAIL=$((FAIL + 1))
fi

# ============================================================================
bold "3. Container Lifecycle"
echo ""

check "ps lists containers" bash -c "$CORTEN ps | grep -q echo-test"
check_output "inspect shows details" "alpine" $CORTEN inspect echo-test
check "rm removes container" $CORTEN rm echo-test
check "rm removes sh-test" $CORTEN rm sh-test
check "rm removes osrel-test" $CORTEN rm osrel-test
check "rm removes exit-test" $CORTEN rm exit-test

# ============================================================================
bold "4. Volume Mounts"
echo ""

TMPDIR=$(mktemp -d)
echo "volume-test-data-12345" > "$TMPDIR/file.txt"

check_output "volume mount read" "volume-test-data-12345" \
    $CORTEN run --name vol-test --network none -v "$TMPDIR:/data" alpine cat /data/file.txt
$CORTEN rm vol-test 2>/dev/null || true

check_fail "volume mount readonly blocks writes" \
    $CORTEN run --name volro-test --network none -v "$TMPDIR:/data:ro" alpine sh -c "echo x > /data/new"
$CORTEN rm volro-test 2>/dev/null || true

rm -rf "$TMPDIR"

# ============================================================================
bold "5. Resource Limits"
echo ""

check_output "memory limit shown in inspect" "64" \
    bash -c "$CORTEN run --name mem-test --network none --memory 64m alpine true && $CORTEN inspect mem-test"
$CORTEN rm mem-test 2>/dev/null || true

check_output "pids limit shown in inspect" "50" \
    bash -c "$CORTEN run --name pids-test --network none --pids-limit 50 alpine true && $CORTEN inspect pids-test"
$CORTEN rm pids-test 2>/dev/null || true

# ============================================================================
bold "6. Filesystem Isolation"
echo ""

# Write in container 1, verify not visible in container 2
$CORTEN run --name iso1 --network none alpine sh -c "echo marker > /tmp/iso-test" 2>/dev/null || true
check_fail "filesystem isolation" \
    $CORTEN run --name iso2 --network none alpine cat /tmp/iso-test
$CORTEN rm iso1 2>/dev/null || true
$CORTEN rm iso2 2>/dev/null || true

# ============================================================================
bold "7. Minimal /dev Security"
echo ""

check "dev/null exists" \
    $CORTEN run --name devnull --network none alpine test -c /dev/null
$CORTEN rm devnull 2>/dev/null || true

check "dev/zero exists" \
    $CORTEN run --name devzero --network none alpine test -c /dev/zero
$CORTEN rm devzero 2>/dev/null || true

check_fail "dev/sda NOT accessible" \
    $CORTEN run --name devsda --network none alpine test -e /dev/sda
$CORTEN rm devsda 2>/dev/null || true

# ============================================================================
bold "8. Network Modes"
echo ""

check_fail "network none blocks ping" \
    $CORTEN run --name net-none --network none alpine ping -c 1 -W 2 8.8.8.8
$CORTEN rm net-none 2>/dev/null || true

check_output "network host shows host interfaces" "eth|enp|ens|wl" \
    $CORTEN run --name net-host --network host alpine ip link show
$CORTEN rm net-host 2>/dev/null || true

# Bridge networking (requires working iptables)
TOTAL=$((TOTAL + 1))
if $CORTEN run --name net-bridge alpine ping -c 1 -W 5 8.8.8.8 2>/dev/null; then
    green "  PASS: bridge networking (outbound ping)"
    PASS=$((PASS + 1))
else
    red "  FAIL: bridge networking (outbound ping) — may need iptables"
    FAIL=$((FAIL + 1))
fi
$CORTEN rm net-bridge 2>/dev/null || true

# ============================================================================
bold "9. Detached Mode + Logs"
echo ""

DETACH_ID=$($CORTEN run -d --name detach-test --network none alpine sh -c "echo log-line-1 && echo log-line-2 && sleep 2" 2>/dev/null)
if [ -n "$DETACH_ID" ]; then
    green "  PASS: detach returns container ID"
    PASS=$((PASS + 1))
else
    red "  FAIL: detach returns container ID"
    FAIL=$((FAIL + 1))
fi
TOTAL=$((TOTAL + 1))

sleep 3  # wait for container to finish

check_output "logs shows output" "log-line" $CORTEN logs detach-test
$CORTEN rm detach-test 2>/dev/null || true

# ============================================================================
bold "10. Build System (Corten.toml)"
echo ""

check_output "build parses example" "Build plan" $CORTEN build examples/simple-alpine.toml
check_output "build parses nginx example" "nginx" $CORTEN build examples/nginx-php.toml

# ============================================================================
bold "11. Ubuntu Image"
echo ""

check_output "run ubuntu" "Ubuntu" \
    $CORTEN run --name ubuntu-test --network none ubuntu:22.04 cat /etc/os-release
$CORTEN rm ubuntu-test 2>/dev/null || true

# ============================================================================
bold "12. System Prune"
echo ""

# Create some throwaway containers
$CORTEN run --name prune1 --network none alpine true 2>/dev/null || true
$CORTEN run --name prune2 --network none alpine true 2>/dev/null || true
check_output "system prune removes stopped containers" "Removed" $CORTEN system prune

# ============================================================================
echo ""
bold "============================================"
if [ $FAIL -eq 0 ]; then
    green "ALL $TOTAL TESTS PASSED"
else
    echo "$PASS/$TOTAL passed, $FAIL failed"
fi
bold "============================================"
echo ""

exit $FAIL
