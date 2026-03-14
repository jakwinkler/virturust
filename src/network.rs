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
    let output = Command::new("ip")
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
    log::info!("IP forwarding enabled");

    Ok(())
}

/// Set up NAT (masquerade) rules for outbound container traffic.
///
/// Adds an iptables MASQUERADE rule for the 10.0.42.0/24 subnet so
/// containers can reach the internet through the host. Checks if the
/// rule already exists before adding it.
pub fn setup_nat() -> Result<()> {
    // Check if the MASQUERADE rule already exists
    let output = Command::new("iptables")
        .args([
            "-t", "nat", "-C", "POSTROUTING",
            "-s", BRIDGE_SUBNET,
            "-j", "MASQUERADE",
        ])
        .output()
        .context("failed to check iptables NAT rule")?;

    if output.status.success() {
        log::info!("NAT masquerade rule already exists for {BRIDGE_SUBNET}");
        return Ok(());
    }

    // Add the MASQUERADE rule
    run_cmd(
        "iptables",
        &[
            "-t", "nat", "-A", "POSTROUTING",
            "-s", BRIDGE_SUBNET,
            "-j", "MASQUERADE",
        ],
    )
    .context("failed to add iptables MASQUERADE rule")?;

    log::info!("added NAT masquerade rule for {BRIDGE_SUBNET}");
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
    let veth_container = "eth0";

    log::info!("setting up network for container {short_id} (PID {child_pid})");

    // 1. Create the veth pair
    run_cmd(
        "ip",
        &[
            "link", "add", &veth_host, "type", "veth",
            "peer", "name", veth_container,
        ],
    )
    .with_context(|| format!("failed to create veth pair {veth_host} <-> {veth_container}"))?;

    // 2. Attach host side to the bridge
    run_cmd("ip", &["link", "set", &veth_host, "master", BRIDGE_NAME])
        .with_context(|| format!("failed to attach {veth_host} to {BRIDGE_NAME}"))?;

    // 3. Bring up the host side
    run_cmd("ip", &["link", "set", &veth_host, "up"])
        .with_context(|| format!("failed to bring up {veth_host}"))?;

    // 4. Move the container side into the container's network namespace
    let pid_str = child_pid.to_string();
    run_cmd(
        "ip",
        &["link", "set", veth_container, "netns", &pid_str],
    )
    .with_context(|| {
        format!("failed to move {veth_container} into namespace of PID {child_pid}")
    })?;

    // 5. Allocate an IP address
    let ip = allocate_ip().context("failed to allocate container IP")?;
    let ip_cidr = format!("{ip}/24");

    // 6. Configure the container side (runs inside the container's netns via nsenter)
    run_cmd(
        "nsenter",
        &[
            "--net", &format!("/proc/{child_pid}/ns/net"),
            "ip", "addr", "add", &ip_cidr, "dev", veth_container,
        ],
    )
    .with_context(|| format!("failed to assign {ip_cidr} to {veth_container}"))?;

    run_cmd(
        "nsenter",
        &[
            "--net", &format!("/proc/{child_pid}/ns/net"),
            "ip", "link", "set", veth_container, "up",
        ],
    )
    .with_context(|| format!("failed to bring up {veth_container} in container"))?;

    run_cmd(
        "nsenter",
        &[
            "--net", &format!("/proc/{child_pid}/ns/net"),
            "ip", "route", "add", "default", "via", BRIDGE_IP,
        ],
    )
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
    let output = Command::new("ip")
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
        let to_dest = format!("{container_ip}:{}", port.container_port);

        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-A", "PREROUTING",
                "-d", &port.host_ip,
                "-p", "tcp", "--dport", &dport,
                "-j", "DNAT", "--to-destination", &to_dest,
            ],
        )
        .with_context(|| format!(
            "failed to add DNAT rule for {}:{} -> {}:{}",
            port.host_ip, port.host_port, container_ip, port.container_port
        ))?;

        // Also add rule for locally-originated traffic (OUTPUT chain)
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-A", "OUTPUT",
                "-d", &port.host_ip,
                "-p", "tcp", "--dport", &dport,
                "-j", "DNAT", "--to-destination", &to_dest,
            ],
        )
        .ok(); // Best-effort for local traffic

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

        // Remove PREROUTING rule
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "PREROUTING",
                "-d", &port.host_ip,
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

/// Register a container in a named network (for DNS resolution).
pub fn register_container_in_network(
    network_name: &str,
    container_name: &str,
    container_ip: &str,
) -> Result<()> {
    let mut info = load_network(network_name)?;
    info.containers
        .insert(container_name.to_string(), container_ip.to_string());
    save_network(&info)?;
    log::info!(
        "registered container '{container_name}' ({container_ip}) in network '{network_name}'"
    );
    Ok(())
}

/// Unregister a container from a named network.
pub fn unregister_container_from_network(
    network_name: &str,
    container_name: &str,
) -> Result<()> {
    let mut info = load_network(network_name)?;
    info.containers.remove(container_name);
    save_network(&info)?;
    log::info!(
        "unregistered container '{container_name}' from network '{network_name}'"
    );
    Ok(())
}

/// Set up DNS for a container on a named network.
///
/// Writes an /etc/hosts file into the container rootfs with entries
/// for all containers in the same network, enabling name resolution.
pub fn setup_named_network_dns(
    rootfs: &Path,
    network_name: &str,
    own_name: &str,
    own_ip: &str,
) -> Result<()> {
    let info = load_network(network_name)?;

    let container_etc = rootfs.join("etc");
    fs::create_dir_all(&container_etc).ok();

    // Build /etc/hosts with entries for all containers in the network
    let mut hosts = String::from("127.0.0.1\tlocalhost\n");
    hosts.push_str(&format!("{own_ip}\t{own_name}\n"));

    for (name, ip) in &info.containers {
        if name != own_name {
            hosts.push_str(&format!("{ip}\t{name}\n"));
        }
    }

    fs::write(container_etc.join("hosts"), &hosts)
        .context("failed to write /etc/hosts for named network DNS")?;

    log::info!("wrote /etc/hosts with {} entries for network '{network_name}'",
        info.containers.len() + 1);
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

/// Run an external command and return an error if it fails.
///
/// Captures stdout and stderr for logging on failure.
fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
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
