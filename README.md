# VirtuRust

A lightweight, high-performance container runtime written in Rust.

VirtuRust provides Docker-like containerization with minimal overhead, using Linux kernel primitives directly — no daemons, no shims, just your binary and the kernel.

## Features

- **Full namespace isolation** — PID, mount, UTS, IPC, and network namespaces
- **cgroups v2 resource limits** — memory, CPU, and process count controls
- **OCI image support** — pull images directly from Docker Hub
- **Minimal footprint** — single static binary, no runtime dependencies
- **Multi-architecture** — automatically selects the right image for your platform

## Supported images

- Alpine Linux (`alpine`, `alpine:3.19`)
- Ubuntu (`ubuntu:22.04`, `ubuntu:24.04`)
- Debian (`debian:bookworm`, `debian:bullseye`)
- Any Linux image on Docker Hub

## Requirements

- **Linux** kernel 4.18+ (for cgroups v2)
- **Root privileges** (required for namespace creation)
- **cgroups v2** mounted at `/sys/fs/cgroup` (default on modern distros)

### Verify cgroups v2

```bash
# Should show "cgroup2fs"
stat -f -c %T /sys/fs/cgroup
```

## Installation

### From source (recommended)

```bash
git clone https://github.com/jakwinkler/virturust.git
cd virturust

# Build and install with Linux capabilities (one-time sudo)
make install
```

After installation, **no sudo needed** — Linux capabilities grant only the
specific privileges required for container operations.

### Manual install

```bash
cargo build --release
sudo cp target/release/virturust /usr/local/bin/
```

### Environment variables

| Variable             | Default              | Description               |
|----------------------|----------------------|---------------------------|
| `VIRTURUST_DATA_DIR` | `/var/lib/virturust` | Image and container store |

## Usage

### Pull an image

```bash
virturust pull alpine
virturust pull ubuntu:22.04
virturust pull debian:bookworm
```

### Run a container

```bash
# Basic — run an interactive shell
virturust run alpine /bin/sh

# With resource limits
virturust run --memory 256m --cpus 0.5 alpine /bin/sh

# Full control
virturust run \
  --memory 1g \
  --cpus 2 \
  --pids-limit 100 \
  --hostname mycontainer \
  --name web-server \
  ubuntu:22.04 /bin/bash

# Run a one-off command
virturust run alpine cat /etc/os-release
```

### Resource limits

| Flag            | Description                    | Example          |
|-----------------|--------------------------------|------------------|
| `--memory`      | Memory limit (k/m/g suffixes)  | `--memory 256m`  |
| `--cpus`        | CPU limit (fractional cores)   | `--cpus 0.5`     |
| `--pids-limit`  | Max number of processes        | `--pids-limit 50`|

### Container management

```bash
# List all containers (running and stopped)
virturust ps

# Inspect a container's details
virturust inspect <name-or-id>

# Stop a running container (SIGTERM, then SIGKILL after timeout)
virturust stop <name-or-id>
virturust stop --time 30 <name-or-id>

# Remove a stopped container
virturust rm <name-or-id>

# List locally cached images
virturust images
```

## Architecture

VirtuRust is structured around Linux kernel isolation primitives:

```
┌─────────────────────────────────────────────────┐
│                   CLI (clap)                    │
├─────────────────────────────────────────────────┤
│               Container Manager                 │
│         (lifecycle orchestration)               │
├──────────┬──────────┬──────────┬────────────────┤
│Namespaces│ cgroups  │Filesystem│   Networking   │
│ (clone)  │  (v2)    │(pivot_rt)│   (netns)      │
├──────────┴──────────┴──────────┴────────────────┤
│              Image Manager                      │
│     (OCI pull from Docker Hub)                  │
├─────────────────────────────────────────────────┤
│              Linux Kernel                       │
└─────────────────────────────────────────────────┘
```

### Module overview

| Module         | Purpose                                            |
|----------------|----------------------------------------------------|
| `cli`          | Command-line argument parsing (clap derive)        |
| `config`       | Configuration types, resource limit parsing        |
| `container`    | Container lifecycle (create → run → stop → cleanup)|
| `namespace`    | Linux namespace creation via `clone()`             |
| `cgroup`       | cgroups v2 resource limit enforcement              |
| `filesystem`   | Mount setup, `pivot_root`, filesystem isolation    |
| `image`        | OCI image pulling from Docker Hub                  |
| `network`      | Network namespace setup (loopback, future: veth)   |

### How `virturust run` works

1. **Parse CLI** — validate arguments and resource limits
2. **Pull image** — download from Docker Hub if not cached locally
3. **Create cgroup** — set up resource limits in `/sys/fs/cgroup/virturust/<id>/`
4. **Clone process** — `clone()` with `CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET`
5. **Add to cgroup** — move the child PID into the cgroup
6. **Signal child** — tell the child that cgroup setup is complete (via pipe)
7. **Child: setup** — set hostname, mount `/proc`, `/sys`, `/dev`, `pivot_root`
8. **Child: exec** — replace the init process with the requested command
9. **Parent: wait** — block until the container exits
10. **Cleanup** — remove cgroup and container state

## Roadmap

- [ ] OverlayFS for copy-on-write container filesystems
- [ ] veth networking with bridge and NAT
- [ ] Port forwarding (`--publish` / `-p`)
- [ ] User namespace mapping (rootless containers)
- [ ] Container-to-container networking
- [ ] Build command (Dockerfile-like)
- [ ] Volume mounts (`--volume` / `-v`)
- [ ] Seccomp-BPF syscall filtering
- [ ] Container logs
- [ ] Compose-like multi-container orchestration

## How it compares to Docker

| Feature              | Docker          | VirtuRust (v0.1)     |
|----------------------|-----------------|----------------------|
| Container runtime    | runc + containerd| Built-in (single binary)|
| Image format         | OCI             | OCI                  |
| Resource limits      | cgroups v1/v2   | cgroups v2           |
| Networking           | Full stack      | Loopback only (WIP)  |
| Rootless mode        | Yes             | Planned              |
| Build system         | Dockerfile      | Planned              |
| Compose              | docker-compose  | Planned              |
| Platform             | Linux/Mac/Win   | Linux only           |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
