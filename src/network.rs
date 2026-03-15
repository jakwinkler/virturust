//! Network namespace setup for containers.
//!
//! Each container is created in its own network namespace (`CLONE_NEWNET`),
//! which gives it a completely isolated network stack: its own interfaces,
//! routing table, iptables rules, and sockets.
//!
//! ## Capabilities
//!
//! - Loopback interface (`lo`) brought up inside the container
//! - Bridge networking (`corten0` bridge at 10.0.42.1/24)
//! - veth pair creation (host <-> container link)
//! - NAT/masquerade for outbound connectivity
//! - DNS forwarding (host resolv.conf copied into container)
//! - IP allocation via file-based allocator

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Subnet used for the container bridge network.
const BRIDGE_SUBNET: &str = "10.0.42.0/24";

/// IP address assigned to the bridge interface on the host side.
const BRIDGE_IP: &str = "10.0.42.1";

/// CIDR notation for the bridge IP.
const BRIDGE_IP_CIDR: &str = "10.0.42.1/24";

/// Name of the bridge interface.
const BRIDGE_NAME: &str = "corten0";

/// Directory where the IP allocator stores its state.
const NETWORK_STATE_DIR: &str = "/var/lib/corten/network";

/// File that tracks the next IP octet to allocate.
const NEXT_IP_FILE: &str = "/var/lib/corten/network/next_ip";

/// Network configuration for a running container.
pub struct ContainerNetwork {
    /// The IP address assigned to the container (e.g., "10.0.42.2")
    pub ip: String,
    /// The bridge interface name (e.g., "corten0")
    pub bridge: String,
    /// The host-side veth interface name (e.g., "veth-abcd1234")
    pub veth_host: String,
}

/// Bring up the loopback interface inside the container's network namespace.
///
/// Without this, even `localhost` / `127.0.0.1` won't work inside the
/// container. The loopback interface exists in every network namespace
/// by default, but starts in the DOWN state.
pub fn setup_loopback() -> Result<()> {
    // Loopback runs inside the container namespace (child process)
    // which is already root, so no pre_exec needed here
    let output = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .output()
        .context("failed to run 'ip link set lo up' — is iproute2 installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("failed to bring up loopback: {stderr}");
    }

    Ok(())
}

/// Ensure the `corten0` bridge interface exists and is configured.
///
/// Creates the bridge if it doesn't already exist, assigns 10.0.42.1/24,
/// brings it up, and enables IP forwarding on the host.
pub fn ensure_bridge() -> Result<()> {
    // Check if bridge already exists
    let output = root_cmd("ip")
        .args(["link", "show", BRIDGE_NAME])
        .output()
        .context("failed to check for bridge interface")?;

    if !output.status.success() {
        // Bridge does not exist — create it
        log::info!("creating bridge interface {BRIDGE_NAME}");

        run_cmd("ip", &["link", "add", BRIDGE_NAME, "type", "bridge"])
            .context("failed to create bridge interface")?;

        run_cmd("ip", &["addr", "add", BRIDGE_IP_CIDR, "dev", BRIDGE_NAME])
            .context("failed to assign IP to bridge")?;

        run_cmd("ip", &["link", "set", BRIDGE_NAME, "up"])
            .context("failed to bring up bridge")?;

        log::info!("bridge {BRIDGE_NAME} created with IP {BRIDGE_IP_CIDR}");
    } else {
        log::info!("bridge {BRIDGE_NAME} already exists");
    }

    // Enable IP forwarding
    fs::write("/proc/sys/net/ipv4/ip_forward", "1")
        .context("failed to enable IP forwarding")?;

    // Enable routing of loopback traffic through NAT.
    // Without this, localhost DNAT (curl 127.0.0.1:port -> container) is
    // silently dropped as "martian" by the kernel. Docker uses docker-proxy
    // to work around this; we use route_localnet instead.
    fs::write("/proc/sys/net/ipv4/conf/all/route_localnet", "1").ok();

    log::info!("IP forwarding enabled");

    // On systems with firewalld (Fedora, RHEL, CentOS), add the bridge
    // to the trusted zone so container traffic isn't blocked by the firewall.
    if root_cmd("firewall-cmd").arg("--state").output()
        .map(|o| o.status.success()).unwrap_or(false)
    {
        run_cmd("firewall-cmd", &[
            "--zone=trusted",
            &format!("--add-interface={BRIDGE_NAME}"),
        ]).ok();

        run_cmd("firewall-cmd", &["--zone=trusted", "--add-masquerade"]).ok();

        log::info!("added {BRIDGE_NAME} to firewalld trusted zone");
    }

    // If Docker is running, its FORWARD chain has policy DROP and routes
    // everything through DOCKER-USER first. Insert our ACCEPT rules there
    // so Corten bridge traffic isn't blocked by Docker's firewall.
    let docker_user_exists = root_cmd("iptables")
        .args(["-L", "DOCKER-USER", "-n"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if docker_user_exists {
        // Check if rules already exist before inserting
        let already_has_rule = root_cmd("iptables")
            .args(["-C", "DOCKER-USER", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !already_has_rule {
            run_cmd("iptables", &["-I", "DOCKER-USER", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
            run_cmd("iptables", &["-I", "DOCKER-USER", "-d", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
            log::info!("added ACCEPT rules to DOCKER-USER chain for {BRIDGE_SUBNET}");
        }
    }

    // Disable bridge netfilter — prevents iptables/nftables from
    // filtering traffic that's already on the bridge (local traffic).
    fs::write("/proc/sys/net/bridge/bridge-nf-call-iptables", "0").ok();
    fs::write("/proc/sys/net/bridge/bridge-nf-call-ip6tables", "0").ok();

    Ok(())
}

/// Set up NAT (masquerade) rules for outbound container traffic.
///
/// Tries firewalld first (Fedora/RHEL), then iptables, then nftables.
pub fn setup_nat() -> Result<()> {
    // Method 1: firewalld (Fedora, RHEL, CentOS)
    if root_cmd("firewall-cmd").arg("--state").output()
        .map(|o| o.status.success()).unwrap_or(false)
    {
        // firewalld handles masquerade via the trusted zone (set in ensure_bridge)
        log::info!("NAT handled by firewalld trusted zone masquerade");
        return Ok(());
    }

    // Method 2: iptables (Ubuntu, Debian, older systems)
    if root_cmd("iptables").arg("--version").output().is_ok() {
        let output = root_cmd("iptables")
            .args(["-t", "nat", "-C", "POSTROUTING", "-s", BRIDGE_SUBNET, "-j", "MASQUERADE"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                log::info!("NAT masquerade rule already exists for {BRIDGE_SUBNET}");
                return Ok(());
            }
        }

        run_cmd("iptables", &["-t", "nat", "-A", "POSTROUTING", "-s", BRIDGE_SUBNET, "-j", "MASQUERADE"]).ok();
        run_cmd("iptables", &["-A", "FORWARD", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
        run_cmd("iptables", &["-A", "FORWARD", "-d", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
        log::info!("added iptables NAT masquerade for {BRIDGE_SUBNET}");
        return Ok(());
    }

    // Method 3: nftables directly (modern Fedora without iptables compat)
    if root_cmd("nft").arg("list").arg("ruleset").output().is_ok() {
        run_cmd("nft", &[
            "add", "table", "ip", "corten",
        ]).ok();
        run_cmd("nft", &[
            "add", "chain", "ip", "corten", "postrouting",
            "{ type nat hook postrouting priority 100 ; }",
        ]).ok();
        run_cmd("nft", &[
            "add", "rule", "ip", "corten", "postrouting",
            "ip", "saddr", BRIDGE_SUBNET, "masquerade",
        ]).ok();
        run_cmd("nft", &[
            "add", "chain", "ip", "corten", "forward",
            "{ type filter hook forward priority 0 ; }",
        ]).ok();
        run_cmd("nft", &[
            "add", "rule", "ip", "corten", "forward",
            "ip", "saddr", BRIDGE_SUBNET, "accept",
        ]).ok();
        run_cmd("nft", &[
            "add", "rule", "ip", "corten", "forward",
            "ip", "daddr", BRIDGE_SUBNET, "accept",
        ]).ok();
        log::info!("added nftables NAT masquerade for {BRIDGE_SUBNET}");
        return Ok(());
    }

    log::warn!("no firewall tool found (firewalld/iptables/nft) — NAT may not work");

    // Allow forwarding for container traffic (FORWARD chain may default to DROP)
    run_cmd(
        "iptables",
        &["-A", "FORWARD", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"],
    )
    .ok();
    run_cmd(
        "iptables",
        &["-A", "FORWARD", "-d", BRIDGE_SUBNET, "-j", "ACCEPT"],
    )
    .ok();

    log::info!("added FORWARD accept rules for {BRIDGE_SUBNET}");
    Ok(())
}

/// Set up the full network stack for a container.
///
/// This must be called from the PARENT process after `clone()` but
/// BEFORE signaling the child (so the network is ready when the child
/// starts). It:
///
/// 1. Creates a veth pair (host side + container side)
/// 2. Attaches the host side to the `corten0` bridge
/// 3. Moves the container side into the container's network namespace
/// 4. Allocates an IP address from the 10.0.42.0/24 pool
/// 5. Configures the container side (IP, default route)
///
/// # Arguments
///
/// * `container_id` - The container's UUID, used to derive interface names
/// * `child_pid` - The container process PID (for namespace operations)
pub fn setup_container_network(container_id: &str, child_pid: i32) -> Result<ContainerNetwork> {
    let short_id = &container_id[..8];
    let veth_host = format!("veth-{short_id}");
    // Use unique peer name in host namespace, rename to eth0 inside container
    let veth_peer = format!("peer-{short_id}");

    log::info!("setting up network for container {short_id} (PID {child_pid})");

    // Clean up stale veth if exists from a crashed run
    run_cmd("ip", &["link", "del", &veth_host]).ok();

    // 1. Create the veth pair (both names unique in host namespace)
    run_cmd(
        "ip",
        &[
            "link", "add", &veth_host, "type", "veth",
            "peer", "name", &veth_peer,
        ],
    )
    .with_context(|| format!("failed to create veth pair {veth_host} <-> {veth_peer}"))?;

    // 2. Attach host side to the bridge
    run_cmd("ip", &["link", "set", &veth_host, "master", BRIDGE_NAME])
        .with_context(|| format!("failed to attach {veth_host} to {BRIDGE_NAME}"))?;

    // 3. Bring up the host side
    run_cmd("ip", &["link", "set", &veth_host, "up"])
        .with_context(|| format!("failed to bring up {veth_host}"))?;

    // 4. Move the peer into the container's network namespace
    let pid_str = child_pid.to_string();
    run_cmd(
        "ip",
        &["link", "set", &veth_peer, "netns", &pid_str],
    )
    .with_context(|| {
        format!("failed to move {veth_peer} into namespace of PID {child_pid}")
    })?;

    // 5. Rename peer to eth0 inside the container namespace
    let net_ns = format!("--net=/proc/{child_pid}/ns/net");
    run_cmd(
        "nsenter",
        &[&net_ns, "ip", "link", "set", &veth_peer, "name", "eth0"],
    )
    .with_context(|| format!("failed to rename {veth_peer} to eth0 in container"))?;

    // 6. Allocate an IP address
    let ip = allocate_ip().context("failed to allocate container IP")?;
    let ip_cidr = format!("{ip}/24");

    // 7. Configure eth0 inside the container namespace
    run_cmd("nsenter", &[&net_ns, "ip", "addr", "add", &ip_cidr, "dev", "eth0"])
        .with_context(|| format!("failed to assign {ip_cidr} to eth0"))?;

    run_cmd("nsenter", &[&net_ns, "ip", "link", "set", "eth0", "up"])
        .with_context(|| "failed to bring up eth0 in container".to_string())?;

    run_cmd("nsenter", &[&net_ns, "ip", "route", "add", "default", "via", BRIDGE_IP])
        .with_context(|| format!("failed to add default route via {BRIDGE_IP}"))?;

    log::info!(
        "container {short_id}: network configured (IP={ip}, bridge={BRIDGE_NAME}, veth={veth_host})"
    );

    Ok(ContainerNetwork {
        ip,
        bridge: BRIDGE_NAME.to_string(),
        veth_host,
    })
}

/// Copy the host's DNS configuration into the container rootfs.
///
/// Copies `/etc/resolv.conf` from the host into the container's rootfs
/// so that DNS resolution works inside the container.
///
/// # Arguments
///
/// * `rootfs` - Path to the container's root filesystem (the merged overlay)
pub fn setup_container_dns(rootfs: &Path) -> Result<()> {
    let host_resolv = Path::new("/etc/resolv.conf");
    let container_etc = rootfs.join("etc");
    let container_resolv = container_etc.join("resolv.conf");

    if !host_resolv.exists() {
        log::warn!("host /etc/resolv.conf not found, skipping DNS setup");
        return Ok(());
    }

    // Ensure the etc directory exists in the container rootfs
    fs::create_dir_all(&container_etc)
        .with_context(|| format!("failed to create {}", container_etc.display()))?;

    fs::copy(host_resolv, &container_resolv).with_context(|| {
        format!(
            "failed to copy /etc/resolv.conf to {}",
            container_resolv.display()
        )
    })?;

    log::info!("copied /etc/resolv.conf into container rootfs");
    Ok(())
}

/// Clean up network resources for a stopped container.
///
/// Deletes the host-side veth interface (which automatically removes
/// the container-side peer as well) and releases the allocated IP.
///
/// # Arguments
///
/// * `container_id` - The container's UUID
pub fn cleanup_container_network(container_id: &str) -> Result<()> {
    let short_id = &container_id[..8];
    let veth_host = format!("veth-{short_id}");

    // Delete the veth pair (removes both ends)
    let output = root_cmd("ip")
        .args(["link", "show", &veth_host])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            run_cmd("ip", &["link", "del", &veth_host]).ok();
            log::info!("deleted veth pair {veth_host}");
        } else {
            log::info!("veth {veth_host} already gone (container exited)");
        }
    }

    // Release the allocated IP (best-effort; the allocator is simple)
    release_ip(container_id).ok();

    Ok(())
}

/// Allocate the next available IP address from the 10.0.42.0/24 pool.
///
/// Uses a simple file-based allocator that stores the next available
/// last-octet value in `/var/lib/corten/network/next_ip`. IPs start
/// at 10.0.42.2 (since .1 is the bridge) and go up to .254.
fn allocate_ip() -> Result<String> {
    fs::create_dir_all(NETWORK_STATE_DIR)
        .context("failed to create network state directory")?;

    // Read current next IP octet, default to 2 (first usable address)
    let next_octet: u8 = if Path::new(NEXT_IP_FILE).exists() {
        let content = fs::read_to_string(NEXT_IP_FILE)
            .context("failed to read next_ip file")?;
        content
            .trim()
            .parse()
            .context("failed to parse next_ip value")?
    } else {
        2
    };

    if next_octet > 254 {
        return Err(anyhow!(
            "IP address pool exhausted (all 253 addresses in 10.0.42.0/24 are allocated)"
        ));
    }

    let ip = format!("10.0.42.{next_octet}");

    // Write the next octet for the next allocation
    let next = next_octet + 1;
    fs::write(NEXT_IP_FILE, next.to_string())
        .context("failed to update next_ip file")?;

    // Record the allocation keyed by container ID
    log::info!("allocated IP {ip} (next available: 10.0.42.{next})");
    Ok(ip)
}

/// Release an IP address back to the pool (best-effort).
///
/// For now this is a no-op since the simple sequential allocator
/// doesn't support gaps. A future improvement could track allocated
/// IPs in a set and reclaim them.
fn release_ip(_container_id: &str) -> Result<()> {
    // TODO: implement IP reclamation with a proper allocator
    // For now, IPs are allocated sequentially and not reclaimed.
    Ok(())
}

/// Set up port forwarding rules for a container.
///
/// For each port mapping, adds an iptables DNAT rule that forwards
/// traffic from the host port to the container's IP and port.
///
/// # Arguments
///
/// * `container_ip` — The container's allocated IP address
/// * `ports` — List of port mappings to set up
pub fn setup_port_forwarding(
    container_ip: &str,
    ports: &[crate::config::PortMapping],
) -> Result<()> {
    for port in ports {
        let dport = port.host_port.to_string();
        let cport = port.container_port.to_string();
        let to_dest = format!("{container_ip}:{}", port.container_port);

        // 1. DNAT in PREROUTING: external traffic arriving at the host
        run_cmd(
            "iptables",
            &["-t", "nat", "-I", "PREROUTING", "-p", "tcp", "--dport", &dport,
              "-j", "DNAT", "--to-destination", &to_dest],
        )
        .with_context(|| format!(
            "failed to add DNAT rule for {}:{} -> {}:{}",
            port.host_ip, port.host_port, container_ip, port.container_port
        ))?;

        // 2. DNAT in OUTPUT: localhost traffic (requires route_localnet=1)
        run_cmd(
            "iptables",
            &["-t", "nat", "-I", "OUTPUT", "-p", "tcp", "--dport", &dport,
              "-j", "DNAT", "--to-destination", &to_dest],
        ).ok();

        // 3. MASQUERADE return traffic from container back to localhost
        // Without this, container replies to 127.0.0.1 which gets dropped
        run_cmd(
            "iptables",
            &["-t", "nat", "-I", "POSTROUTING", "-d", container_ip,
              "-s", "127.0.0.0/8", "-p", "tcp", "--dport", &cport,
              "-j", "MASQUERADE"],
        ).ok();

        // 4. FORWARD: explicit accept for DNATed traffic to container
        // Needed because Docker sets FORWARD policy to DROP
        run_cmd(
            "iptables",
            &["-I", "FORWARD", "-d", container_ip, "-o", BRIDGE_NAME,
              "-p", "tcp", "--dport", &cport, "-j", "ACCEPT"],
        ).ok();

        // 5. FORWARD: allow return traffic from container
        run_cmd(
            "iptables",
            &["-I", "FORWARD", "-s", container_ip, "-i", BRIDGE_NAME,
              "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"],
        ).ok();

        log::info!(
            "port forwarding: {}:{} -> {}:{}",
            port.host_ip, port.host_port, container_ip, port.container_port
        );
    }
    Ok(())
}

/// Remove port forwarding rules for a container.
pub fn cleanup_port_forwarding(
    container_ip: &str,
    ports: &[crate::config::PortMapping],
) -> Result<()> {
    for port in ports {
        let dport = port.host_port.to_string();
        let to_dest = format!("{container_ip}:{}", port.container_port);

        // Remove iptables DNAT rules (best-effort)
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "PREROUTING",
                "-p", "tcp", "--dport", &dport,
                "-j", "DNAT", "--to-destination", &to_dest,
            ],
        )
        .ok();

        // Remove OUTPUT rule
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "OUTPUT",
                "-d", &port.host_ip,
                "-p", "tcp", "--dport", &dport,
                "-j", "DNAT", "--to-destination", &to_dest,
            ],
        )
        .ok();

        log::info!(
            "removed port forwarding: {}:{} -> {}:{}",
            port.host_ip, port.host_port, container_ip, port.container_port
        );
    }
    Ok(())
}

/// Flush all corten-related iptables DNAT rules (for system prune).
///
/// Removes any PREROUTING/OUTPUT rules targeting the corten subnet.
pub fn flush_port_forwarding() {
    // Remove all DNAT rules targeting 10.0.42.x from PREROUTING and OUTPUT
    for chain in &["PREROUTING", "OUTPUT"] {
        // Keep removing matching rules until none left
        loop {
            let output = root_cmd("iptables")
                .args(["-t", "nat", "-S", chain])
                .output();
            let Ok(output) = output else { break };
            let rules = String::from_utf8_lossy(&output.stdout);
            let rule_to_delete = rules.lines().find(|line| {
                line.contains("DNAT") && line.contains("10.0.42.")
            });
            if let Some(rule) = rule_to_delete {
                // Convert -A to -D for deletion
                let delete_rule = rule.replacen("-A", "-D", 1);
                let args: Vec<&str> = delete_rule.split_whitespace().collect();
                let mut cmd = root_cmd("iptables");
                cmd.arg("-t").arg("nat");
                for arg in &args {
                    cmd.arg(arg);
                }
                cmd.output().ok();
            } else {
                break;
            }
        }
    }
    log::info!("flushed all corten port forwarding rules");
}

/// Clean up all stale corten veth interfaces.
pub fn cleanup_stale_veths() {
    let output = root_cmd("ip").args(["link", "show"]).output();
    if let Ok(output) = output {
        let links = String::from_utf8_lossy(&output.stdout);
        for line in links.lines() {
            // Match veth-XXXXXXXX (corten veth host side)
            if let Some(start) = line.find("veth-") {
                let name: String = line[start..].chars().take_while(|c| !c.is_whitespace() && *c != '@').collect();
                if name.starts_with("veth-") && name.len() <= 14 {
                    // Check if it's NO-CARRIER (peer is gone)
                    if line.contains("NO-CARRIER") || line.contains("LOWERLAYERDOWN") {
                        run_cmd("ip", &["link", "del", &name]).ok();
                        log::info!("cleaned up stale veth: {name}");
                    }
                }
            }
        }
    }
}

// =============================================================================
// Named networks
// =============================================================================

/// Directory where named network metadata is stored.
const NETWORKS_DIR: &str = "/var/lib/corten/networks";

/// Named network metadata stored on disk.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct NetworkInfo {
    /// Network name
    pub name: String,
    /// Bridge interface name (e.g., "corten-backend")
    pub bridge: String,
    /// Subnet in CIDR (e.g., "10.0.43.0/24")
    pub subnet: String,
    /// Gateway IP (e.g., "10.0.43.1")
    pub gateway: String,
    /// Container name → IP mappings for DNS resolution
    pub containers: std::collections::HashMap<String, String>,
}

/// Create a named network with its own bridge and subnet.
///
/// Each named network gets a unique subnet: 10.0.43.0/24, 10.0.44.0/24, etc.
pub fn create_network(name: &str) -> Result<NetworkInfo> {
    let networks_dir = Path::new(NETWORKS_DIR);
    fs::create_dir_all(networks_dir).context("failed to create networks directory")?;

    let network_dir = networks_dir.join(name);
    if network_dir.exists() {
        return Err(anyhow!("network '{name}' already exists"));
    }
    fs::create_dir_all(&network_dir)?;

    // Allocate a subnet: read how many networks exist, pick the next third-octet
    let third_octet = allocate_network_octet()?;
    let subnet = format!("10.0.{third_octet}.0/24");
    let gateway = format!("10.0.{third_octet}.1");
    let gateway_cidr = format!("{gateway}/24");
    let bridge_name = format!("corten-{}", &name[..name.len().min(10)]);

    // Create the bridge
    run_cmd("ip", &["link", "add", &bridge_name, "type", "bridge"])
        .with_context(|| format!("failed to create bridge {bridge_name}"))?;
    run_cmd("ip", &["addr", "add", &gateway_cidr, "dev", &bridge_name])
        .with_context(|| format!("failed to assign {gateway_cidr} to {bridge_name}"))?;
    run_cmd("ip", &["link", "set", &bridge_name, "up"])
        .with_context(|| format!("failed to bring up {bridge_name}"))?;

    // Add NAT for the new subnet
    run_cmd(
        "iptables",
        &["-t", "nat", "-A", "POSTROUTING", "-s", &subnet, "-j", "MASQUERADE"],
    )
    .ok(); // Best-effort

    let info = NetworkInfo {
        name: name.to_string(),
        bridge: bridge_name,
        subnet,
        gateway,
        containers: std::collections::HashMap::new(),
    };

    // Save metadata
    let meta_path = network_dir.join("network.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&info)?)
        .context("failed to save network metadata")?;

    log::info!("created network '{}' ({})", info.name, info.subnet);
    Ok(info)
}

/// List all named networks.
pub fn list_networks() -> Result<Vec<NetworkInfo>> {
    let networks_dir = Path::new(NETWORKS_DIR);
    let mut networks = Vec::new();

    if !networks_dir.exists() {
        return Ok(networks);
    }

    for entry in fs::read_dir(networks_dir)? {
        let entry = entry?;
        let meta_path = entry.path().join("network.json");
        if meta_path.exists() {
            if let Ok(data) = fs::read_to_string(&meta_path) {
                if let Ok(info) = serde_json::from_str::<NetworkInfo>(&data) {
                    networks.push(info);
                }
            }
        }
    }

    Ok(networks)
}

/// Remove a named network.
pub fn remove_network(name: &str) -> Result<()> {
    let network_dir = Path::new(NETWORKS_DIR).join(name);
    if !network_dir.exists() {
        return Err(anyhow!("network '{name}' not found"));
    }

    let meta_path = network_dir.join("network.json");
    let info: NetworkInfo = serde_json::from_str(
        &fs::read_to_string(&meta_path).context("failed to read network metadata")?,
    )
    .context("failed to parse network metadata")?;

    if !info.containers.is_empty() {
        return Err(anyhow!(
            "network '{name}' has {} active container(s). Stop them first.",
            info.containers.len()
        ));
    }

    // Stop dnsmasq
    stop_network_dns(name);

    // Delete the bridge
    run_cmd("ip", &["link", "del", &info.bridge]).ok();

    // Remove NAT rule
    run_cmd(
        "iptables",
        &["-t", "nat", "-D", "POSTROUTING", "-s", &info.subnet, "-j", "MASQUERADE"],
    )
    .ok();

    // Remove metadata
    fs::remove_dir_all(&network_dir).context("failed to remove network directory")?;

    log::info!("removed network '{name}'");
    Ok(())
}

/// Load a named network's metadata.
pub fn load_network(name: &str) -> Result<NetworkInfo> {
    let meta_path = Path::new(NETWORKS_DIR).join(name).join("network.json");
    let data = fs::read_to_string(&meta_path)
        .with_context(|| format!("network '{name}' not found"))?;
    serde_json::from_str(&data).context("failed to parse network metadata")
}

/// Save updated network metadata.
fn save_network(info: &NetworkInfo) -> Result<()> {
    let meta_path = Path::new(NETWORKS_DIR).join(&info.name).join("network.json");
    fs::write(&meta_path, serde_json::to_string_pretty(info)?)
        .context("failed to save network metadata")
}

/// Register a container in a named network and update DNS.
pub fn register_container_in_network(
    network_name: &str,
    container_name: &str,
    container_ip: &str,
) -> Result<()> {
    let mut info = load_network(network_name)?;
    info.containers
        .insert(container_name.to_string(), container_ip.to_string());
    save_network(&info)?;

    // Update dnsmasq hosts file and reload
    update_network_dns(network_name, &info)?;

    log::info!(
        "registered container '{container_name}' ({container_ip}) in network '{network_name}'"
    );
    Ok(())
}

/// Unregister a container from a named network and update DNS.
pub fn unregister_container_from_network(
    network_name: &str,
    container_name: &str,
) -> Result<()> {
    let mut info = load_network(network_name)?;
    info.containers.remove(container_name);
    save_network(&info)?;

    // Update dnsmasq hosts file and reload
    update_network_dns(network_name, &info)?;

    // If no containers left, stop dnsmasq
    if info.containers.is_empty() {
        stop_network_dns(network_name);
    }

    log::info!(
        "unregistered container '{container_name}' from network '{network_name}'"
    );
    Ok(())
}

/// Start dnsmasq for a named network (if not already running).
///
/// dnsmasq listens on the gateway IP and resolves container names
/// to their IPs. This is how Docker does it (at 127.0.0.11), but
/// we use the gateway IP directly — no daemon overhead.
fn start_network_dns(network_name: &str, info: &NetworkInfo) -> Result<()> {
    let network_dir = Path::new(NETWORKS_DIR).join(network_name);
    let pid_file = network_dir.join("dnsmasq.pid");
    let hosts_file = network_dir.join("dnsmasq.hosts");

    // Check if already running
    if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                if unsafe { libc::kill(pid, 0) } == 0 {
                    return Ok(()); // Already running
                }
            }
        }
        // Stale pid file
        fs::remove_file(&pid_file).ok();
    }

    // Write initial hosts file
    let mut hosts = String::new();
    for (name, ip) in &info.containers {
        hosts.push_str(&format!("{ip} {name}\n"));
    }
    fs::write(&hosts_file, &hosts)?;

    // Start dnsmasq bound to the gateway IP
    let status = root_cmd("dnsmasq")
        .args([
            "--no-daemon",
            "--no-resolv",
            "--no-hosts",
            "--bind-interfaces",
            &format!("--listen-address={}", info.gateway),
            &format!("--addn-hosts={}", hosts_file.display()),
            &format!("--pid-file={}", pid_file.display()),
            "--log-facility=-",  // log to stderr, not syslog
            "--keep-in-foreground",
        ])
        // Actually we need it in the background
        .arg("--keep-in-foreground")
        .spawn();

    // dnsmasq with --keep-in-foreground stays in foreground, we need --no-daemon removed
    // Let me use the proper daemon mode instead
    drop(status);

    root_cmd("dnsmasq")
        .args([
            "--no-resolv",
            "--no-hosts",
            "--bind-interfaces",
            &format!("--listen-address={}", info.gateway),
            &format!("--addn-hosts={}", hosts_file.display()),
            &format!("--pid-file={}", pid_file.display()),
            "--log-facility=-",
        ])
        .output()
        .context("failed to start dnsmasq — is it installed?")?;

    log::info!("started dnsmasq for network '{network_name}' on {}", info.gateway);
    Ok(())
}

/// Update the dnsmasq hosts file and reload (SIGHUP).
fn update_network_dns(network_name: &str, info: &NetworkInfo) -> Result<()> {
    let network_dir = Path::new(NETWORKS_DIR).join(network_name);
    let hosts_file = network_dir.join("dnsmasq.hosts");
    let pid_file = network_dir.join("dnsmasq.pid");

    // Write updated hosts file
    let mut hosts = String::new();
    for (name, ip) in &info.containers {
        hosts.push_str(&format!("{ip} {name}\n"));
    }
    fs::write(&hosts_file, &hosts)?;

    // Send SIGHUP to dnsmasq to reload hosts
    if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGHUP) };
                log::info!("reloaded dnsmasq for network '{network_name}'");
            }
        }
    } else {
        // dnsmasq not running yet — start it
        start_network_dns(network_name, info)?;
    }

    Ok(())
}

/// Stop dnsmasq for a named network.
fn stop_network_dns(network_name: &str) {
    let network_dir = Path::new(NETWORKS_DIR).join(network_name);
    let pid_file = network_dir.join("dnsmasq.pid");

    if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGTERM) };
                log::info!("stopped dnsmasq for network '{network_name}'");
            }
        }
        fs::remove_file(&pid_file).ok();
    }
}

/// Set up DNS for a container on a named network.
///
/// Points the container's resolv.conf to the gateway IP where dnsmasq runs.
/// Also writes /etc/hosts with localhost entry.
pub fn setup_named_network_dns(
    rootfs: &Path,
    network_name: &str,
    own_name: &str,
    own_ip: &str,
) -> Result<()> {
    let info = load_network(network_name)?;

    let container_etc = rootfs.join("etc");
    fs::create_dir_all(&container_etc).ok();

    // Point resolv.conf to the gateway where dnsmasq listens
    let resolv = format!("nameserver {}\n", info.gateway);
    fs::write(container_etc.join("resolv.conf"), &resolv)
        .context("failed to write /etc/resolv.conf")?;

    // Write minimal /etc/hosts (just localhost + self)
    let hosts = format!("127.0.0.1\tlocalhost\n{own_ip}\t{own_name}\n");
    fs::write(container_etc.join("hosts"), &hosts)
        .context("failed to write /etc/hosts")?;

    // Ensure dnsmasq is running on this network
    start_network_dns(network_name, &info)?;

    log::info!("DNS configured for '{own_name}' on network '{network_name}' (nameserver={})",
        info.gateway);
    Ok(())
}

/// Set up a container on a named network (creates veth, allocates IP from that subnet).
pub fn setup_container_named_network(
    network_name: &str,
    container_id: &str,
    child_pid: i32,
) -> Result<ContainerNetwork> {
    let info = load_network(network_name)?;

    let short_id = &container_id[..8];
    let veth_host = format!("veth-{short_id}");
    let veth_container = "eth0";

    // Create veth pair
    run_cmd(
        "ip",
        &["link", "add", &veth_host, "type", "veth", "peer", "name", veth_container],
    )?;

    // Attach to the named network's bridge
    run_cmd("ip", &["link", "set", &veth_host, "master", &info.bridge])?;
    run_cmd("ip", &["link", "set", &veth_host, "up"])?;

    // Move container side into netns
    let pid_str = child_pid.to_string();
    run_cmd("ip", &["link", "set", veth_container, "netns", &pid_str])?;

    // Allocate IP from the named network's subnet
    let third_octet: u8 = info.gateway.split('.').nth(2)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("invalid gateway IP in network '{network_name}'"))?;

    let next_ip_file = format!("{NETWORKS_DIR}/{network_name}/next_ip");
    let next_octet: u8 = if Path::new(&next_ip_file).exists() {
        fs::read_to_string(&next_ip_file)?.trim().parse().unwrap_or(2)
    } else {
        2
    };

    if next_octet > 254 {
        return Err(anyhow!("IP pool exhausted for network '{network_name}'"));
    }

    let ip = format!("10.0.{third_octet}.{next_octet}");
    let ip_cidr = format!("{ip}/24");
    fs::write(&next_ip_file, (next_octet + 1).to_string())?;

    // Configure container side
    let ns_path = format!("/proc/{child_pid}/ns/net");
    run_cmd("nsenter", &["--net", &ns_path, "ip", "addr", "add", &ip_cidr, "dev", veth_container])?;
    run_cmd("nsenter", &["--net", &ns_path, "ip", "link", "set", veth_container, "up"])?;
    run_cmd("nsenter", &["--net", &ns_path, "ip", "route", "add", "default", "via", &info.gateway])?;

    log::info!(
        "container {short_id}: connected to network '{network_name}' (IP={ip}, bridge={})",
        info.bridge
    );

    Ok(ContainerNetwork {
        ip,
        bridge: info.bridge,
        veth_host,
    })
}

/// Allocate the next third-octet for a named network subnet.
fn allocate_network_octet() -> Result<u8> {
    let alloc_file = format!("{NETWORKS_DIR}/.next_octet");
    fs::create_dir_all(NETWORKS_DIR)?;

    let octet: u8 = if Path::new(&alloc_file).exists() {
        fs::read_to_string(&alloc_file)?.trim().parse().unwrap_or(43)
    } else {
        43 // Start at 43 (42 is the default bridge)
    };

    if octet > 254 {
        return Err(anyhow!("network subnet pool exhausted"));
    }

    fs::write(&alloc_file, (octet + 1).to_string())?;
    Ok(octet)
}

/// Create a Command that will setuid(0) before exec.
/// Needed because child processes don't inherit Linux capabilities.
fn root_cmd(program: &str) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(program);
    unsafe {
        cmd.pre_exec(|| {
            libc::setgid(0);
            libc::setuid(0);
            Ok(())
        });
    }
    cmd
}

/// Run an external command and return an error if it fails.
///
/// Captures stdout and stderr for logging on failure.
fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let output = root_cmd(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "{program} {} failed (exit {}): {}{}",
            args.join(" "),
            output.status,
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!("\nstdout: {}", stdout.trim())
            }
        ));
    }

    Ok(())
}
