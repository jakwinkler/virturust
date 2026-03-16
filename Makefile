.PHONY: build install uninstall clean test test-integration test-e2e test-all clippy check smoke-test docker

PREFIX ?= /usr/local
BINARY = corten
TARGET = target/release/$(BINARY)

## Build the release binary
build:
	cargo build --release

## Run unit tests (no root needed)
test:
	cargo test

## Run integration tests (needs root + cgroups v2)
test-integration:
	cargo test -- --ignored --test-threads=1

## Run E2E tests (needs root + cgroups v2 + network)
test-e2e:
	cargo test -- --ignored --test-threads=1

## Run all tests
test-all:
	cargo test
	cargo test -- --ignored --test-threads=1

## Lint with clippy
clippy:
	cargo clippy --all-targets -- -D warnings

## Full pre-commit check
check:
	cargo clippy --all-targets -- -D warnings
	cargo test

## Install corten system-wide with Linux capabilities (no sudo needed after install)
##
## This is a one-time sudo operation. After installation, you can run
## 'corten' directly without sudo — Linux capabilities grant the
## binary only the specific privileges it needs.
ELEVATE := $(shell command -v sudo 2>/dev/null || command -v doas 2>/dev/null || echo "pkexec")

install: build
	@echo "=== Installing Corten (one-time root needed) ==="
	@echo ""
	@echo "1. Installing binary to $(PREFIX)/bin/$(BINARY)..."
	$(ELEVATE) install -m 755 $(TARGET) $(PREFIX)/bin/$(BINARY)
	@echo ""
	@echo "2. Setting Linux capabilities (no sudo needed after this)..."
	$(ELEVATE) setcap 'cap_sys_admin,cap_net_admin,cap_sys_chroot,cap_dac_override,cap_fowner,cap_chown,cap_setuid,cap_setgid,cap_mknod+eip' $(PREFIX)/bin/$(BINARY)
	@echo ""
	@echo "3. Creating data directories..."
	$(ELEVATE) mkdir -p /var/lib/corten/images /var/lib/corten/containers
	$(ELEVATE) chown -R $$(id -u):$$(id -g) /var/lib/corten
	@echo ""
	@echo "4. Setting up cgroup delegation..."
	$(ELEVATE) mkdir -p /sys/fs/cgroup/corten
	$(ELEVATE) chown $$(id -u):$$(id -g) /sys/fs/cgroup/corten
	@echo "+memory +cpu +pids" | $(ELEVATE) tee /sys/fs/cgroup/cgroup.subtree_control > /dev/null 2>&1 || true
	@echo ""
	@echo "=== Installation complete! ==="
	@echo ""
	@echo "You can now run corten WITHOUT sudo:"
	@echo "  corten pull alpine"
	@echo "  corten run --memory 256m --cpus 1 alpine /bin/sh"
	@echo ""

## Remove the installed binary (preserves data)
uninstall:
	sudo rm -f $(PREFIX)/bin/$(BINARY)
	@echo "Removed $(PREFIX)/bin/$(BINARY)"
	@echo "Data at /var/lib/corten was preserved. Remove manually if desired:"
	@echo "  sudo rm -rf /var/lib/corten"

## Run smoke tests (needs root + network + pulled images)
## Usage: sudo make smoke-test  (or build first with 'make build', then 'sudo ./scripts/smoke-test.sh')
smoke-test:
	@if [ ! -f target/release/corten ]; then echo "Run 'make build' first (as your user, not root)"; exit 1; fi
	./scripts/smoke-test.sh

## Build Docker image for testing
docker:
	docker build -t corten .

## Clean build artifacts
clean:
	cargo clean
