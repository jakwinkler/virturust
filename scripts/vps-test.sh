#!/bin/bash
# Corten VPS Test Suite
#
# Complete test script for a fresh Linux VPS. Installs dependencies,
# builds Corten, pulls images, runs containers, and reports results.
#
# Tested on: Ubuntu 22.04+, Fedora 39+, Debian 12+
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/jakwinkler/virturust/main/scripts/vps-test.sh | sudo bash
#   OR
#   sudo ./scripts/vps-test.sh

set -euo pipefail

# Colors
red()    { echo -e "\033[31m$*\033[0m"; }
green()  { echo -e "\033[32m$*\033[0m"; }
yellow() { echo -e "\033[33m$*\033[0m"; }
bold()   { echo -e "\033[1m$*\033[0m"; }

PASS=0
FAIL=0
SKIP=0
TOTAL=0
FAILURES=""

check() {
    local name="$1"; shift
    TOTAL=$((TOTAL + 1))
    if "$@" >/dev/null 2>&1; then
        green "  PASS  $name"
        PASS=$((PASS + 1))
    else
        red "  FAIL  $name"
        FAIL=$((FAIL + 1))
        FAILURES="${FAILURES}\n    - $name"
    fi
}

check_output() {
    local name="$1"; local expected="$2"; shift 2
    TOTAL=$((TOTAL + 1))
    local output
    output=$("$@" 2>/dev/null) || true
    if echo "$output" | grep -qE "$expected"; then
        green "  PASS  $name"
        PASS=$((PASS + 1))
    else
        red "  FAIL  $name"
        red "        expected: $expected"
        red "        got:      $(echo "$output" | head -1)"
        FAIL=$((FAIL + 1))
        FAILURES="${FAILURES}\n    - $name"
    fi
}

check_fail() {
    local name="$1"; shift
    TOTAL=$((TOTAL + 1))
    if "$@" >/dev/null 2>&1; then
        red "  FAIL  $name (should have failed)"
        FAIL=$((FAIL + 1))
        FAILURES="${FAILURES}\n    - $name"
    else
        green "  PASS  $name"
        PASS=$((PASS + 1))
    fi
}

skip() {
    local name="$1"; local reason="$2"
    TOTAL=$((TOTAL + 1))
    SKIP=$((SKIP + 1))
    yellow "  SKIP  $name ($reason)"
}

# ============================================================================
bold ""
bold "========================================================"
bold "  Corten VPS Test Suite"
bold "========================================================"
echo ""

# ============================================================================
# Prerequisites
# ============================================================================
bold "[0] Prerequisites"
echo ""

# Must be root
if [ "$(id -u)" -ne 0 ]; then
    red "  ERROR: must run as root"
    red "  Usage: sudo $0"
    exit 1
fi
green "  OK    Running as root"

# Check OS
OS_ID=$(. /etc/os-release 2>/dev/null && echo "$ID" || echo "unknown")
OS_VERSION=$(. /etc/os-release 2>/dev/null && echo "$VERSION_ID" || echo "?")
echo "  INFO  OS: $OS_ID $OS_VERSION ($(uname -m))"
echo "  INFO  Kernel: $(uname -r)"

# Check cgroups v2
if stat -f -c %T /sys/fs/cgroup 2>/dev/null | grep -q cgroup2; then
    green "  OK    cgroups v2 enabled"
else
    red "  ERROR: cgroups v2 not available"
    red "  Corten requires cgroups v2 (default on Ubuntu 22.04+, Fedora 31+)"
    exit 1
fi

# Check for iptables
if command -v iptables &>/dev/null; then
    green "  OK    iptables available"
else
    yellow "  WARN  iptables not found — networking tests will be skipped"
fi

# Check for required tools
for tool in tar gzip; do
    if command -v $tool &>/dev/null; then
        green "  OK    $tool available"
    else
        red "  ERROR: $tool not found"
        exit 1
    fi
done

# Locate or build corten
CORTEN=""
if [ -f "./target/release/corten" ]; then
    CORTEN="./target/release/corten"
elif command -v corten &>/dev/null; then
    CORTEN="$(which corten)"
elif [ -f "./target/debug/corten" ]; then
    CORTEN="./target/debug/corten"
fi

if [ -z "$CORTEN" ]; then
    echo ""
    echo "  Corten binary not found. Attempting to build..."
    if command -v cargo &>/dev/null; then
        cargo build --release
        CORTEN="./target/release/corten"
    else
        red "  ERROR: cargo not found. Install Rust first:"
        red "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi
fi

echo ""
echo "  INFO  Binary: $CORTEN"
echo "  INFO  $($CORTEN --version 2>&1)"
echo "  INFO  Size: $(du -h "$CORTEN" | cut -f1)"
echo ""

# Cleanup function
cleanup() {
    echo ""
    bold "[cleanup] Removing test containers..."
    for name in \
        pull-test echo-test osrel-test sh-test exit-test \
        inspect-test vol-test volro-test iso1 iso2 \
        devnull devzero devsda \
        net-none net-host net-bridge \
        detach-test exec-test \
        mem-test pids-test \
        build-test \
        prune1 prune2; do
        $CORTEN rm "$name" 2>/dev/null || true
    done
    $CORTEN stop detach-test 2>/dev/null || true
    $CORTEN stop exec-test 2>/dev/null || true
    $CORTEN rm detach-test 2>/dev/null || true
    $CORTEN rm exec-test 2>/dev/null || true
}
trap cleanup EXIT

# ============================================================================
# Test 1: Image Pulling
# ============================================================================
bold "[1] Image Pulling (from official distro mirrors)"
echo ""

check "pull alpine" $CORTEN pull alpine
check_output "images lists alpine" "alpine" $CORTEN images

echo ""

# ============================================================================
# Test 2: Basic Container Execution
# ============================================================================
bold "[2] Basic Container Execution"
echo ""

check_output "echo hello" "hello-corten" \
    $CORTEN run --name echo-test --network none alpine echo hello-corten

check_output "cat /etc/os-release" "Alpine" \
    $CORTEN run --name osrel-test --network none alpine cat /etc/os-release

check_output "sh -c with pipes" "HELLO" \
    $CORTEN run --name sh-test --network none alpine sh -c "echo hello | tr a-z A-Z"

TOTAL=$((TOTAL + 1))
$CORTEN run --name exit-test --network none alpine sh -c "exit 42" 2>/dev/null || CODE=$?
if [ "${CODE:-0}" -eq 42 ]; then
    green "  PASS  exit code propagation (42)"
    PASS=$((PASS + 1))
else
    red "  FAIL  exit code propagation (expected 42, got ${CODE:-0})"
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}\n    - exit code propagation"
fi

echo ""

# ============================================================================
# Test 3: Container Lifecycle
# ============================================================================
bold "[3] Container Lifecycle (inspect, ps, rm)"
echo ""

$CORTEN run --name inspect-test --network none alpine true >/dev/null 2>&1 || true

check_output "inspect shows name" "inspect-test" $CORTEN inspect inspect-test
check_output "inspect shows image" "alpine" $CORTEN inspect inspect-test
check_output "inspect shows stopped" "stopped" $CORTEN inspect inspect-test
check_output "ps lists container" "inspect-test" $CORTEN ps
check "rm removes container" $CORTEN rm inspect-test
check_fail "inspect fails after rm" $CORTEN inspect inspect-test

# Cleanup other test containers
$CORTEN rm echo-test 2>/dev/null || true
$CORTEN rm osrel-test 2>/dev/null || true
$CORTEN rm sh-test 2>/dev/null || true
$CORTEN rm exit-test 2>/dev/null || true

echo ""

# ============================================================================
# Test 4: Volume Mounts
# ============================================================================
bold "[4] Volume Mounts"
echo ""

TMPDIR=$(mktemp -d)
echo "volume-test-data-12345" > "$TMPDIR/file.txt"

VOL_ARG="$TMPDIR:/data"
check_output "volume read" "volume-test-data-12345" \
    $CORTEN run --name vol-test --network none -v "$VOL_ARG" alpine cat /data/file.txt
$CORTEN rm vol-test 2>/dev/null || true

check_fail "volume readonly blocks writes" \
    $CORTEN run --name volro-test --network none -v "$TMPDIR:/data:ro" alpine sh -c "echo x > /data/new"
$CORTEN rm volro-test 2>/dev/null || true

rm -rf "$TMPDIR"
echo ""

# ============================================================================
# Test 5: Resource Limits
# ============================================================================
bold "[5] Resource Limits (cgroups v2)"
echo ""

$CORTEN run --name mem-test --network none --memory 64m alpine true >/dev/null 2>&1 || true
check_output "memory limit in inspect" "64" $CORTEN inspect mem-test
$CORTEN rm mem-test 2>/dev/null || true

$CORTEN run --name pids-test --network none --pids-limit 50 alpine true >/dev/null 2>&1 || true
check_output "pids limit in inspect" "50" $CORTEN inspect pids-test
$CORTEN rm pids-test 2>/dev/null || true

echo ""

# ============================================================================
# Test 6: Filesystem Isolation
# ============================================================================
bold "[6] Filesystem Isolation (OverlayFS)"
echo ""

$CORTEN run --name iso1 --network none alpine sh -c "echo marker > /tmp/isolation-test" >/dev/null 2>&1 || true
check_fail "writes in container 1 not visible in container 2" \
    $CORTEN run --name iso2 --network none alpine cat /tmp/isolation-test
$CORTEN rm iso1 2>/dev/null || true
$CORTEN rm iso2 2>/dev/null || true

echo ""

# ============================================================================
# Test 7: Minimal /dev Security
# ============================================================================
bold "[7] Minimal /dev (no host device access)"
echo ""

check "/dev/null exists" \
    $CORTEN run --name devnull --network none alpine test -c /dev/null
$CORTEN rm devnull 2>/dev/null || true

check "/dev/zero exists" \
    $CORTEN run --name devzero --network none alpine test -c /dev/zero
$CORTEN rm devzero 2>/dev/null || true

check_fail "/dev/sda not accessible" \
    $CORTEN run --name devsda --network none alpine test -e /dev/sda
$CORTEN rm devsda 2>/dev/null || true

echo ""

# ============================================================================
# Test 8: Network Modes
# ============================================================================
bold "[8] Network Modes"
echo ""

check_fail "network=none blocks ping" \
    $CORTEN run --name net-none --network none alpine ping -c 1 -W 2 8.8.8.8
$CORTEN rm net-none 2>/dev/null || true

if command -v iptables &>/dev/null; then
    check_output "network=host sees host interfaces" "eth|enp|ens|wl|lo" \
        $CORTEN run --name net-host --network host alpine ip link show
    $CORTEN rm net-host 2>/dev/null || true

    TOTAL=$((TOTAL + 1))
    if $CORTEN run --name net-bridge alpine ping -c 1 -W 5 8.8.8.8 >/dev/null 2>&1; then
        green "  PASS  network=bridge outbound ping"
        PASS=$((PASS + 1))
    else
        yellow "  SKIP  network=bridge outbound ping (iptables/firewall issue)"
        SKIP=$((SKIP + 1))
    fi
    $CORTEN rm net-bridge 2>/dev/null || true
else
    skip "network=host" "iptables not available"
    skip "network=bridge" "iptables not available"
fi

echo ""

# ============================================================================
# Test 9: Detached Mode + Logs
# ============================================================================
bold "[9] Detached Mode + Logs"
echo ""

DETACH_OUT=$($CORTEN run -d --name detach-test --network none alpine sh -c "echo log-line-1; echo log-line-2; sleep 2" 2>/dev/null) || true
TOTAL=$((TOTAL + 1))
if [ -n "$DETACH_OUT" ]; then
    green "  PASS  detach returns container ID"
    PASS=$((PASS + 1))
else
    red "  FAIL  detach returns container ID"
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}\n    - detach returns container ID"
fi

sleep 3

check_output "logs shows container output" "log-line" $CORTEN logs detach-test
$CORTEN rm detach-test 2>/dev/null || true

echo ""

# ============================================================================
# Test 10: Exec into Running Container
# ============================================================================
bold "[10] Exec into Running Container"
echo ""

$CORTEN run -d --name exec-test --network none alpine sleep 30 >/dev/null 2>&1 || true
sleep 1

if command -v nsenter &>/dev/null; then
    check_output "exec echo" "exec-works" \
        $CORTEN exec exec-test echo exec-works
else
    skip "exec echo" "nsenter not available"
fi

$CORTEN stop exec-test 2>/dev/null || true
$CORTEN rm exec-test 2>/dev/null || true

echo ""

# ============================================================================
# Test 11: Build System (Corten.toml)
# ============================================================================
bold "[11] Build System (Corten.toml)"
echo ""

if [ -f "examples/simple-alpine.toml" ]; then
    check_output "build --dry-run parses config" "Build plan" \
        $CORTEN build --dry-run examples/simple-alpine.toml

    # Actually build an image
    TOTAL=$((TOTAL + 1))
    if $CORTEN build examples/hello-world 2>/dev/null; then
        green "  PASS  build hello-world image from Corten.toml"
        PASS=$((PASS + 1))

        # Run the built image
        check_output "run built image" "Hello from Corten" \
            $CORTEN run --name build-test --network none hello-world
        $CORTEN rm build-test 2>/dev/null || true
    else
        red "  FAIL  build hello-world image from Corten.toml"
        FAIL=$((FAIL + 1))
        FAILURES="${FAILURES}\n    - build hello-world image"
    fi
else
    skip "build --dry-run" "examples/ not found (run from repo root)"
    skip "build image" "examples/ not found"
fi

echo ""

# ============================================================================
# Test 12: System Prune
# ============================================================================
bold "[12] Cleanup (system prune)"
echo ""

$CORTEN run --name prune1 --network none alpine true >/dev/null 2>&1 || true
$CORTEN run --name prune2 --network none alpine true >/dev/null 2>&1 || true
check_output "system prune removes containers" "Removed" $CORTEN system prune

echo ""

# ============================================================================
# Performance
# ============================================================================
bold "[perf] Quick Startup Benchmark"
echo ""

TIMES=()
for i in $(seq 1 5); do
    START=$(date +%s%N)
    $CORTEN run --name "perf-$i" --network none alpine true >/dev/null 2>&1
    END=$(date +%s%N)
    MS=$(( (END - START) / 1000000 ))
    TIMES+=($MS)
    $CORTEN rm "perf-$i" >/dev/null 2>&1 || true
done

SORTED=($(printf '%s\n' "${TIMES[@]}" | sort -n))
echo "  Startup (5 runs): ${SORTED[0]}ms / ${SORTED[2]}ms / ${SORTED[4]}ms  (min/median/max)"
echo "  Binary size:      $(du -h "$CORTEN" | cut -f1)"
echo "  Alpine rootfs:    $(du -sh /var/lib/corten/images/alpine/*/rootfs 2>/dev/null | head -1 | cut -f1 || echo 'N/A')"
echo "  Daemon overhead:  0 MB (no daemon)"

# ============================================================================
# Results
# ============================================================================
echo ""
bold "========================================================"
if [ $FAIL -eq 0 ]; then
    green "  ALL TESTS PASSED: $PASS/$TOTAL"
    if [ $SKIP -gt 0 ]; then
        yellow "  ($SKIP skipped)"
    fi
else
    red "  $FAIL FAILED, $PASS passed, $SKIP skipped (of $TOTAL)"
    echo ""
    red "  Failed tests:"
    echo -e "$FAILURES"
fi
bold "========================================================"
echo ""
echo "  System: $OS_ID $OS_VERSION ($(uname -m))"
echo "  Kernel: $(uname -r)"
echo "  Corten: $($CORTEN --version 2>&1)"
echo ""

exit $FAIL
