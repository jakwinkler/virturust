//! Network namespace setup for containers.
//!
//! Each container is created in its own network namespace (`CLONE_NEWNET`),
//! which gives it a completely isolated network stack: its own interfaces,
//! routing table, iptables rules, and sockets.
//!
//! ## Current capabilities
//!
//! - Loopback interface (`lo`) brought up inside the container
//!
//! ## Planned (future releases)
//!
//! - veth pair creation (host <-> container link)
//! - Bridge networking (`virturust0` bridge)
//! - NAT/masquerade for outbound connectivity
//! - Port forwarding (`--publish` / `-p`)
//! - Container-to-container networking

use anyhow::{Context, Result};
use std::process::Command;

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
