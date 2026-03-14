# Corten Feature Map

## Container Runtime

### Execution
- [x] `corten run <image> [cmd]` — run a container
- [x] `corten run -d` — detached mode (background)
- [ ] `corten run -it` — interactive TTY mode
- [x] `corten run --name` — named container
- [x] `corten run --hostname` — custom hostname
- [ ] `corten run --rm` — auto-remove on exit
- [ ] `corten run --privileged` — disable security restrictions
- [ ] `corten run --read-only` — read-only root filesystem
- [ ] `corten run --init` — PID 1 init process (signal reaping)
- [ ] `corten run --pull always|missing|never` — pull policy
- [ ] `corten run --entrypoint` — override entrypoint
- [ ] `corten run --stop-signal` — custom stop signal
- [ ] `corten run --stop-timeout` — custom stop timeout
- [ ] `corten run --cidfile` — write container ID to file

### Environment
- [x] Image ENV/CMD/ENTRYPOINT/WORKDIR/USER applied
- [ ] `corten run -e KEY=VALUE` — runtime env vars
- [ ] `corten run --env-file` — env vars from file
- [x] `corten run -w/--workdir` — (from image config)
- [x] `corten run -u/--user` — (from image config)

### Resource Limits
- [x] `--memory` — hard memory limit (cgroups v2)
- [x] `--cpus` — CPU quota (CFS bandwidth)
- [x] `--pids-limit` — max processes
- [ ] `--memory-reservation` — soft memory limit
- [ ] `--memory-swap` — swap limit
- [ ] `--cpu-shares` — relative CPU weight
- [ ] `--cpuset-cpus` — pin to specific CPUs
- [ ] `--shm-size` — /dev/shm size
- [ ] `--ulimit` — set ulimits
- [ ] `--device` — add host device

### Volumes & Mounts
- [x] `-v /host:/container` — bind mount
- [x] `-v /host:/container:ro` — read-only bind mount
- [ ] `--mount type=bind,src=,dst=` — advanced mount syntax
- [ ] `--mount type=tmpfs,dst=` — tmpfs mount
- [ ] `--volumes-from` — mount from another container
- [ ] Named volumes (`-v name:/path`)

### Networking
- [x] `--network bridge` — default bridge with NAT
- [x] `--network none` — no networking
- [x] `--network host` — share host network
- [x] `-p host:container` — port forwarding (DNAT)
- [x] Named networks with DNS resolution
- [ ] `-P` — publish all exposed ports
- [ ] `--dns` — custom DNS servers
- [ ] `--add-host` — add /etc/hosts entry
- [ ] `--network container:name` — share another container's network
- [ ] `--mac-address` — custom MAC
- [ ] `--ip` — static IP assignment

### Security
- [x] Capability dropping (Docker-compatible default set)
- [x] Seccomp-BPF (15 blocked syscalls)
- [x] Masked paths (/proc/kcore, /proc/keys, etc.)
- [x] Read-only /proc subsystems
- [x] Minimal /dev (no host device access)
- [x] Rootless mode (`--rootless`)
- [ ] `--cap-add` / `--cap-drop` — per-container capability control
- [ ] `--security-opt seccomp=<profile>` — custom seccomp profile
- [ ] `--security-opt apparmor=<profile>` — AppArmor
- [ ] `--privileged` — disable all restrictions

## Container Lifecycle

### Management
- [x] `corten ps` — list containers
- [x] `corten inspect <name>` — container details
- [x] `corten stop <name>` — graceful stop (SIGTERM → SIGKILL)
- [x] `corten stop --time` — custom grace period
- [x] `corten rm <name>` — remove stopped container
- [x] `corten exec <name> <cmd>` — exec into running container
- [x] `corten logs <name>` — view logs
- [x] `corten logs -f` — follow/stream logs
- [x] `corten logs -n` — tail last N lines
- [ ] `corten start <name>` — start stopped container
- [ ] `corten restart <name>` — restart container
- [ ] `corten kill <name>` — send signal
- [ ] `corten kill --signal` — custom signal
- [ ] `corten pause/unpause` — freeze/resume
- [ ] `corten cp <name>:/path /host` — copy files
- [ ] `corten stats` — live resource usage
- [ ] `corten top <name>` — running processes
- [ ] `corten wait <name>` — wait for exit
- [ ] `corten rename` — rename container
- [ ] `corten rm -f` — force remove running container
- [ ] `corten ps -a` — show all (including stopped)
- [ ] `corten ps -q` — quiet (IDs only)
- [ ] `corten ps --format` — custom output format
- [ ] `corten inspect --format` — Go-template formatting
- [ ] Signal forwarding to container PID 1

### Restart Policies
- [x] `--restart no` — never restart
- [x] `--restart always` — always restart
- [x] `--restart on-failure:N` — restart on failure with max retries
- [ ] `--restart unless-stopped` — restart unless manually stopped
- [ ] Exponential backoff (100ms → 60s, reset after 10s running)

### Health Checks
- [ ] `--health-cmd` — health check command
- [ ] `--health-interval` — check interval
- [ ] `--health-timeout` — check timeout
- [ ] `--health-retries` — retries before unhealthy
- [ ] `--health-start-period` — grace period
- [ ] `--no-healthcheck` — disable
- [ ] Health states: starting → healthy → unhealthy

## Images

### Management
- [x] `corten pull <distro>` — pull from official distro mirrors
- [x] `corten images` — list local images
- [x] `corten image prune` — remove all images
- [ ] `corten rmi <image>` — remove specific image
- [ ] `corten tag <image> <new>` — tag image
- [ ] `corten save <image> > file.tar` — export image
- [ ] `corten load < file.tar` — import image
- [ ] `corten history <image>` — show build history

### Supported Pull Sources (no Docker Hub)
- [x] Alpine — dl-cdn.alpinelinux.org
- [x] Ubuntu — cloud-images.ubuntu.com
- [x] Debian — debootstrap
- [x] Fedora — kojipkgs.fedoraproject.org
- [x] Arch — geo.mirror.pkgbuild.com
- [x] Void — repo-default.voidlinux.org
- [ ] Gentoo — distfiles.gentoo.org
- [ ] Amazon Linux
- [ ] Rocky/Alma Linux

## Build System

### Corten.toml
- [x] Parse `[base]` — OS + version
- [x] Parse `[packages]` — package list
- [x] Parse `[files]` — file copy operations
- [x] Parse `[env]` — environment variables
- [x] Parse `[setup]` — shell commands
- [x] Parse `[container]` — runtime defaults (cmd, user, workdir)
- [x] `corten build <path>` — build image
- [x] `corten build --dry-run` — preview build plan
- [x] Auto-detect package manager (apt/apk/dnf/pacman/zypper)
- [x] Alpine bootstrap (minirootfs download)
- [x] Ubuntu/Debian bootstrap (debootstrap)
- [x] Package installation via chroot
- [x] File copying with ownership
- [x] Setup command execution in chroot
- [x] Automatic package cache cleanup
- [ ] Build caching (skip rebuild if unchanged)
- [ ] Multi-stage builds
- [ ] Build args (`--build-arg KEY=VALUE`)

## Networks

- [x] `corten network create <name>` — create named network
- [x] `corten network ls` — list networks
- [x] `corten network rm <name>` — remove network
- [ ] `corten network inspect <name>` — show details
- [ ] `corten network connect <net> <container>` — connect running container
- [ ] `corten network disconnect <net> <container>` — disconnect container
- [ ] Custom subnet (`--subnet`)
- [ ] Custom gateway (`--gateway`)
- [ ] Internal networks (`--internal`)

## System

- [x] `corten system prune` — remove stopped containers + images
- [x] `corten image prune` — remove all images
- [ ] `corten info` — system info (kernel, cgroups, storage, arch)
- [ ] `corten version` — detailed version info
- [ ] `corten events` — stream container events

## Compose (corten compose)

- [ ] `corten compose up` — start services from YAML
- [ ] `corten compose down` — stop and remove services
- [ ] `corten compose ps` — list services
- [ ] `corten compose logs` — view service logs
- [ ] `corten compose exec` — exec into service
- [ ] `corten compose build` — build service images
- [ ] `corten compose stop/start/restart` — lifecycle
- [ ] Service dependencies (`depends_on`)
- [ ] Health-based dependencies (`condition: service_healthy`)
- [ ] Auto-create networks per project
- [ ] Auto-create volumes per project
- [ ] Scale services (`--scale service=N`)
- [ ] Environment variable interpolation
- [ ] `.env` file support

## Output & UX

- [ ] `--format` flag for ps/inspect/stats (Go-template style)
- [ ] JSON output mode
- [ ] Progress bars for pull/build
- [ ] Shell completion (bash/zsh/fish)
- [ ] `~/.corten/config.toml` — user config
- [ ] Meaningful exit codes (125/126/127)
- [ ] Colored output
- [ ] Detach key sequence (Ctrl+P,Ctrl+Q)
