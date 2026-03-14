# Corten Architecture

> Corten (named after corten/weathering steel — naturally rusted, tough, low-maintenance).
> Less weight, more strength.

## Overview

Corten is a lightweight container runtime written in Rust. It aims to be a credible
alternative to Docker's lower stack (runc + containerd) with a single binary, no daemon,
and a simpler mental model.

```
Docker stack:              Corten:
  docker CLI                 corten CLI
  dockerd (daemon)           (none — no daemon)
  containerd                 (none — no shim)
  runc  <--------------->    corten (equivalent layer)
  Linux kernel               Linux kernel
```

Corten is not trying to replace the entire Docker ecosystem. It targets the OCI runtime
layer and adds its own build system and image format on top.

---

## Product Modes

Corten has two planned execution modes under one CLI:

| Mode | What it runs | Isolation | Status |
|------|-------------|-----------|--------|
| **Containers** | OCI/rootfs processes | Namespaces + cgroups | Active development |
| **Machines** | Bootable disk images (qcow2/raw) | KVM/QEMU | Future |

For v1, only container mode exists. The CLI uses flat commands (`corten run`, `corten pull`).
When machine mode ships, the CLI will namespace them: `corten container run` / `corten machine run`.

---

## Container Runtime Architecture

```
                    ┌───────────────────────────────────┐
                    │          CLI (clap)               │
                    ├───────────────────────────────────┤
                    │       Container Manager           │
                    │    (lifecycle orchestration)      │
                    ├────────┬────────┬────────┬────────┤
                    │  NS    │cgroups │  FS    │  Net   │
                    │(clone) │ (v2)   │(pivot) │(veth)  │
                    ├────────┴────────┴────────┴────────┤
                    │        Image Manager              │
                    │   OCI (Docker Hub) + Native       │
                    ├───────────────────────────────────┤
                    │         Linux Kernel              │
                    └───────────────────────────────────┘
```

### Execution flow: `corten run`

```
1. Parse CLI args + validate
2. Resolve image (pull from Docker Hub if not cached)
3. Prepare rootfs:
   - OCI: extract layers with whiteout handling → rootfs
   - Native: mount SquashFS → rootfs
4. Set up OverlayFS (lower=image rootfs, upper=per-container writable layer)
5. Create cgroup at /sys/fs/cgroup/corten/<id>/, apply resource limits
6. clone() with CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET
7. Parent: add child to cgroup, set up veth pair + bridge, signal child via pipe
8. Child:
   a. Wait for parent signal
   b. Set hostname
   c. Mount /proc, /sys, minimal /dev
   d. Mount volumes (bind mounts)
   e. pivot_root to container rootfs
   f. Apply OCI config (env, workdir, user)
   g. Drop capabilities, apply seccomp filter
   h. execvp() the command
9. Parent: wait for child exit, cleanup cgroup + network + overlay
```

---

## Image Strategy: Dual Format

Corten supports two image formats. OCI for ecosystem compatibility, SquashFS for
its own native workflow.

```
OCI path:     pull layers → verify digests → extract tar.gz → apply whiteouts → rootfs → overlayfs → pivot_root
Native path:  mount squashfs → rootfs → overlayfs → pivot_root  (faster, no extraction)
```

### OCI images (Docker Hub compatibility)

Standard OCI/Docker v2 images. Pull from any OCI registry. Layer-by-layer extraction
with proper whiteout handling. This is the "day 1" experience — users can
`corten pull nginx` and have it work immediately.

### Native images (SquashFS)

Corten's own format. A single compressed, read-only filesystem image.

Advantages over OCI layers:
- Build however you want (shell scripts, Ansible, Nix, manual chroot)
- No layer cache invalidation
- Smaller images (SquashFS compression)
- Faster startup (mount directly, no layer unpacking)
- Simpler mental model (an image is just a filesystem snapshot)

### Image source resolution

```rust
enum ImageSource {
    Oci { name: String, tag: String },       // from Docker Hub / OCI registry
    Native { name: String, tag: String },     // local SquashFS
}
```

The runtime doesn't care about the source — both end up as a rootfs path
passed to OverlayFS and then pivot_root.

---

## Build System: Corten.toml

Declarative image definition replacing Dockerfile.

```toml
[image]
name = "my-app"
tag = "1.0"

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

[env]
APP_ENV = "production"

[setup]
run = ["useradd -r appuser"]

[container]
command = ["nginx", "-g", "daemon off;"]
user = "appuser"
workdir = "/var/www/html"
```

### What Corten handles automatically

| Docker makes you do | Corten does for you |
|---|---|
| `apt-get update && apt-get install -y` | Auto-detects package manager, runs update + install |
| `rm -rf /var/lib/apt/lists/*` | Auto-cleans package cache before packing |
| `ENV DEBIAN_FRONTEND=noninteractive` | Set automatically during build |
| `&&` chaining in RUN commands | No layers — commands run independently |
| Shell form vs exec form | Always exec form |

### Package manager auto-detection

```
ubuntu/debian  → apt
alpine         → apk
fedora/rhel    → dnf
arch           → pacman
```

### Build flow

```
corten build .
  1. Bootstrap base OS rootfs (debootstrap / apk --root / dnf --installroot)
  2. Install packages (auto-detected package manager)
  3. Copy files + set ownership
  4. Run setup commands (if any)
  5. Clean package cache (automatic)
  6. Set env vars
  7. Pack into SquashFS
  → myapp-1.0.squashfs
```

---

## Filesystem Isolation

### OverlayFS (planned)

Every container gets its own writable layer on top of the read-only image rootfs:

```
┌─────────────────────┐
│ merged (container /) │  ← what the container sees
├─────────────────────┤
│ upper (writable)     │  ← per-container, discarded on rm
├─────────────────────┤
│ lower (image rootfs) │  ← shared, read-only
└─────────────────────┘
```

This prevents containers from mutating cached images and enables efficient
layer sharing between containers using the same image.

### Minimal /dev

Instead of bind-mounting the host `/dev` (security hole), Corten creates a
minimal `/dev` with only the devices containers need:

| Device | Purpose |
|--------|---------|
| `/dev/null` | Discard output |
| `/dev/zero` | Zero bytes source |
| `/dev/random` | Random bytes (blocking) |
| `/dev/urandom` | Random bytes (non-blocking) |
| `/dev/tty` | Controlling terminal |
| `/dev/console` | Console device |
| `/dev/ptmx` | PTY master |
| `/dev/pts/` | PTY slave directory |

### Volume mounts

```
corten run -v /host/path:/container/path alpine ls /container/path
corten run -v /host/path:/container/path:ro alpine cat /container/path/file
```

Volumes are bind-mounted into the container rootfs after OverlayFS setup
but before `pivot_root`. The `:ro` suffix triggers a remount with `MS_RDONLY`.

---

## Networking Architecture (planned)

### Default bridge network

Every container connects to the `corten0` bridge by default. Containers on
the same bridge can communicate directly by IP.

```
Host:
  ┌───────────────────────────────┐
  │          corten0               │  bridge (10.0.42.1/24)
  │          (bridge)              │
  └──┬────────┬────────┬─────────┘
     │        │        │
  veth-host  veth-host  veth-host     host-side veth endpoints
     │        │        │
  ───┼────────┼────────┼──────────── (namespace boundary)
     │        │        │
  eth0      eth0      eth0            container-side endpoints
     │        │        │
  ┌──┴──┐  ┌─┴───┐  ┌─┴───┐
  │ct-1 │  │ct-2 │  │ct-3 │          containers (10.0.42.x/24)
  │ .2  │  │ .3  │  │ .4  │
  └─────┘  └─────┘  └─────┘
```

- **Bridge**: `corten0` created on first container run, destroyed when last container stops
- **veth pairs**: One end in host namespace (attached to bridge), other in container namespace
- **IP allocation**: File-based allocator, sequential from 10.0.42.2-254
- **NAT**: iptables MASQUERADE on the bridge interface for outbound traffic
- **Port forwarding**: `-p 8080:80` via iptables DNAT rules
- **DNS**: Inject host nameservers into container `/etc/resolv.conf`

### Named networks

Named networks provide isolated subnets and DNS-based service discovery.
Each named network gets its own bridge and subnet.

```bash
corten network create backend              # creates corten-backend bridge
corten run --name api --network backend     # joins the backend network
corten run --name db  --network backend     # joins the backend network
# api can: ping db (resolved via built-in DNS)
# api cannot: reach containers on the default bridge (isolated)
```

```
  ┌──────────────┐     ┌──────────────────┐
  │   corten0     │     │ corten-backend    │
  │ 10.0.42.0/24  │     │ 10.0.43.0/24     │
  └──┬────────┬──┘     └──┬────────┬──────┘
     │        │           │        │
  ┌──┴──┐  ┌─┴───┐    ┌──┴──┐  ┌─┴───┐
  │web  │  │worker│    │api  │  │db   │
  └─────┘  └─────┘    └─────┘  └─────┘
  (default bridge)      (backend network)
```

### Network modes

| Mode | Flag | Behavior |
|------|------|----------|
| **bridge** (default) | `--network bridge` | Container gets veth + bridge + NAT |
| **none** | `--network none` | Loopback only, no external connectivity |
| **host** | `--network host` | Share host network namespace (no isolation) |
| **named** | `--network <name>` | Join a named network with DNS discovery |

### DNS service discovery

Containers on the same named network can resolve each other by name.
A lightweight DNS resolver runs on 127.0.0.11 inside each container
and intercepts name lookups.

```
container "api" looks up "db"
  → /etc/resolv.conf points to 127.0.0.11
  → built-in resolver checks container registry for "db" on same network
  → returns 10.0.43.3
  → falls through to host DNS for external names
```

### Network module structure

```
src/network/
  mod.rs          Network mode selection + container IP management
  bridge.rs       Bridge creation/deletion, veth pair setup
  nat.rs          iptables MASQUERADE + DNAT rules
  dns.rs          resolv.conf injection + built-in DNS resolver
  named.rs        Named network CRUD + subnet allocation
```

---

## Security Model (planned)

### Defense in depth

| Layer | Mechanism | What it prevents |
|-------|-----------|------------------|
| 1 | Namespaces | Process, mount, network, UTS, IPC isolation |
| 2 | Rootless mode (user NS) | No root on host |
| 3 | Capability dropping | Only keep needed capabilities |
| 4 | Seccomp-BPF | Block dangerous syscalls |
| 5 | AppArmor/SELinux | MAC policies |
| 6 | Read-only rootfs | Prevent filesystem tampering |
| 7 | Minimal /dev | No access to host devices |
| 8 | Masked paths | Hide sensitive procfs/sysfs entries |

### Seccomp default profile

Block syscalls that containers should never need:
- `kexec_load`, `reboot`, `swapon/swapoff`
- `mount`, `umount` (outside setup)
- `clock_settime`, `settimeofday`
- `init_module`, `finit_module`, `delete_module`
- `acct`, `quotactl`, `pivot_root` (outside setup)

---

## Module Structure

### Current

```
src/
  main.rs          CLI entry point
  lib.rs           Library root
  cli.rs           Argument definitions (clap derive)
  config.rs        Types + parsing (ContainerConfig, ResourceLimits)
  container.rs     Lifecycle management (run, stop, state persistence)
  namespace.rs     clone() + child process initialization
  filesystem.rs    Mount setup + pivot_root
  cgroup.rs        cgroups v2 resource limits
  image.rs         OCI image pulling from Docker Hub
  network.rs       Network namespace setup (loopback only)
```

### Target

```
src/
  main.rs
  lib.rs
  cli.rs
  config.rs
  container.rs
  namespace.rs
  cgroup.rs
  network/
    mod.rs          Bridge + veth management
    nat.rs          iptables NAT rules
    dns.rs          resolv.conf injection
  filesystem/
    mod.rs          Mount orchestration
    overlay.rs      OverlayFS setup
    dev.rs          Minimal /dev creation
    volumes.rs      Bind mount handling
    pivot.rs        pivot_root
  image/
    mod.rs          Unified ImageSource enum
    oci.rs          OCI pull + whiteout handling
    native.rs       SquashFS mount/build
    convert.rs      OCI <-> SquashFS conversion
  build/
    mod.rs          Corten.toml → SquashFS pipeline
    toml.rs         Corten.toml parser
    bootstrap.rs    Base OS rootfs creation
    package.rs      Package manager auto-detection
  security/
    mod.rs          Security orchestration
    seccomp.rs      Seccomp-BPF filter
    caps.rs         Capability dropping
```

---

## Storage Layout

```
/var/lib/corten/
  images/
    oci/
      alpine/latest/rootfs/          # extracted OCI layers
      nginx/1.27/rootfs/
    native/
      myapp/1.0.squashfs             # SquashFS images
  containers/
    <uuid>/
      config.json                    # ContainerConfig
      state.json                     # ContainerState
      overlay/
        upper/                       # writable layer
        work/                        # OverlayFS workdir
        merged/                      # union mount (rootfs)
```

---

## Competitive Positioning

### Where Corten wins

- **Startup latency** — no daemon round-trip; `clone()` → `execvp()` directly
- **Memory footprint** — no long-running daemon (Docker: ~100-200MB)
- **Simplicity** — single binary, ~2k lines of Rust vs Docker's multi-project Go ecosystem
- **Memory safety without GC** — Rust guarantees at compile time
- **Build UX** — Corten.toml is declarative and obvious vs Dockerfile footguns
- **Image efficiency** — SquashFS native format is smaller and faster than layered tar.gz

### Where Docker wins (for now)

- Ecosystem (Compose, registries, BuildKit, Swarm, Kubernetes CRI)
- Production maturity at massive scale
- Full networking stack
- Rootless mode
- Security hardening (seccomp, AppArmor)

### Target niches

1. **OCI-compliant runtime** — drop-in runc replacement for containerd/Kubernetes
2. **Edge/embedded/serverless** — where startup latency and memory footprint matter
3. **Security-critical workloads** — Rust memory safety vs Go's runc (CVE-2019-5736)
4. **Developer experience** — simpler build + run workflow than Docker

---

## Naming

Corten is the working project name. Before any public release, a trademark search
is needed — "COR-TEN" is an existing trademark in the steel/shipping industry.
