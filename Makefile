.PHONY: build install uninstall clean test

PREFIX ?= /usr/local
BINARY = virturust
TARGET = target/release/$(BINARY)

## Build the release binary
build:
	cargo build --release

## Run all tests
test:
	cargo test

## Install virturust system-wide with Linux capabilities (no sudo needed after install)
##
## This is a one-time sudo operation. After installation, you can run
## 'virturust' directly without sudo — Linux capabilities grant the
## binary only the specific privileges it needs.
install: build
	@echo "=== Installing VirtuRust ==="
	@echo ""
	@echo "1. Installing binary to $(PREFIX)/bin/$(BINARY)..."
	sudo install -m 755 $(TARGET) $(PREFIX)/bin/$(BINARY)
	@echo ""
	@echo "2. Setting Linux capabilities (replaces need for sudo)..."
	sudo setcap 'cap_sys_admin,cap_net_admin,cap_sys_chroot,cap_dac_override,cap_fowner,cap_setuid,cap_setgid,cap_mknod+eip' $(PREFIX)/bin/$(BINARY)
	@echo ""
	@echo "3. Creating data directories..."
	sudo mkdir -p /var/lib/virturust/images /var/lib/virturust/containers
	sudo chown -R $$(id -u):$$(id -g) /var/lib/virturust
	@echo ""
	@echo "4. Setting up cgroup delegation..."
	sudo mkdir -p /sys/fs/cgroup/virturust
	sudo chown $$(id -u):$$(id -g) /sys/fs/cgroup/virturust
	@echo "+memory +cpu +pids" | sudo tee /sys/fs/cgroup/cgroup.subtree_control > /dev/null 2>&1 || true
	@echo ""
	@echo "=== Installation complete! ==="
	@echo ""
	@echo "You can now run virturust WITHOUT sudo:"
	@echo "  virturust pull alpine"
	@echo "  virturust run --memory 256m --cpus 1 alpine /bin/sh"
	@echo ""

## Remove the installed binary (preserves data)
uninstall:
	sudo rm -f $(PREFIX)/bin/$(BINARY)
	@echo "Removed $(PREFIX)/bin/$(BINARY)"
	@echo "Data at /var/lib/virturust was preserved. Remove manually if desired:"
	@echo "  sudo rm -rf /var/lib/virturust"

## Clean build artifacts
clean:
	cargo clean
