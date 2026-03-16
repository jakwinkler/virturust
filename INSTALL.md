# Installing Corten

## Prerequisites

**Operating System:** Linux (kernel 5.x or later)

**Required:**
- Rust toolchain (1.85+)
- `iproute2` — for networking (`ip` command)
- `iptables` — for NAT and port forwarding

**Optional:**
- `dnsmasq` — for named network DNS resolution
- `ab` (Apache Bench) — for running benchmarks
- `redis-cli` / `pgbench` / `mysql` — for database benchmarks

### Install on Fedora / RHEL / CentOS

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# System packages
sudo dnf install -y iproute iptables dnsmasq
```

### Install on Ubuntu / Debian

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# System packages
sudo apt install -y iproute2 iptables dnsmasq
```

### Install on Arch Linux

```bash
# Rust
sudo pacman -S rustup
rustup default stable

# System packages
sudo pacman -S iproute2 iptables dnsmasq
```

## Build

```bash
git clone https://github.com/jakwinkler/virturust.git
cd virturust

# Build release binary
make build
# or: cargo build --release

# The binary is at: target/release/corten
```

## Install

```bash
make install
```

This does four things (requires one-time sudo/root):

1. **Copies the binary** to `/usr/local/bin/corten`
2. **Sets Linux capabilities** — so corten can create namespaces, bridges, cgroups without sudo
3. **Creates data directories** — `/var/lib/corten/images`, `/var/lib/corten/containers`
4. **Sets up cgroup delegation** — so corten can manage container resource limits
5. **Creates the `corten` group** — for multi-user access control

After installation, **you never need sudo again**:

```bash
corten pull alpine
corten run alpine echo "no sudo!"
```

## Verify Installation

```bash
# Check version
corten --version

# Check capabilities are set
getcap /usr/local/bin/corten
# Should show: cap_sys_admin,cap_net_admin,cap_sys_chroot,...

# Check cgroups v2
stat -f -c %T /sys/fs/cgroup
# Should show: cgroup2fs

# Test a container
corten pull alpine
corten run --rm alpine echo "corten works!"
```

## Multi-User Setup

To allow other users to run containers:

```bash
# Add user to corten group
sudo usermod -aG corten alice

# User must log out and back in for group change to take effect
su alice -c "corten ps"   # Should work
```

Each user gets isolated containers — User A cannot see or manage User B's containers.

## Custom Installation

### Change install location

```bash
make install PREFIX=/opt/corten
```

### Change data directory

```bash
export CORTEN_DATA_DIR=/data/containers
corten run alpine echo "custom data dir"
```

### Uninstall

```bash
sudo rm /usr/local/bin/corten
sudo rm -rf /var/lib/corten
sudo groupdel corten
```

## Updating

```bash
cd virturust
git pull
make install
```

## Troubleshooting

### "insufficient privileges"

The binary needs Linux capabilities. Run `make install` or:

```bash
sudo setcap 'cap_sys_admin,cap_net_admin,cap_sys_chroot,cap_dac_override,cap_fowner,cap_chown,cap_setuid,cap_setgid,cap_mknod+eip' /usr/local/bin/corten
```

### "Permission denied: user is not in the 'corten' group"

```bash
sudo usermod -aG corten $(whoami)
# Log out and back in
```

### "cgroup2fs not found"

Your system uses cgroups v1. Enable v2:

```bash
# Fedora/RHEL
sudo grubby --update-kernel=ALL --args="systemd.unified_cgroup_hierarchy=1"
sudo reboot

# Ubuntu
sudo nano /etc/default/grub
# Add: GRUB_CMDLINE_LINUX="systemd.unified_cgroup_hierarchy=1"
sudo update-grub && sudo reboot
```

### Bridge networking doesn't work with Docker

Docker's FORWARD chain has policy DROP. Corten adds rules to the DOCKER-USER chain automatically. If it still fails:

```bash
sudo iptables -I DOCKER-USER -s 10.0.42.0/24 -j ACCEPT
sudo iptables -I DOCKER-USER -d 10.0.42.0/24 -j ACCEPT
```

### Port forwarding doesn't work from localhost

Corten enables `route_localnet` automatically. If it doesn't work:

```bash
sudo sysctl -w net.ipv4.conf.all.route_localnet=1
```
