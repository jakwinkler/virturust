# Corten Roadmap

Phased implementation plan ordered by dependency and priority.
Each phase builds on the previous one. Phases cannot be skipped.

---

## Phase 0: Foundation (rename + critical fixes)

Fix the bugs that make the current runtime incorrect and unsafe.
No new features until these are resolved.

### 0.1 Rename virturust to corten

- [ ] Rename crate in `Cargo.toml` (`name = "corten"`)
- [ ] Rename binary in `Cargo.toml`
- [ ] Update all `virturust` references in source code
- [ ] Update CLI name, help text, and error messages
- [ ] Rename data directory: `/var/lib/corten/`
- [ ] Rename env var: `CORTEN_DATA_DIR`
- [ ] Rename cgroup path: `/sys/fs/cgroup/corten/`
- [ ] Update `Makefile`, `README.md`

### 0.2 Fix unsafe pivot_root

The current `pivot_root` call passes raw path pointers that may not be
NUL-terminated. Convert paths to `CString` before passing to the syscall.

- [ ] Use `path_to_cstring()` for both args in the `pivot_root` syscall
- [ ] Add error context for the conversion

Files: `src/filesystem.rs`

### 0.3 Minimal /dev instead of host bind mount

The current code bind-mounts the host `/dev` into the container, giving
it access to all host devices. Replace with a minimal tmpfs `/dev`.

- [ ] Mount tmpfs on `/dev` inside the container
- [ ] Create device nodes: null, zero, random, urandom, tty, console
- [ ] Create `/dev/pts` (devpts) and `/dev/ptmx` symlink
- [ ] Create `/dev/shm` (tmpfs)
- [ ] Create standard symlinks: stdin → /proc/self/fd/0, stdout → fd/1, stderr → fd/2

Files: `src/filesystem.rs`

### 0.4 Per-container writable rootfs

Containers currently run directly on the cached image rootfs. Writes
in one container mutate the image and leak into other containers.

- [ ] Create per-container directory: `/var/lib/corten/containers/<id>/rootfs/`
- [ ] Copy image rootfs into container directory (or use OverlayFS — see Phase 1)
- [ ] Minimum viable: recursive copy of image rootfs per container
- [ ] Pass container-specific rootfs to namespace setup

Files: `src/container.rs`, `src/config.rs`

### 0.5 Fix OCI whiteout handling

OCI image layers use whiteout files (`.wh.<name>`) and opaque directories
(`.wh..wh..opq`) to represent deletions. Currently these are ignored,
producing incorrect root filesystems for many real-world images.

- [ ] During layer extraction, detect `.wh.<name>` files and delete the target
- [ ] Detect `.wh..wh..opq` and clear the directory before extracting the layer
- [ ] Remove the whiteout marker files themselves after processing
- [ ] Add tests with synthetic whiteout layers

Files: `src/image.rs`

### Verification

```bash
# All existing tests still pass
cargo test

# Containers no longer share writable state
corten run alpine sh -c "echo test > /tmp/marker"
corten run alpine cat /tmp/marker  # should fail (file not found)

# Container cannot see host devices
corten run alpine ls /dev/sda  # should fail
```

---

## Phase 1: Core Container Features

Make Corten actually useful for real workloads.

### 1.1 OverlayFS

Replace the rootfs copy from 0.4 with proper OverlayFS. This enables
efficient layer sharing between containers using the same image.

- [ ] Create overlay directories per container: `upper/`, `work/`, `merged/`
- [ ] Mount OverlayFS: `lowerdir=<image rootfs>, upperdir=upper, workdir=work`
- [ ] Use `merged/` as the container rootfs
- [ ] Clean up overlay mount on container exit
- [ ] Fall back to copy if OverlayFS is not available (old kernels)

Files: new `src/filesystem/overlay.rs`, `src/container.rs`

### 1.2 Volume mounts

```bash
corten run -v /host/path:/container/path alpine ls /container/path
corten run -v /host/path:/container/path:ro alpine cat /container/path/file
```

- [ ] Add `VolumeMount` struct to `config.rs` (host_path, container_path, read_only)
- [ ] Add `parse_volume()` function: `/host:/container[:ro]`
- [ ] Add `-v / --volume` repeated arg to `RunArgs` in `cli.rs`
- [ ] Add `volumes: Vec<VolumeMount>` to `ContainerConfig`
- [ ] Pass volumes through `ChildArgs` to the child process
- [ ] Implement `mount_volumes()` in filesystem — bind mounts after rootfs setup, before `pivot_root`
- [ ] For `:ro` volumes, remount with `MS_RDONLY | MS_REMOUNT | MS_BIND`
- [ ] Display volumes in `inspect` output
- [ ] Tests for `parse_volume()` edge cases
- [ ] Tests for `-v` flag in CLI

Files: `src/config.rs`, `src/cli.rs`, `src/namespace.rs`, `src/filesystem.rs`, `src/container.rs`, `src/main.rs`

### 1.3 OCI image config handling

Docker images carry configuration (ENV, WORKDIR, USER, ENTRYPOINT, CMD)
that Corten currently ignores.

- [ ] Parse OCI image config JSON during pull
- [ ] Store config alongside the rootfs
- [ ] Apply ENV vars before exec (set in child process)
- [ ] Apply WORKDIR (chdir before exec)
- [ ] Apply USER (setuid/setgid before exec)
- [ ] Handle ENTRYPOINT + CMD combination logic
- [ ] Allow CLI args to override image defaults

Files: `src/image.rs`, `src/namespace.rs`, `src/config.rs`

### 1.4 Networking: full stack

The network stack needs to support outbound connectivity, inbound port
forwarding, and container-to-container communication. This is split into
four sub-phases, each building on the last.

#### 1.4.1 Bridge + veth + NAT (outbound connectivity)

Containers can reach the internet.

- [ ] Create `corten0` bridge on first container run (10.0.42.1/24)
- [ ] Enable IP forwarding on the host (`/proc/sys/net/ipv4/ip_forward`)
- [ ] Create veth pair per container (one end in host NS, one in container NS)
- [ ] Assign unique IP from 10.0.42.0/24 range (simple file-based allocator)
- [ ] Add iptables MASQUERADE rule on the host for outbound NAT
- [ ] Set default route inside container to bridge IP (10.0.42.1)
- [ ] Inject `/etc/resolv.conf` with host nameservers
- [ ] Clean up veth pair on container exit
- [ ] Clean up bridge when last container stops

Files: new `src/network/mod.rs`, `src/network/bridge.rs`, `src/network/nat.rs`, `src/network/dns.rs`

#### 1.4.2 Port forwarding (inbound connectivity)

Host ports forward into containers.

```bash
corten run -p 8080:80 nginx
corten run -p 127.0.0.1:3000:3000 myapp
```

- [ ] Add `-p / --publish` repeated arg to `RunArgs`
- [ ] Parse formats: `host:container`, `ip:host:container`, `container` (auto host port)
- [ ] Add iptables DNAT rule for each port mapping
- [ ] Store port mappings in container state for `ps` and `inspect` display
- [ ] Clean up DNAT rules on container exit

Files: `src/cli.rs`, `src/network/nat.rs`, `src/container.rs`, `src/config.rs`

#### 1.4.3 Container-to-container networking

Containers on the same bridge can talk to each other by IP.

- [ ] Verify containers on `corten0` can reach each other (should work once
      bridge + veth is correct — this is mostly testing)
- [ ] Add `--network` flag: `--network bridge` (default), `--network none`, `--network host`
- [ ] `--network none`: skip veth setup, loopback only
- [ ] `--network host`: skip CLONE_NEWNET, share host network namespace
- [ ] Display container IP in `inspect` and `ps` output

Files: `src/cli.rs`, `src/namespace.rs`, `src/network/mod.rs`, `src/config.rs`

#### 1.4.4 Named networks + DNS service discovery

Containers can find each other by name.

```bash
corten network create backend
corten run --name api --network backend alpine
corten run --name db  --network backend alpine ping api  # resolves to api's IP
```

- [ ] Add `corten network create/ls/rm` subcommands
- [ ] Each named network gets its own bridge + subnet (10.0.43.0/24, 10.0.44.0/24, ...)
- [ ] Store network metadata in `/var/lib/corten/networks/<name>/`
- [ ] Built-in DNS resolver: listen on 127.0.0.11 inside containers
- [ ] Resolve container names to IPs within the same network
- [ ] Inject DNS server into container `/etc/resolv.conf`
- [ ] Allow connecting a container to multiple networks

Files: new `src/network/named.rs`, `src/network/dns.rs`, `src/cli.rs`, `src/config.rs`

### Verification

```bash
# Volumes
mkdir -p /tmp/test-vol && echo "hello" > /tmp/test-vol/file.txt
corten run -v /tmp/test-vol:/data alpine cat /data/file.txt  # prints "hello"
corten run -v /tmp/test-vol:/data:ro alpine sh -c "echo x > /data/new"  # fails

# Outbound networking
corten run alpine ping -c 1 8.8.8.8       # works

# Port forwarding
corten run -d -p 8080:80 --name web nginx
curl localhost:8080                         # served by container

# Container-to-container
corten run -d --name server alpine sleep 3600
corten run --name client alpine ping -c 1 <server-ip>  # works

# Named networks + DNS
corten network create mynet
corten run -d --name db --network mynet alpine sleep 3600
corten run --network mynet alpine ping -c 1 db          # resolves by name

# OverlayFS
corten run alpine sh -c "echo test > /marker"
corten run alpine cat /marker              # fails (isolated writable layer)
```

---

## Phase 2: Security Hardening

Make containers secure enough for untrusted workloads.

### 2.1 Capability dropping

- [ ] Drop all capabilities except a safe default set in the child process
- [ ] Allow `--cap-add` / `--cap-drop` CLI flags
- [ ] Default set: CHOWN, DAC_OVERRIDE, FOWNER, FSETID, KILL, NET_BIND_SERVICE,
      SETFCAP, SETGID, SETPCAP, SETUID, SYS_CHROOT

Files: new `src/security/caps.rs`, `src/namespace.rs`

### 2.2 Seccomp-BPF

- [ ] Load a default seccomp filter before exec in the child process
- [ ] Block dangerous syscalls: kexec_load, reboot, swapon/off, mount (post-setup),
      clock_settime, init_module, delete_module, acct, quotactl
- [ ] Allow override with `--security-opt seccomp=unconfined`

Files: new `src/security/seccomp.rs`, `src/namespace.rs`

### 2.3 Rootless mode (user namespaces)

- [ ] Add `CLONE_NEWUSER` to clone flags when not running as root
- [ ] Map UID/GID ranges via `/proc/<pid>/uid_map` and `/proc/<pid>/gid_map`
- [ ] Use `newuidmap` / `newgidmap` helpers if available
- [ ] Handle `/etc/subuid` and `/etc/subgid` configuration

Files: `src/namespace.rs`, `src/config.rs`

### 2.4 Masked and read-only paths

- [ ] Mask sensitive paths: `/proc/kcore`, `/proc/keys`, `/proc/timer_list`,
      `/proc/sched_debug`, `/sys/firmware`
- [ ] Mount masked paths as bind to `/dev/null`
- [ ] Make `/proc/sys`, `/proc/irq`, `/proc/bus` read-only

Files: `src/filesystem.rs`

### Verification

```bash
# Capabilities
corten run alpine cat /proc/self/status | grep Cap  # reduced capability set

# Seccomp
corten run alpine reboot  # blocked by seccomp

# Rootless
corten run alpine whoami  # works without sudo
```

---

## Phase 3: Build System

Corten's differentiator: declarative image building with Corten.toml.

### 3.1 Corten.toml parser

- [ ] Define TOML schema: `[image]`, `[base]`, `[packages]`, `[files]`, `[env]`,
      `[setup]`, `[container]`
- [ ] Parse and validate with `toml` crate
- [ ] Map to internal build plan struct

Files: new `src/build/toml.rs`

### 3.2 Base OS bootstrapping

- [ ] Auto-detect package manager from `[base].system`
- [ ] Implement bootstrappers:
  - `debootstrap` for ubuntu/debian
  - `apk --root` for alpine
  - `dnf --installroot` for fedora/rhel
- [ ] Create minimal rootfs in a temp directory

Files: new `src/build/bootstrap.rs`, `src/build/package.rs`

### 3.3 Image packing

- [ ] Install packages via detected package manager
- [ ] Copy files with ownership from `[files]` section
- [ ] Run `[setup]` commands in chroot
- [ ] Clean package caches automatically
- [ ] Pack result into SquashFS with `mksquashfs`

Files: new `src/build/mod.rs`

### 3.4 CLI integration

- [ ] Add `corten build [PATH]` subcommand (reads Corten.toml from PATH)
- [ ] Add `corten import <file>` for importing existing squashfs/tar.gz

Files: `src/cli.rs`, `src/main.rs`

### Verification

```bash
# Build from Corten.toml
cat > Corten.toml <<EOF
[base]
system = "alpine"
version = "3.19"

[packages]
install = ["curl", "jq"]

[container]
command = ["sh"]
EOF

corten build .
corten run my-image curl --version  # curl is available
```

---

## Phase 4: Lifecycle + Ecosystem

### 4.1 Detached mode + logs

- [ ] Add `-d / --detach` flag — run container in background
- [ ] Redirect stdout/stderr to log files in container directory
- [ ] Add `corten logs <name>` command
- [ ] Add `--follow / -f` flag for streaming logs

### 4.2 Exec into running containers

- [ ] Add `corten exec <container> <command>` subcommand
- [ ] Enter existing container namespaces via `setns()`
- [ ] Share the container's cgroup

### 4.3 Restart policies + health checks

- [ ] Add `--restart=no|always|on-failure[:max]` flag
- [ ] Add `--health-cmd`, `--health-interval`, `--health-retries`
- [ ] Supervisor loop in parent process for restart policies

### 4.4 Image garbage collection

- [ ] `corten image prune` — remove unused images
- [ ] `corten system prune` — remove stopped containers + unused images
- [ ] Track image usage timestamps

### 4.5 Corten registry

- [ ] Simple HTTP server hosting SquashFS files + JSON index
- [ ] `corten push <image>` to upload to registry
- [ ] `corten pull <image>` resolves native images before falling back to OCI
- [ ] Support OCI registries as alternative storage (squashfs as OCI artifact blobs)

### 4.6 OCI <-> SquashFS conversion

- [ ] `corten convert <image> --to native` — OCI layers → SquashFS
- [ ] `corten convert <image> --to oci` — SquashFS → OCI layers

---

## Phase 5: Machine Images (future)

Second execution mode: run bootable disk images with full OS isolation.

### 5.1 VM runtime

- [ ] KVM/QEMU backend for running qcow2/raw images
- [ ] `corten machine run <image>` subcommand
- [ ] Resource limits (memory, vCPUs) passed to VM
- [ ] Serial console access

### 5.2 Machine image management

- [ ] `corten machine import iso <file>` — install ISO to sealed disk image
- [ ] `corten machine import qcow2 <file>` — import existing disk image
- [ ] Copy-on-write snapshots for per-instance state

### 5.3 CLI restructuring

- [ ] Namespace commands: `corten container run` / `corten machine run`
- [ ] Keep `corten run` as alias for `corten container run` (backward compat)

---

## Testing Framework

The current test suite has ~630 lines across 5 files, but most integration
tests are `#[ignore]` because they need root, cgroups, or network access.
This makes it easy to ship regressions unnoticed.

### Problem with current approach

| What works | What doesn't |
|------------|-------------|
| Unit tests for parsing (config, CLI) | No tests for actual container runs |
| Serialization roundtrips | No tests for filesystem isolation |
| CLI help/version output | No tests for networking |
| cgroup tests exist but are `#[ignore]` | No tests for volume mounts |
| Image path tests | No tests for OverlayFS |

### Test tiers

Organize tests into three tiers with clear naming and CI support.

#### Tier 1: Unit tests (no privileges, no I/O)

Run on every commit, every PR, every local `cargo test`.

- Pure function tests: parsing, serialization, validation
- No filesystem, no network, no root
- Target: <1 second total runtime

```
tests/
  unit/
    config_test.rs        # parse_memory, parse_image_ref, parse_volume
    cli_test.rs           # argument parsing via binary
    state_test.rs         # ContainerState serialization, status display
    network_parse_test.rs # parse port mappings, IP allocation logic
    overlay_test.rs       # overlay path construction logic
```

Convention: `cargo test` runs all tier 1 tests by default.

#### Tier 2: Integration tests (need root + cgroups)

Run with `cargo test -- --ignored` or via a dedicated `make test-integration`
target. Require a Linux host with root and cgroups v2.

- Actual container creation and execution
- Filesystem isolation verification
- cgroup enforcement
- Volume mount behavior
- OverlayFS correctness

```
tests/
  integration/
    container_run_test.rs   # run alpine, check exit code, verify isolation
    cgroup_test.rs          # create/destroy cgroups, verify limits
    filesystem_test.rs      # verify /dev is minimal, rootfs is isolated
    overlay_test.rs         # verify writable layer isolation
    volume_test.rs          # verify bind mounts, read-only mounts
    whiteout_test.rs        # verify OCI whiteout handling
```

Test helper pattern for integration tests:

```rust
/// Helper: skip test if not running as root with cgroups v2.
fn require_root_and_cgroups() {
    if !is_root() || !cgroups_v2_available() {
        eprintln!("skipping: requires root + cgroups v2");
        return;
    }
}

/// Helper: run a container and capture exit code + stdout.
fn run_container(image: &str, cmd: &[&str]) -> (i32, String) {
    // Build ContainerConfig, call container::run(), capture output
}

/// Helper: create a temp directory that auto-cleans.
fn temp_container_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
```

Convention: all tier 2 tests use `#[ignore]` attribute. The `Makefile`
provides `make test-integration` that runs them as root.

#### Tier 3: Network + E2E tests (need root + network)

Run with `make test-e2e`. Require root, cgroups v2, network access,
and a pulled image.

- Outbound connectivity (ping, DNS resolution)
- Port forwarding (start container, curl from host)
- Container-to-container communication
- Named network DNS resolution
- Image pulling from Docker Hub

```
tests/
  e2e/
    network_outbound_test.rs  # container can reach external hosts
    port_forward_test.rs      # -p flag works from host
    c2c_test.rs               # two containers communicate via bridge
    dns_test.rs               # named network DNS resolution
    pull_test.rs              # pull and run real images
    lifecycle_test.rs         # run → stop → inspect → rm full cycle
```

Test helper pattern for E2E tests:

```rust
/// Helper: ensure alpine image is available (pull if not).
fn ensure_image(name: &str, tag: &str) {
    if !image::image_exists(name, tag) {
        tokio::runtime::Runtime::new().unwrap()
            .block_on(image::pull_image(name, tag)).unwrap();
    }
}

/// Helper: start a detached container, return its ID.
fn start_detached(image: &str, name: &str, args: &[&str]) -> String {
    // Build config, run in background, return container ID
}

/// Helper: wait for a port to become reachable.
fn wait_for_port(port: u16, timeout: Duration) -> bool {
    // Poll TcpStream::connect until success or timeout
}
```

Convention: all tier 3 tests use `#[ignore]` and are tagged with a module
path that `make test-e2e` targets specifically.

### CI configuration

```makefile
# Makefile targets

test:                ## Run unit tests (no root needed)
	cargo test

test-integration:    ## Run integration tests (needs root + cgroups v2)
	sudo -E cargo test -- --ignored --test-threads=1

test-e2e:            ## Run E2E tests (needs root + network + pulled images)
	sudo -E cargo test -- --ignored --test-threads=1

test-all:            ## Run everything
	cargo test
	sudo -E cargo test -- --ignored --test-threads=1

clippy:              ## Lint
	cargo clippy --all-targets -- -D warnings

check:               ## Full pre-commit check
	cargo clippy --all-targets -- -D warnings
	cargo test
```

Note: `--test-threads=1` for integration/E2E tests because they share
global resources (cgroups, bridges, iptables rules, container data dir).

### GitHub Actions CI

```yaml
jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - cargo test                    # tier 1 only, no root

  integration-tests:
    runs-on: ubuntu-latest
    steps:
      - sudo cargo test -- --ignored  # tier 2 + 3, runs as root
```

### Test coverage targets per phase

| Phase | What to test | Tier |
|-------|-------------|------|
| **0** | pivot_root fix, minimal /dev contents, rootfs isolation, whiteout handling | 2 |
| **1.1** | OverlayFS mount/unmount, writable layer isolation | 2 |
| **1.2** | parse_volume() edge cases, bind mount behavior, read-only enforcement | 1 + 2 |
| **1.3** | OCI config parsing, ENV/WORKDIR/USER application | 1 + 2 |
| **1.4.1** | Bridge creation, veth setup, outbound ping | 3 |
| **1.4.2** | Port mapping parsing, DNAT rule creation, port reachability | 1 + 3 |
| **1.4.3** | Container-to-container ping across bridge | 3 |
| **1.4.4** | Network CRUD, DNS name resolution between containers | 1 + 3 |
| **2** | Capability set verification, seccomp blocks dangerous syscalls | 2 |
| **3** | Corten.toml parsing, package manager detection | 1 |

### Dependencies to add

```toml
[dev-dependencies]
tempfile = "3"       # temp directories for test isolation
assert_cmd = "2"     # CLI binary testing (replaces manual Command usage)
predicates = "3"     # assertion helpers for assert_cmd
```

### Migration from current tests

The existing test files stay where they are for now. As each phase is
implemented, new tests go into the tiered structure. Once all existing
tests are migrated, remove the old files.

```
# Current (keep working)
tests/config_tests.rs        → eventually: tests/unit/config_test.rs
tests/cli_tests.rs           → eventually: tests/unit/cli_test.rs
tests/container_tests.rs     → eventually: tests/unit/state_test.rs
tests/cgroup_tests.rs        → eventually: tests/integration/cgroup_test.rs
tests/image_tests.rs         → split: unit → tests/unit/, integration → tests/e2e/
```

---

## Implementation Order Summary

| Phase | Focus | Depends on |
|-------|-------|------------|
| **0** | Rename + fix critical bugs | Nothing |
| **1.1-1.3** | OverlayFS, volumes, OCI config | Phase 0 |
| **1.4.1** | Bridge + veth + NAT (outbound) | Phase 0 |
| **1.4.2** | Port forwarding (inbound) | 1.4.1 |
| **1.4.3** | Container-to-container networking | 1.4.1 |
| **1.4.4** | Named networks + DNS discovery | 1.4.3 |
| **2** | Seccomp, caps, rootless, masked paths | Phase 1 |
| **3** | Corten.toml build system, SquashFS | Phase 1 |
| **4** | Detach, logs, exec, registry, GC | Phase 1 |
| **5** | Machine images (KVM/QEMU) | Phase 1 |

Testing runs continuously alongside all phases — each feature ships with
its corresponding tests in the tiered framework.

Phases 1.1-1.3 and 1.4.1 can be worked on in parallel.
Phases 2, 3, and 4 can be worked on in parallel once Phase 1 is complete.
Phase 5 is independent and can start any time after Phase 1.
