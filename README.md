# Corten

A lightweight, high-performance container runtime written in Rust.

Corten provides Docker-like containerization with minimal overhead, using Linux kernel primitives directly — no daemons, no shims, just your binary and the kernel. Named after corten (weathering) steel: less weight, more strength.

## Features

- **Full namespace isolation** — PID, mount, UTS, IPC, and network namespaces
- **cgroups v2 resource limits** — memory, CPU, and process count controls
- **OCI image support** — pull images directly from Docker Hub with full config handling (ENV, CMD, ENTRYPOINT, WORKDIR, USER)
- **OverlayFS** — copy-on-write container filesystems with per-container writable layers
- **Volume mounts** — bind mount host directories into containers (`-v /host:/container[:ro]`)
- **Full networking stack** — bridge (`corten0`), veth pairs, NAT, DNS, port forwarding (`-p`)
- **Named networks** — create isolated networks with DNS-based container name resolution
- **Detached mode** — run containers in the background (`-d`), view logs, exec into running containers
- **Security hardening** — capability dropping, seccomp-BPF syscall filtering, masked sensitive paths
- **Restart policies** — `--restart always`, `--restart on-failure:5`
- **Declarative builds** — `Corten.toml` build file parser (image building coming soon)
- **Minimal footprint** — single binary, no runtime dependencies, no daemon

## Quick start

```bash
# Install
git clone https://github.com/jakwinkler/virturust.git
cd virturust && make install

# Run a container
corten run alpine echo "hello from corten"

# Run with resource limits and port forwarding
corten run --memory 256m --cpus 0.5 -p 8080:80 nginx

# Run in the background
corten run -d --name myapp alpine sleep 3600

# View logs and exec into it
corten logs myapp
corten exec myapp /bin/sh

# Stop and clean up
corten stop myapp
corten rm myapp
```

## Requirements

- **Linux** kernel 4.18+ (for cgroups v2 and OverlayFS)
- **Root privileges** or Linux capabilities (`make install` sets these automatically)
- **cgroups v2** mounted at `/sys/fs/cgroup` (default on modern distros)

```bash
# Verify cgroups v2
stat -f -c %T /sys/fs/cgroup   # Should show "cgroup2fs"
```

## Installation

```bash
git clone https://github.com/jakwinkler/virturust.git
cd virturust
make install    # builds, installs, and sets Linux capabilities
```

After installation, **no sudo needed** for container operations.

### Environment variables

| Variable          | Default            | Description               |
|-------------------|--------------------|---------------------------|
| `CORTEN_DATA_DIR` | `/var/lib/corten`  | Image and container store |

## Usage

### Images

```bash
corten pull alpine                # Pull from Docker Hub
corten pull ubuntu:22.04
corten images                     # List local images
corten image prune                # Remove all images
```

### Running containers

```bash
# Interactive shell
corten run alpine /bin/sh

# One-off command
corten run alpine cat /etc/os-release

# With resource limits
corten run --memory 256m --cpus 0.5 --pids-limit 100 alpine /bin/sh

# Named container with custom hostname
corten run --name web --hostname webserver nginx

# Volume mounts
corten run -v /src:/app alpine ls /app
corten run -v /data:/mnt:ro alpine cat /mnt/config.txt

# Port forwarding
corten run -p 8080:80 nginx
corten run -p 127.0.0.1:3000:3000 myapp

# Detached mode (background)
corten run -d --name myapp alpine sleep 3600

# Restart policies
corten run --restart always --name daemon alpine my-service
corten run --restart on-failure:5 --name worker alpine my-job

# Network modes
corten run --network bridge alpine ping 8.8.8.8    # default, full networking
corten run --network none alpine /bin/sh            # no network
corten run --network host alpine ip addr            # share host network
```

### Container management

```bash
corten ps                         # List all containers
corten inspect <name-or-id>       # Show detailed info
corten logs <name> [-f] [-n 50]   # View container logs
corten exec <name> /bin/sh        # Exec into running container
corten stop <name>                # Stop (SIGTERM → SIGKILL)
corten stop --time 30 <name>      # Custom grace period
corten rm <name>                  # Remove stopped container
corten system prune               # Remove stopped containers + images
```

### Named networks

```bash
# Create an isolated network
corten network create backend

# Run containers on it (they can resolve each other by name)
corten run -d --name api --network backend alpine sleep 3600
corten run -d --name db  --network backend alpine sleep 3600
corten run --network backend alpine ping api    # resolves via /etc/hosts

# Manage networks
corten network ls
corten network rm backend
```

### Build system (Corten.toml)

Declarative image definition — simpler than Dockerfile:

```toml
[base]
system = "ubuntu"
version = "22.04"

[packages]
install = ["nginx", "php8.1-fpm", "php8.1-mysql"]

[files]
copy = [
    { src = "nginx.conf", dest = "/etc/nginx/sites-available/default" },
    { src = "src/", dest = "/var/www/html/", owner = "www-data" },
]

[container]
command = ["nginx", "-g", "daemon off;"]
user = "www-data"
workdir = "/var/www/html"
```

```bash
corten build .                    # Parse and validate (image building coming soon)
corten build examples/nginx-php.toml
```

See `examples/` for more Corten.toml templates.

### Resource limits

| Flag            | Description                    | Example          |
|-----------------|--------------------------------|------------------|
| `--memory`      | Memory limit (k/m/g suffixes)  | `--memory 256m`  |
| `--cpus`        | CPU limit (fractional cores)   | `--cpus 0.5`     |
| `--pids-limit`  | Max number of processes        | `--pids-limit 50`|

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    CLI (clap)                         │
├──────────────────────────────────────────────────────┤
│           Container Manager + Build System            │
│          (lifecycle, restart, Corten.toml)            │
├────────┬────────┬────────┬────────┬──────────────────┤
│  NS    │cgroups │  FS    │  Net   │    Security      │
│(clone) │ (v2)   │(pivot) │(bridge)│ (caps+seccomp)   │
├────────┴────────┴────────┴────────┴──────────────────┤
│                  Image Manager                        │
│       OCI (Docker Hub) + Native (SquashFS)            │
├──────────────────────────────────────────────────────┤
│                   Linux Kernel                        │
└──────────────────────────────────────────────────────┘
```

### Module overview

| Module       | Purpose                                              |
|--------------|------------------------------------------------------|
| `cli`        | Command-line argument parsing (clap derive)          |
| `config`     | Configuration types, volume/port/memory parsing      |
| `container`  | Container lifecycle (run, stop, restart, state)      |
| `namespace`  | Linux namespace creation via `clone()`               |
| `cgroup`     | cgroups v2 resource limit enforcement                |
| `filesystem` | OverlayFS, mount setup, `pivot_root`, minimal `/dev` |
| `image`      | OCI image pulling with config + whiteout handling    |
| `network`    | Bridge, veth, NAT, port forwarding, named networks   |
| `security`   | Capability dropping, seccomp-BPF, path masking       |
| `build`      | Corten.toml parser and build planning                |

### How `corten run` works

1. **Parse CLI** — validate arguments, resource limits, volumes, ports
2. **Pull image** — download from Docker Hub if not cached (with OCI config)
3. **OverlayFS** — set up copy-on-write layer over the image rootfs
4. **DNS** — copy host resolv.conf into container rootfs
5. **Bridge + NAT** — create `corten0` bridge, enable IP forwarding
6. **Create cgroup** — set up resource limits
7. **Clone process** — `clone()` with PID/mount/UTS/IPC/network namespaces
8. **Networking** — create veth pair, assign IP, set up routing
9. **Port forwarding** — add iptables DNAT rules
10. **Signal child** — tell child that setup is complete
11. **Child: init** — hostname, mount proc/sys/dev, volumes, `pivot_root`
12. **Child: security** — mask paths, drop capabilities, apply seccomp
13. **Child: exec** — set env/workdir/user, exec the command
14. **Parent: wait** — block until exit (or return in detach mode)
15. **Cleanup** — remove cgroup, network, overlay

## How it compares to Docker

| Feature              | Docker              | Corten                   |
|----------------------|---------------------|--------------------------|
| Architecture         | Client → daemon → containerd → runc | Single binary, no daemon |
| Container runtime    | runc + containerd   | Built-in                 |
| Image format         | OCI                 | OCI + SquashFS (planned) |
| Resource limits      | cgroups v1/v2       | cgroups v2               |
| Filesystem           | OverlayFS           | OverlayFS                |
| Networking           | Full stack           | Bridge, NAT, veth, DNS   |
| Port forwarding      | Yes                 | Yes                      |
| Volume mounts        | Yes                 | Yes                      |
| Detached mode        | Yes                 | Yes                      |
| Logs                 | Yes                 | Yes                      |
| Exec                 | Yes                 | Yes                      |
| Security             | seccomp + caps      | seccomp + caps           |
| Build system         | Dockerfile          | Corten.toml (in progress)|
| Rootless mode        | Yes                 | Planned                  |
| Compose              | docker-compose      | Planned                  |
| Platform             | Linux/Mac/Win       | Linux                    |
| Language             | Go                  | Rust                     |

## Testing

```bash
make test                # Unit tests (no root needed)
make test-integration    # Integration tests (needs root + cgroups v2)
make test-e2e            # E2E tests (needs root + cgroups v2 + network)
make test-all            # Everything
make clippy              # Lint
make check               # Lint + unit tests
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
