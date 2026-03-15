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
//!
//! ## Implementation
//!
//! Uses rtnetlink (netlink) for all `ip` operations instead of forking
//! external processes. Uses nix::sched::setns() instead of `nsenter`.
//! Only iptables/nftables still use Command (netfilter crates are immature).

use anyhow::{anyhow, Context, Result};
use futures::TryStreamExt;
use std::fs;
use std::os::unix::io::AsFd;
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

/// Run an async block on a fresh tokio runtime in a dedicated thread.
/// Avoids "cannot block_on inside runtime" when called from #[tokio::main].
macro_rules! netlink_block {
    ($body:expr) => {{
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to create netlink runtime")?;
                rt.block_on($body)
            })
            .join()
            .map_err(|_| anyhow!("netlink thread panicked"))?
        })
    }};
}

/// Bring up the loopback interface inside the container's network namespace.
///
/// Without this, even `localhost` / `127.0.0.1` won't work inside the
/// container. The loopback interface exists in every network namespace
/// by default, but starts in the DOWN state.
pub fn setup_loopback() -> Result<()> {
    netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name("lo".to_string()).execute();
        if let Some(link) = links.try_next().await? {
            handle.link().set(link.header.index).up().execute().await?;
        }
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}

/// Ensure the `corten0` bridge interface exists and is configured.
///
/// Creates the bridge if it doesn't already exist, assigns 10.0.42.1/24,
/// brings it up, and enables IP forwarding on the host.
pub fn ensure_bridge() -> Result<()> {
    netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        // Check if bridge exists
        let mut links = handle.link().get().match_name(BRIDGE_NAME.to_string()).execute();
        let bridge_exists = links.try_next().await?.is_some();

        if !bridge_exists {
            log::info!("creating bridge interface {BRIDGE_NAME}");

            // Create bridge
            handle
                .link()
                .add()
                .bridge(BRIDGE_NAME.to_string())
                .execute()
                .await
                .context("failed to create bridge")?;

            // Get bridge index
            let mut links = handle.link().get().match_name(BRIDGE_NAME.to_string()).execute();
            let bridge = links
                .try_next()
                .await?
                .ok_or_else(|| anyhow!("bridge not found after creation"))?;
            let bridge_idx = bridge.header.index;

            // Add IP address
            let addr: std::net::Ipv4Addr = BRIDGE_IP.parse()?;
            handle
                .address()
                .add(bridge_idx, std::net::IpAddr::V4(addr), 24)
                .execute()
                .await
                .context("failed to assign IP to bridge")?;

            // Bring up
            handle
                .link()
                .set(bridge_idx)
                .up()
                .execute()
                .await
                .context("failed to bring up bridge")?;

            log::info!("bridge {BRIDGE_NAME} created with IP {BRIDGE_IP_CIDR}");
        } else {
            log::info!("bridge {BRIDGE_NAME} already exists");
        }
        Ok::<_, anyhow::Error>(())
    })?;

    // Enable IP forwarding
    fs::write("/proc/sys/net/ipv4/ip_forward", "1")
        .context("failed to enable IP forwarding")?;
    log::info!("IP forwarding enabled");

    // Docker coexistence: add DOCKER-USER rules if Docker is running
    if Command::new("iptables")
        .args(["-L", "DOCKER-USER", "-n"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let already = Command::new("iptables")
            .args(["-C", "DOCKER-USER", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !already {
            run_cmd("iptables", &["-I", "DOCKER-USER", "-s", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
            run_cmd("iptables", &["-I", "DOCKER-USER", "-d", BRIDGE_SUBNET, "-j", "ACCEPT"]).ok();
            log::info!("added ACCEPT rules to DOCKER-USER chain for {BRIDGE_SUBNET}");
        }
    }

    // Disable bridge netfilter
    fs::write("/proc/sys/net/bridge/bridge-nf-call-iptables", "0").ok();
    fs::write("/proc/sys/net/bridge/bridge-nf-call-ip6tables", "0").ok();

    Ok(())
}

/// Set up NAT (masquerade) rules for outbound container traffic.
///
/// Adds iptables MASQUERADE + FORWARD rules directly.
/// Falls back to nftables if iptables is unavailable.
pub fn setup_nat() -> Result<()> {
    // Try iptables first
    if Command::new("iptables").arg("--version").output().is_ok() {
        // Check if the MASQUERADE rule already exists
        let output = Command::new("iptables")
            .args(["-t", "nat", "-C", "POSTROUTING", "-s", BRIDGE_SUBNET, "-j", "MASQUERADE"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                log::info!("NAT masquerade rule already exists for {BRIDGE_SUBNET}");
                return Ok(());
            }
        }

        run_cmd(
            "iptables",
            &["-t", "nat", "-A", "POSTROUTING", "-s", BRIDGE_SUBNET, "-j", "MASQUERADE"],
        )
        .ok();
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
        log::info!("added iptables NAT masquerade for {BRIDGE_SUBNET}");
        return Ok(());
    }

    // Fallback: nftables
    if Command::new("nft").arg("list").arg("ruleset").output().is_ok() {
        run_cmd("nft", &["add", "table", "ip", "corten"]).ok();
        run_cmd(
            "nft",
            &[
                "add", "chain", "ip", "corten", "postrouting",
                "{ type nat hook postrouting priority 100 ; }",
            ],
        )
        .ok();
        run_cmd(
            "nft",
            &[
                "add", "rule", "ip", "corten", "postrouting",
                "ip", "saddr", BRIDGE_SUBNET, "masquerade",
            ],
        )
        .ok();
        run_cmd(
            "nft",
            &[
                "add", "chain", "ip", "corten", "forward",
                "{ type filter hook forward priority 0 ; }",
            ],
        )
        .ok();
        run_cmd(
            "nft",
            &[
                "add", "rule", "ip", "corten", "forward",
                "ip", "saddr", BRIDGE_SUBNET, "accept",
            ],
        )
        .ok();
        run_cmd(
            "nft",
            &[
                "add", "rule", "ip", "corten", "forward",
                "ip", "daddr", BRIDGE_SUBNET, "accept",
            ],
        )
        .ok();
        log::info!("added nftables NAT masquerade for {BRIDGE_SUBNET}");
        return Ok(());
    }

    log::warn!("no firewall tool found (iptables/nft) — NAT may not work");
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
/// 5. Configures the container side (IP, default route) via setns()
///
/// # Arguments
///
/// * `container_id` - The container's UUID, used to derive interface names
/// * `child_pid` - The container process PID (for namespace operations)
pub fn setup_container_network(container_id: &str, child_pid: i32) -> Result<ContainerNetwork> {
    let short_id = &container_id[..8];
    let veth_host = format!("veth-{short_id}");
    let veth_peer = format!("peer-{short_id}");

    log::info!("setting up network for container {short_id} (PID {child_pid})");

    let ip = allocate_ip().context("failed to allocate container IP")?;

    // Phase 1: Host-side netlink operations (create veth, attach to bridge, move peer)
    let veth_host2 = veth_host.clone();
    let veth_peer2 = veth_peer.clone();
    netlink_block!(async move {
        let (veth_host, veth_peer) = (veth_host2, veth_peer2);
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        // Clean up stale veth if exists
        let mut links = handle.link().get().match_name(veth_host.clone()).execute();
        if let Some(link) = links.try_next().await? {
            handle.link().del(link.header.index).execute().await.ok();
        }

        // 1. Create veth pair
        handle
            .link()
            .add()
            .veth(veth_host.clone(), veth_peer.clone())
            .execute()
            .await
            .context("failed to create veth pair")?;

        // 2. Get bridge index
        let mut links = handle.link().get().match_name(BRIDGE_NAME.to_string()).execute();
        let bridge = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("bridge {BRIDGE_NAME} not found"))?;
        let bridge_idx = bridge.header.index;

        // 3. Get host veth index and attach to bridge
        let mut links = handle.link().get().match_name(veth_host.clone()).execute();
        let host_veth = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("veth {veth_host} not found"))?;
        let host_veth_idx = host_veth.header.index;

        handle
            .link()
            .set(host_veth_idx)
            .controller(bridge_idx)
            .execute()
            .await
            .context("failed to attach veth to bridge")?;
        handle
            .link()
            .set(host_veth_idx)
            .up()
            .execute()
            .await
            .context("failed to bring up host veth")?;

        // 4. Get peer veth index and move to container netns
        let mut links = handle.link().get().match_name(veth_peer.clone()).execute();
        let peer_veth = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("peer veth not found"))?;
        let peer_veth_idx = peer_veth.header.index;

        handle
            .link()
            .set(peer_veth_idx)
            .setns_by_pid(child_pid as u32)
            .execute()
            .await
            .context("failed to move veth to container netns")?;

        Ok::<_, anyhow::Error>(())
    })?;

    // Phase 2: Configure inside the container's network namespace using setns()
    configure_container_netns(child_pid, &veth_peer, &ip, BRIDGE_IP)?;

    log::info!(
        "container {short_id}: network configured (IP={ip}, bridge={BRIDGE_NAME}, veth={veth_host})"
    );

    Ok(ContainerNetwork {
        ip,
        bridge: BRIDGE_NAME.to_string(),
        veth_host,
    })
}

/// Enter a container's network namespace via setns(), configure networking,
/// then return to the host namespace.
fn configure_container_netns(
    child_pid: i32,
    veth_peer: &str,
    ip: &str,
    gateway: &str,
) -> Result<()> {
    // Open the container's net namespace
    let ns_path = format!("/proc/{child_pid}/ns/net");
    let ns_fd = std::fs::File::open(&ns_path).context("failed to open container net namespace")?;

    // Save our current namespace so we can return to it
    let my_ns =
        std::fs::File::open("/proc/self/ns/net").context("failed to open own net namespace")?;

    // Enter container's namespace
    nix::sched::setns(ns_fd.as_fd(), nix::sched::CloneFlags::CLONE_NEWNET)
        .context("failed to setns into container")?;

    // Configure networking inside the container namespace
    let result = configure_inside_container_netns(veth_peer, ip, gateway);

    // Always return to our own namespace, even if configuration failed
    nix::sched::setns(my_ns.as_fd(), nix::sched::CloneFlags::CLONE_NEWNET)
        .context("failed to return to host namespace")?;

    result
}

/// Configure networking inside the container's network namespace.
/// Must be called while already in the container's netns.
fn configure_inside_container_netns(veth_peer: &str, ip: &str, gateway: &str) -> Result<()> {
    netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        // Find the peer veth (it's now in this namespace)
        let mut links = handle.link().get().execute();
        let mut peer_idx = None;
        while let Some(link) = links.try_next().await? {
            for nla in &link.attributes {
                if let netlink_packet_route::link::LinkAttribute::IfName(name) = nla {
                    if name == veth_peer {
                        peer_idx = Some(link.header.index);
                    }
                }
            }
        }

        let idx = peer_idx.ok_or_else(|| anyhow!("peer veth not found in container ns"))?;

        // Rename to eth0
        handle
            .link()
            .set(idx)
            .name("eth0".to_string())
            .execute()
            .await
            .context("failed to rename veth to eth0")?;

        // Add IP address
        let addr: std::net::Ipv4Addr = ip.parse()?;
        handle
            .address()
            .add(idx, std::net::IpAddr::V4(addr), 24)
            .execute()
            .await
            .context("failed to assign IP")?;

        // Bring up eth0
        handle
            .link()
            .set(idx)
            .up()
            .execute()
            .await
            .context("failed to bring up eth0")?;

        // Bring up loopback too
        let mut lo_links = handle.link().get().match_name("lo".to_string()).execute();
        if let Some(lo) = lo_links.try_next().await? {
            handle.link().set(lo.header.index).up().execute().await.ok();
        }

        // Add default route via bridge gateway
        let gw: std::net::Ipv4Addr = gateway.parse()?;
        handle
            .route()
            .add()
            .v4()
            .destination_prefix(std::net::Ipv4Addr::new(0, 0, 0, 0), 0)
            .gateway(gw)
            .execute()
            .await
            .context("failed to add default route")?;

        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
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

    netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name(veth_host.clone()).execute();
        if let Some(link) = links.try_next().await? {
            handle.link().del(link.header.index).execute().await.ok();
            log::info!("deleted veth pair {veth_host}");
        } else {
            log::info!("veth {veth_host} already gone");
        }
        Ok::<_, anyhow::Error>(())
    })?;

    release_ip(container_id).ok();
    Ok(())
}

/// Allocate the next available IP address from the 10.0.42.0/24 pool.
///
/// Uses a simple file-based allocator that stores the next available
/// last-octet value in `/var/lib/corten/network/next_ip`. IPs start
/// at 10.0.42.2 (since .1 is the bridge) and go up to .254.
fn allocate_ip() -> Result<String> {
    fs::create_dir_all(NETWORK_STATE_DIR).context("failed to create network state directory")?;

    // Read current next IP octet, default to 2 (first usable address)
    let next_octet: u8 = if Path::new(NEXT_IP_FILE).exists() {
        let content =
            fs::read_to_string(NEXT_IP_FILE).context("failed to read next_ip file")?;
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
    fs::write(NEXT_IP_FILE, next.to_string()).context("failed to update next_ip file")?;

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

        // PREROUTING: external traffic arriving at the host
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-I", "PREROUTING", "-p", "tcp", "--dport", &dport, "-j", "DNAT",
                "--to-destination", &to_dest,
            ],
        )
        .with_context(|| {
            format!(
                "failed to add DNAT rule for {}:{} -> {}:{}",
                port.host_ip, port.host_port, container_ip, port.container_port
            )
        })?;

        // OUTPUT: localhost traffic (curl http://127.0.0.1:port)
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-I", "OUTPUT", "-p", "tcp", "--dport", &dport, "-j", "DNAT",
                "--to-destination", &to_dest,
            ],
        )
        .ok();

        log::info!(
            "port forwarding: {}:{} -> {}:{}",
            port.host_ip,
            port.host_port,
            container_ip,
            port.container_port
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
                "-t", "nat", "-D", "PREROUTING", "-p", "tcp", "--dport", &dport, "-j", "DNAT",
                "--to-destination", &to_dest,
            ],
        )
        .ok();

        // Remove OUTPUT rule
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "OUTPUT", "-d", &port.host_ip, "-p", "tcp", "--dport", &dport,
                "-j", "DNAT", "--to-destination", &to_dest,
            ],
        )
        .ok();

        log::info!(
            "removed port forwarding: {}:{} -> {}:{}",
            port.host_ip,
            port.host_port,
            container_ip,
            port.container_port
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
            let output = Command::new("iptables")
                .args(["-t", "nat", "-S", chain])
                .output();
            let Ok(output) = output else { break };
            let rules = String::from_utf8_lossy(&output.stdout);
            let rule_to_delete = rules
                .lines()
                .find(|line| line.contains("DNAT") && line.contains("10.0.42."));
            if let Some(rule) = rule_to_delete {
                // Convert -A to -D for deletion
                let delete_rule = rule.replacen("-A", "-D", 1);
                let args: Vec<&str> = delete_rule.split_whitespace().collect();
                let mut cmd = Command::new("iptables");
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
    let _: Result<()> = netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().execute();
        let mut to_delete = Vec::new();

        while let Ok(Some(link)) = links.try_next().await {
            for nla in &link.attributes {
                if let netlink_packet_route::link::LinkAttribute::IfName(name) = nla {
                    if name.starts_with("veth-") || name.starts_with("peer-") {
                        let has_lower_up = link.header.flags.iter().any(|f| {
                            matches!(f, netlink_packet_route::link::LinkFlag::LowerUp)
                        });
                        if !has_lower_up {
                            to_delete.push((link.header.index, name.clone()));
                        }
                    }
                }
            }
        }

        for (idx, name) in to_delete {
            handle.link().del(idx).execute().await.ok();
            log::info!("cleaned up stale veth: {name}");
        }
        Ok(())
    });
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
    /// Container name -> IP mappings for DNS resolution
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

    // Create the bridge via netlink
    let nl_bridge = bridge_name.clone();
    let nl_gateway = gateway.clone();
    let nl_gateway_cidr = gateway_cidr.clone();
    netlink_block!(async move {
        let (bridge_name, gateway, gateway_cidr) = (nl_bridge, nl_gateway, nl_gateway_cidr);
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        handle
            .link()
            .add()
            .bridge(bridge_name.clone())
            .execute()
            .await
            .with_context(|| format!("failed to create bridge {bridge_name}"))?;

        let mut links = handle.link().get().match_name(bridge_name.clone()).execute();
        let bridge = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("bridge {bridge_name} not found after creation"))?;
        let bridge_idx = bridge.header.index;

        let addr: std::net::Ipv4Addr = gateway.parse()?;
        handle
            .address()
            .add(bridge_idx, std::net::IpAddr::V4(addr), 24)
            .execute()
            .await
            .with_context(|| format!("failed to assign {gateway_cidr} to {bridge_name}"))?;

        handle
            .link()
            .set(bridge_idx)
            .up()
            .execute()
            .await
            .with_context(|| format!("failed to bring up {bridge_name}"))?;

        Ok::<_, anyhow::Error>(())
    })?;

    // Add NAT for the new subnet
    run_cmd(
        "iptables",
        &[
            "-t",
            "nat",
            "-A",
            "POSTROUTING",
            "-s",
            &subnet,
            "-j",
            "MASQUERADE",
        ],
    )
    .ok();

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

    // Delete the bridge via netlink
    netlink_block!(async move {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name(info.bridge.clone()).execute();
        if let Some(link) = links.try_next().await? {
            handle.link().del(link.header.index).execute().await.ok();
        }
        Ok::<_, anyhow::Error>(())
    })
    .ok();

    // Remove NAT rule
    run_cmd(
        "iptables",
        &[
            "-t",
            "nat",
            "-D",
            "POSTROUTING",
            "-s",
            &info.subnet,
            "-j",
            "MASQUERADE",
        ],
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
    let data =
        fs::read_to_string(&meta_path).with_context(|| format!("network '{name}' not found"))?;
    serde_json::from_str(&data).context("failed to parse network metadata")
}

/// Save updated network metadata.
fn save_network(info: &NetworkInfo) -> Result<()> {
    let meta_path = Path::new(NETWORKS_DIR)
        .join(&info.name)
        .join("network.json");
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

    log::info!(
        "wrote /etc/hosts with {} entries for network '{network_name}'",
        info.containers.len() + 1
    );
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
    let veth_peer = format!("peer-{short_id}");

    // Create veth pair, attach to bridge, move peer to container netns
    let veth_host2 = veth_host.clone();
    let veth_peer2 = veth_peer.clone();
    let info_bridge = info.bridge.clone();
    netlink_block!(async move {
        let (veth_host, veth_peer) = (veth_host2, veth_peer2);
        let info_bridge = info_bridge;
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        // Create veth pair
        handle
            .link()
            .add()
            .veth(veth_host.clone(), veth_peer.clone())
            .execute()
            .await
            .context("failed to create veth pair")?;

        // Get bridge index
        let mut links = handle.link().get().match_name(info_bridge.clone()).execute();
        let bridge = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("bridge {} not found", info_bridge))?;
        let bridge_idx = bridge.header.index;

        // Attach host veth to bridge and bring up
        let mut links = handle.link().get().match_name(veth_host.clone()).execute();
        let host_veth = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("veth {veth_host} not found"))?;
        let host_veth_idx = host_veth.header.index;

        handle
            .link()
            .set(host_veth_idx)
            .controller(bridge_idx)
            .execute()
            .await?;
        handle.link().set(host_veth_idx).up().execute().await?;

        // Move peer to container netns
        let mut links = handle.link().get().match_name(veth_peer.clone()).execute();
        let peer_veth = links
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("peer veth not found"))?;
        let peer_veth_idx = peer_veth.header.index;

        handle
            .link()
            .set(peer_veth_idx)
            .setns_by_pid(child_pid as u32)
            .execute()
            .await
            .context("failed to move veth to container netns")?;

        Ok::<_, anyhow::Error>(())
    })?;

    // Allocate IP from the named network's subnet
    let third_octet: u8 = info
        .gateway
        .split('.')
        .nth(2)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("invalid gateway IP in network '{network_name}'"))?;

    let next_ip_file = format!("{NETWORKS_DIR}/{network_name}/next_ip");
    let next_octet: u8 = if Path::new(&next_ip_file).exists() {
        fs::read_to_string(&next_ip_file)?
            .trim()
            .parse()
            .unwrap_or(2)
    } else {
        2
    };

    if next_octet > 254 {
        return Err(anyhow!("IP pool exhausted for network '{network_name}'"));
    }

    let ip = format!("10.0.{third_octet}.{next_octet}");
    fs::write(&next_ip_file, (next_octet + 1).to_string())?;

    // Configure container side via setns
    configure_container_netns(child_pid, &veth_peer, &ip, &info.gateway)?;

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
        fs::read_to_string(&alloc_file)?
            .trim()
            .parse()
            .unwrap_or(43)
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
