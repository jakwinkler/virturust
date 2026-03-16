# Corten

**Containers without demons.**

A lightweight, high-performance container runtime written in Rust. No daemon, no shims, no Docker Hub — just your binary and the Linux kernel.

Named after [corten (weathering) steel](https://en.wikipedia.org/wiki/Weathering_steel): less weight, more strength.

## Why Corten?

Docker runs a 200MB daemon that sits in memory 24/7. 
Every container spawns shim processes. Port forwarding goes through a userland TCP proxy. 
All users share the same container namespace.

Corten does none of that:

|                     | Docker                                      | Corten                        |
|---------------------|---------------------------------------------|-------------------------------|
| **Architecture**    | CLI -> daemon -> containerd -> shim -> runc | **Single binary, no daemon**  |
| **Idle memory**     | 145 MB (dockerd + containerd)               | **0 MB**                      |
| **Container start** | 508ms                                       | **42ms (12x faster)**         |
| **100 containers**  | 1,566 MB total                              | **430 MB (3.6x less)**        |
| **Nginx req/s**     | 17,737                                      | **22,452 (+27%)**             |
| **Redis GET/s**     | 77,519                                      | **147,059 (+90%)**            |
| **MySQL SELECT/s**  | 4,807                                       | **7,812 (+63%)**              |
| **Binary size**     | 179 MB (cli+dockerd+containerd)             | **8.5 MB**                    |
| **Image source**    | Docker Hub                                  | **Official distro mirrors**   |
| **User isolation**  | Shared namespace                            | **Per-user containers**       |
| **Config format**   | Dockerfile (imperative)                     | **Corten.toml (declarative)** |

## Benchmarks

Full benchmark suite included (`scripts/`). Run on Fedora 43, kernel 6.18, AMD Ryzen:

### Throughput

| Workload        | Docker       | Corten       | Advantage |
|-----------------|--------------|--------------|-----------|
| Nginx HTTP      | 17,737 req/s | 22,452 req/s | **+27%**  |
| Node.js HTTP    | 6,280 req/s  | 6,899 req/s  | **+10%**  |
| Python REST API | 1,732 req/s  | 7,782 req/s  | **4.5x**  |
| Redis GET       | 77,519/s     | 147,059/s    | **+90%**  |
| Redis SET       | 75,758/s     | 97,087/s     | **+28%**  |
| MariaDB SELECT  | 4,807/s      | 7,812/s      | **+63%**  |
| PostgreSQL TPS  | 644          | 783          | **+22%**  |

### Startup & Resources

| Metric                 | Docker   | Corten   | Advantage            |
|------------------------|----------|----------|----------------------|
| Single container start | 508ms    | 42ms     | **12x faster**       |
| 20x parallel start     | 7,631ms  | 651ms    | **12x faster**       |
| 250 working containers | 2,807 MB | 1,343 MB | **2.1x less memory** |
| Disk I/O (100MB write) | 894ms    | 356ms    | **2.5x faster**      |
| Binary size            | 179 MB   | 8.5 MB   | **21x smaller**      |
| Daemon memory          | 145 MB   | 0 MB     | **No daemon**        |

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

## Build System — Corten.toml vs Dockerfile

Dockerfiles are **imperative** — a sequence of shell commands where order matters, caching is fragile, and one wrong `RUN` layer bloats your image. Corten.toml is **declarative** — you describe what you want, Corten figures out how to build it.

### Side-by-side Comparison

**Dockerfile (Docker):**
```dockerfile
FROM alpine:3.20
RUN apk add --no-cache nginx php83 php83-fpm
RUN mkdir -p /run/nginx /var/www/html
RUN echo '<h1>Hello!</h1>' > /var/www/html/index.html
# Wait, should I have combined those RUN commands?
# Each one creates a layer... let me squash them:
RUN apk add --no-cache nginx php83 php83-fpm \
    && mkdir -p /run/nginx /var/www/html \
    && echo '<h1>Hello!</h1>' > /var/www/html/index.html \
    && rm -rf /var/cache/apk/*
COPY nginx.conf /etc/nginx/nginx.conf
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
```

**Corten.toml (Corten):**
```toml
[image]
name = "my-app"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["nginx", "php83", "php83-fpm"]

[files]
copy = [
    { src = "nginx.conf", dest = "/etc/nginx/nginx.conf" },
]

[setup]
run = [
    "mkdir -p /run/nginx /var/www/html",
    "echo '<h1>Hello!</h1>' > /var/www/html/index.html",
]

[container]
command = ["nginx", "-g", "daemon off;"]
expose = [80]
```

### Why Corten.toml is Better

| | Dockerfile | Corten.toml |
|---|---|---|
| **Format** | Shell script disguised as config | Structured data (TOML or JSONC) |
| **Package install** | `RUN apk add --no-cache ...` | `install = ["nginx", "php"]` |
| **Layer optimization** | Manual (`&&` chaining, multi-stage) | Automatic (packages → files → setup → cleanup) |
| **Cache cleanup** | You must remember `rm -rf /var/cache/apk/*` | Automatic after package install |
| **File copying** | `COPY src dest` (no ownership control) | `{ src, dest, owner }` |
| **Validation** | Fails at build time | Validated before build starts (`--dry-run`) |
| **Comments** | `#` only | TOML `#` or JSONC `//` and `/* */` |
| **Parseable** | No (it's shell) | Yes (TOML/JSON — any tool can read it) |
| **IDE support** | Basic | Full schema validation possible |

### Three Format Options

Corten supports TOML, JSON, and JSONC. Use whichever your team prefers:

**TOML** (recommended — clean, readable):
```toml
# Corten.toml
[image]
name = "my-app"
tag = "latest"

[base]
system = "alpine"
version = "3.20"

[packages]
install = ["nginx"]

[container]
command = ["nginx", "-g", "daemon off;"]
```

**JSONC** (JSON with Comments — familiar to JS/TS devs):
```jsonc
{
  // Corten.jsonc
  "image": { "name": "my-app", "tag": "latest" },
  "base": { "system": "alpine", "version": "3.20" },

  // Packages are auto-installed via the distro's package manager
  "packages": { "install": ["nginx"] },

  "container": {
    "command": ["nginx", "-g", "daemon off;"]
  }
}
```

**JSON** (strict — for automation/CI):
```json
{
  "image": { "name": "my-app", "tag": "latest" },
  "base": { "system": "alpine", "version": "3.20" },
  "packages": { "install": ["nginx"] }
}
```

Auto-detected by file extension: `.toml`, `.jsonc`, `.json`.

```bash
corten build .                    # Looks for Corten.toml, .jsonc, .json
corten build my-app.jsonc         # Explicit file
corten build --dry-run .          # Preview without building
```

## Multi-Container Orchestration (Forge)

Docker Compose uses YAML — indentation-sensitive, type-ambiguous (`8080:80` — string or time?), and the [Norway problem](https://hitchdev.com/strictyaml/why/implicit-typing-removed/) (`NO` becomes `false`).

Corten Forge uses **TOML or JSONC** — no indentation games, explicit types, proper comments.

**docker-compose.yml (Docker):**
```yaml
services:
  api:
    image: my-api
    ports:
      - "8080:80"        # Is this a string? A number? Who knows
    depends_on:
      - db               # One wrong space = broken
    deploy:
      resources:
        limits:
          memory: 256M    # Why so deeply nested?
    environment:
      - DB_HOST=db        # A list of strings, not a map?

  db:
    image: my-db
```

**Cortenforge.toml (Corten):**
```toml
[services.api]
image = "my-api"
ports = ["8080:80"]
depends_on = ["db"]
memory = "256m"              # Flat, not deploy.resources.limits.memory

[services.api.env]
DB_HOST = "db"               # Proper key-value map

[services.db]
image = "my-db"
memory = "512m"
```

**Cortenforge.jsonc** (same thing, for JSON fans):
```jsonc
{
  "services": {
    "api": {
      "image": "my-api",
      "ports": ["8080:80"],
      "depends_on": ["db"],
      "memory": "256m",
      // Environment as a real map, not a list of "KEY=VALUE" strings
      "env": { "DB_HOST": "db" }
    },
    "db": {
      "image": "my-db",
      "memory": "512m"
    }
  }
}
```

```bash
corten forge up       # Start in dependency order
corten forge ps       # List services
corten forge logs     # View output
corten forge down     # Stop and clean up
```

## Per-User Container Isolation

A security feature neither Docker nor Podman offers at this level.

### The Docker Problem

Docker's `docker` group is essentially **root access**. Any user in the group can see, stop, exec into, and read logs of every other user's containers — and even escape to the host filesystem via volume mounts.

```bash
# In Docker: alice can mess with bob's containers
alice$ docker ps              # See ALL containers (bob's too)
alice$ docker exec bob-db sh  # Shell into bob's database
alice$ docker logs bob-app    # Read bob's secrets
alice$ docker stop bob-app    # Kill bob's production app
```

### The Corten Solution

Each user gets their own isolated container directory. The kernel UID (from `getuid()`) determines which containers you can access — it cannot be spoofed.

```
/var/lib/corten/
  images/                    # Shared read-only (all users)
    alpine/latest/rootfs/
    my-nginx/latest/rootfs/
  users/
    1000/                    # jakub (uid 1000)
      containers/
        abc123/              # jakub's web server
        def456/              # jakub's database
    1001/                    # alice (uid 1001)
      containers/
        789xyz/              # alice's API — jakub can't see this
```

**What happens when alice tries to access jakub's container:**

```bash
jakub$ corten run -d --name mydb alpine sleep 3600  # Stored under users/1000/
alice$ corten ps           # Searches users/1001/ → empty, mydb not visible
alice$ corten stop mydb    # "container not found"
alice$ corten exec mydb sh # "container not found"
alice$ corten logs mydb    # "container not found"
```

Alice cannot even **discover** that jakub's container exists.

### Security Layers

| Layer | What it does | How it works |
|-------|-------------|--------------|
| **Group gate** | Controls who can run corten at all | `corten` group membership checked against real `getuid()` |
| **Per-user paths** | Isolates container storage | Every operation scoped to `/var/lib/corten/users/<uid>/` |
| **Shared images** | Efficient storage | Images at `/var/lib/corten/images/` — pull once, used by everyone via OverlayFS |

### What's Protected

| Operation | Docker | Corten |
|-----------|--------|--------|
| `ps` — list containers | Sees ALL users' containers | **Only own** |
| `stop` — stop container | Can stop anyone's | **Only own** |
| `rm` — remove container | Can remove anyone's | **Only own** |
| `exec` — shell into | Can exec into anyone's | **Only own** |
| `logs` — read output | Can read anyone's | **Only own** |
| `inspect` — view config | Can inspect anyone's | **Only own** |
| `images` — list images | Shared | Shared |
| `pull` — download image | Shared | Shared |

### Setup

```bash
# Create the corten group (done automatically by make install)
sudo groupadd corten

# Add users who should be allowed to run containers
sudo usermod -aG corten jakub
sudo usermod -aG corten alice

# Users must log out and back in for group change to take effect
```

### Verified by Tests

8 end-to-end tests confirm the isolation:

```
test user_a_cannot_see_user_b_containers ........ ok
test user_cannot_rm_other_users_container ........ ok
test user_cannot_stop_other_users_container ...... ok
test user_cannot_inspect_other_users_container ... ok
test user_cannot_exec_into_other_users_container . ok
test user_cannot_read_other_users_logs ........... ok
test ps_only_shows_own_containers ................ ok
test shared_images_work_for_all_users ............ ok
```

## Image Sources (No Docker Hub)

Corten pulls directly from official distro mirrors:

| Distro | Source                       |
|--------|------------------------------|
| Alpine | dl-cdn.alpinelinux.org       |
| Ubuntu | cloud-images.ubuntu.com      |
| Debian | deb.debian.org (debootstrap) |
| Fedora | kojipkgs.fedoraproject.org   |
| Arch   | geo.mirror.pkgbuild.com      |
| Void   | repo-default.voidlinux.org   |

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
