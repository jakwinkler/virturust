# Corten

**Containers without demons.**

A lightweight, high-performance container runtime written in Rust. No daemon, no shims, no Docker Hub — just your binary and the Linux kernel.

Named after [corten (weathering) steel](https://en.wikipedia.org/wiki/Weathering_steel): less weight, more strength.

## Why Corten?

Docker runs a 200MB daemon that sits in memory 24/7. Every container spawns shim processes. Port forwarding goes through a userland TCP proxy. All users share the same container namespace.

Corten does none of that:

| | Docker | Corten |
|---|---|---|
| **Architecture** | CLI -> daemon -> containerd -> shim -> runc | **Single binary, no daemon** |
| **Idle memory** | 145 MB (dockerd + containerd) | **0 MB** |
| **Container start** | 508ms | **42ms (12x faster)** |
| **100 containers** | 1,566 MB total | **430 MB (3.6x less)** |
| **Nginx req/s** | 17,737 | **22,452 (+27%)** |
| **Redis GET/s** | 77,519 | **147,059 (+90%)** |
| **MySQL SELECT/s** | 4,807 | **7,812 (+63%)** |
| **Binary size** | 179 MB (cli+dockerd+containerd) | **8.5 MB** |
| **Image source** | Docker Hub | **Official distro mirrors** |
| **User isolation** | Shared namespace | **Per-user containers** |
| **Config format** | Dockerfile (imperative) | **Corten.toml (declarative)** |

## Benchmarks

Full benchmark suite included (`scripts/`). Run on Fedora 43, kernel 6.18, AMD Ryzen:

### Throughput

| Workload | Docker | Corten | Advantage |
|----------|--------|--------|-----------|
| Nginx HTTP | 17,737 req/s | 22,452 req/s | **+27%** |
| Node.js HTTP | 6,280 req/s | 6,899 req/s | **+10%** |
| Python REST API | 1,732 req/s | 7,782 req/s | **4.5x** |
| Redis GET | 77,519/s | 147,059/s | **+90%** |
| Redis SET | 75,758/s | 97,087/s | **+28%** |
| MariaDB SELECT | 4,807/s | 7,812/s | **+63%** |
| PostgreSQL TPS | 644 | 783 | **+22%** |

### Startup & Resources

| Metric | Docker | Corten | Advantage |
|--------|--------|--------|-----------|
| Single container start | 508ms | 42ms | **12x faster** |
| 20x parallel start | 7,631ms | 651ms | **12x faster** |
| 250 working containers | 2,807 MB | 1,343 MB | **2.1x less memory** |
| Disk I/O (100MB write) | 894ms | 356ms | **2.5x faster** |
| Binary size | 179 MB | 8.5 MB | **21x smaller** |
| Daemon memory | 145 MB | 0 MB | **No daemon** |

## Quick Start

```bash
# Build and install (one-time sudo for capabilities)
git clone https://github.com/jakwinkler/virturust.git
cd virturust
make install

# Pull an image (from official Alpine mirrors, NOT Docker Hub)
corten pull alpine

# Run a container — no sudo needed
corten run alpine echo "hello from corten"

# Run with resource limits and port forwarding
corten run --memory 256m --cpus 0.5 -p 8080:80 my-nginx

# Detached mode
corten run -d --name myapp alpine sleep 3600
corten logs myapp
corten exec myapp /bin/sh
corten stop myapp && corten rm myapp
```

## Build System (Corten.toml)

Declarative image builds — no Dockerfile needed:

```toml
# Corten.toml
[image]
name = "my-app"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["nginx", "php83", "php83-fpm"]

[setup]
run = [
    "mkdir -p /run/nginx /var/www/html",
    "echo '<h1>Hello!</h1>' > /var/www/html/index.html",
]

[container]
command = ["nginx", "-g", "daemon off;"]
```

```bash
corten build .
corten run -p 8080:80 my-app
```

Also supports **JSONC** (JSON with Comments):

```jsonc
{
  // My app config
  "image": { "name": "my-app", "tag": "latest" },
  "base": { "system": "alpine", "version": "3.20" },
  "packages": { "install": ["nginx"] }
}
```

## Multi-Container Orchestration (Forge)

`Cortenforge.toml` — like Docker Compose but in TOML (no YAML, no indentation games):

```toml
# Cortenforge.toml
[services.api]
image = "my-api"
ports = ["8080:80"]
depends_on = ["db"]
memory = "256m"

[services.api.env]
DB_HOST = "db"

[services.db]
image = "my-db"
memory = "512m"
```

```bash
corten forge up       # Start in dependency order
corten forge ps       # List services
corten forge logs     # View output
corten forge down     # Stop and clean up
```

## Per-User Container Isolation

A security feature Docker doesn't have. Each user gets their own container namespace:

```
/var/lib/corten/
  images/              # Shared (all users)
  users/
    1000/containers/   # jakub's containers (only jakub sees these)
    1001/containers/   # alice's containers (only alice sees these)
```

- User A cannot see, stop, exec, or read logs of User B's containers
- The `corten` group controls who can run containers
- Images are shared read-only across all users

```bash
# Add a user to the corten group
sudo usermod -aG corten alice

# Alice can only see her own containers
su alice -c "corten ps"    # Shows only alice's containers
```

## Image Sources (No Docker Hub)

Corten pulls directly from official distro mirrors:

| Distro | Source |
|--------|--------|
| Alpine | dl-cdn.alpinelinux.org |
| Ubuntu | cloud-images.ubuntu.com |
| Debian | deb.debian.org (debootstrap) |
| Fedora | kojipkgs.fedoraproject.org |
| Arch | geo.mirror.pkgbuild.com |
| Void | repo-default.voidlinux.org |

```bash
corten pull alpine          # 3 MB, instant
corten pull ubuntu:22.04    # Cloud rootfs
corten pull fedora:40       # Koji bootstrap
```

## Networking

```bash
# Bridge (default) — full networking with NAT
corten run alpine ping 8.8.8.8

# Port forwarding
corten run -p 8080:80 my-nginx

# No network (maximum isolation)
corten run --network none alpine /bin/sh

# Host network (share host stack)
corten run --network host alpine ip addr

# Named networks with DNS
corten network create backend
corten run -d --name api --network backend my-api
corten run -d --name db  --network backend my-db
# api can reach db by name: ping db
```

## Security

- **Linux capabilities** — dropped to Docker-compatible default set (13 caps)
- **Seccomp-BPF** — blocks dangerous syscalls (reboot, kexec, etc.)
- **Masked paths** — /proc/kcore, /proc/keys, etc. not accessible
- **Read-only /proc** — /proc/sys, /proc/irq, /proc/bus
- **Minimal /dev** — only null, zero, random, urandom, tty
- **Group access control** — `corten` group membership required
- **Per-user isolation** — containers scoped to invoking user

## Requirements

- Linux kernel 5.x+ (cgroups v2, OverlayFS)
- Rust 1.85+ (for building)
- `iproute2`, `iptables` (for networking)
- `dnsmasq` (for named network DNS)

```bash
# Verify cgroups v2
stat -f -c %T /sys/fs/cgroup   # Should show "cgroup2fs"
```

## Full CLI Reference

```
corten run [OPTIONS] <IMAGE> [COMMAND]...
  -d, --detach          Run in background
  -p, --publish         Port mapping (host:container)
  -v, --volume          Bind mount (host:container[:ro])
  -e, --env             Environment variable (KEY=VALUE)
  --env-file            Load env from file
  --name                Container name
  --hostname            Container hostname
  --memory              Memory limit (256m, 1g)
  --cpus                CPU limit (0.5, 2.0)
  --pids-limit          Max processes
  --network             bridge|none|host|<name>
  --restart             no|always|on-failure:N
  --rm                  Auto-remove on exit
  --privileged          Disable security restrictions
  --read-only           Read-only root filesystem
  --entrypoint          Override entrypoint
  --rootless            User namespace mode

corten build [--dry-run] <PATH>
corten pull <IMAGE>
corten images
corten ps
corten inspect <CONTAINER>
corten logs [-f] [-n N] <CONTAINER>
corten exec <CONTAINER> <COMMAND>...
corten stop [--time N] <CONTAINER>
corten rm <CONTAINER>
corten kill [-s SIGNAL] <CONTAINER>
corten cp <SRC> <DST>
corten stats [CONTAINER]
corten network create|ls|rm <NAME>
corten forge [-f FILE] up|down|ps|logs
corten system prune
```

## Testing

```bash
cargo test                    # 185 tests (unit + integration)
cargo test -- --ignored       # E2E tests (needs make install)
```

## Benchmarks

```bash
./scripts/nginx-benchmark.sh      # Nginx HTTP
./scripts/mysql-benchmark.sh      # MariaDB SQL
./scripts/redis-benchmark.sh      # Redis
./scripts/postgres-benchmark.sh   # PostgreSQL
./scripts/nodejs-benchmark.sh     # Node.js HTTP
./scripts/startup-benchmark.sh    # Container startup speed
./scripts/throughput-benchmark.sh  # I/O + CPU + static files
./scripts/concurrent-benchmark.sh  # N concurrent containers
./scripts/realapp-benchmark.sh    # Python REST API + PHP app
./scripts/run-all-benchmarks.sh   # Run everything
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
