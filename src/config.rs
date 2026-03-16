//! Configuration types and parsing utilities.
//!
//! Defines the core data structures that describe a container's
//! configuration and resource limits, along with helper functions
//! for parsing human-readable values from CLI arguments.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Resource limits for a container, enforced via cgroups v2.
///
/// All fields are optional — `None` means no limit is applied
/// for that resource, and the container inherits the host's limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Memory limit in bytes. Enforced as a hard limit via `memory.max`.
    /// When exceeded, the kernel OOM killer terminates container processes.
    pub memory_bytes: Option<u64>,

    /// CPU limit as fractional CPUs (e.g., 0.5 = 50% of one core).
    /// Enforced via `cpu.max` using CFS bandwidth control.
    pub cpu_quota: Option<f64>,

    /// Maximum number of processes/threads. Enforced via `pids.max`.
    /// Protects against fork bombs.
    pub pids_max: Option<u64>,
}

/// A volume mount binding a host path into the container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    /// Absolute path on the host
    pub host_path: PathBuf,
    /// Absolute path inside the container
    pub container_path: PathBuf,
    /// Whether the mount is read-only
    pub read_only: bool,
}

/// A port mapping forwarding a host port to a container port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    /// IP address to bind on the host (default: 0.0.0.0)
    pub host_ip: String,
    /// Port on the host
    pub host_port: u16,
    /// Port inside the container
    pub container_port: u16,
}

/// Complete configuration for a container instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    /// Unique container identifier (UUID v4)
    pub id: String,

    /// Human-readable container name
    pub name: String,

    /// Source image reference (e.g., "alpine:latest")
    pub image: String,

    /// Command and arguments to run inside the container
    pub command: Vec<String>,

    /// Container hostname (visible inside the UTS namespace)
    pub hostname: String,

    /// Resource limits
    pub resources: ResourceLimits,

    /// Absolute path to the container's root filesystem
    pub rootfs: PathBuf,

    /// Volume mounts (host path → container path)
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,

    /// Environment variables (KEY=VALUE)
    #[serde(default)]
    pub env: Vec<String>,

    /// Working directory inside the container
    #[serde(default)]
    pub working_dir: String,

    /// User to run as (user or user:group)
    #[serde(default)]
    pub user: String,

    /// Network mode: "bridge", "none", or "host"
    #[serde(default = "default_network_mode")]
    pub network_mode: String,

    /// Port mappings (host port → container port)
    #[serde(default)]
    pub ports: Vec<PortMapping>,

    /// Restart policy: "no", "always", "on-failure:N"
    #[serde(default = "default_restart_policy")]
    pub restart_policy: String,

    /// Whether to run in rootless mode (user namespace)
    #[serde(default)]
    pub rootless: bool,

    /// Give extended privileges (disable security restrictions)
    #[serde(default)]
    pub privileged: bool,

    /// Mount root filesystem as read-only
    #[serde(default)]
    pub read_only: bool,

    /// Automatically remove container on exit
    #[serde(default)]
    pub auto_remove: bool,
}

fn default_network_mode() -> String {
    "bridge".to_string()
}

fn default_restart_policy() -> String {
    "no".to_string()
}

/// Runtime state of a container instance.
///
/// Persisted to `<containers_dir>/<id>/state.json` and updated
/// throughout the container lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerState {
    /// Current lifecycle status
    pub status: ContainerStatus,

    /// Container process PID (host namespace). Set when running.
    pub pid: Option<i32>,

    /// Unix timestamp when the container was created
    pub created_at: u64,

    /// Unix timestamp when the container started running
    pub started_at: Option<u64>,

    /// Unix timestamp when the container exited
    pub finished_at: Option<u64>,

    /// Exit code of the container process (set after exit)
    pub exit_code: Option<i32>,
}

/// Lifecycle status of a container.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ContainerStatus {
    /// Container metadata created, not yet started
    Created,
    /// Container process is running
    Running,
    /// Container process has exited
    Stopped,
}

impl std::fmt::Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// Get the current Unix timestamp in seconds.
pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Check if the current process has CAP_SYS_ADMIN capability.
///
/// This allows running without sudo when Linux capabilities are set
/// on the binary via `setcap`. Reads the effective capabilities from
/// `/proc/self/status`.
pub fn has_cap_sys_admin() -> bool {
    if nix::unistd::geteuid().is_root() {
        return true;
    }

    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };

    for line in status.lines() {
        if let Some(hex) = line.strip_prefix("CapEff:\t") {
            if let Ok(caps) = u64::from_str_radix(hex.trim(), 16) {
                // CAP_SYS_ADMIN is bit 21
                return caps & (1 << 21) != 0;
            }
        }
    }

    false
}

/// Base directory for all Corten data.
///
/// Defaults to `/var/lib/corten`. Can be overridden with the
/// `CORTEN_DATA_DIR` environment variable.
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CORTEN_DATA_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/var/lib/corten")
    }
}

/// Per-user data directory for container isolation.
///
/// Each user gets their own container space at `/var/lib/corten/users/<uid>/`.
/// Images are shared (read-only), but containers are per-user.
/// This means User A cannot see, stop, or exec into User B's containers.
fn user_dir() -> PathBuf {
    let uid = std::env::var("CORTEN_REAL_UID").unwrap_or_else(|_| "0".to_string());
    data_dir().join("users").join(uid)
}

/// Directory where pulled images are stored (shared across all users).
/// Layout: `<images_dir>/<name>/<tag>/rootfs/`
pub fn images_dir() -> PathBuf {
    data_dir().join("images")
}

/// Directory where container state is stored (per-user).
/// Layout: `<containers_dir>/<container-id>/config.json`
///
/// Each user only sees their own containers. This is enforced by
/// storing containers under `/var/lib/corten/users/<uid>/containers/`.
pub fn containers_dir() -> PathBuf {
    let uid = std::env::var("CORTEN_REAL_UID").unwrap_or_else(|_| "0".to_string());
    if uid == "0" {
        // Root sees all containers in the legacy location
        data_dir().join("containers")
    } else {
        user_dir().join("containers")
    }
}

/// Parse a human-readable memory string into bytes.
///
/// Supports binary suffixes (kibibytes, mebibytes, gibibytes):
/// - `k` or `K` — multiply by 1,024
/// - `m` or `M` — multiply by 1,048,576
/// - `g` or `G` — multiply by 1,073,741,824
/// - No suffix — treated as raw bytes
///
/// # Examples
///
/// ```
/// # use corten::config::parse_memory;
/// assert_eq!(parse_memory("256m").unwrap(), 268_435_456);
/// assert_eq!(parse_memory("1g").unwrap(), 1_073_741_824);
/// assert_eq!(parse_memory("512k").unwrap(), 524_288);
/// assert_eq!(parse_memory("1048576").unwrap(), 1_048_576);
/// ```
pub fn parse_memory(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty memory string"));
    }

    let (num_str, multiplier) = match s.as_bytes().last() {
        Some(b'k' | b'K') => (&s[..s.len() - 1], 1024u64),
        Some(b'm' | b'M') => (&s[..s.len() - 1], 1024 * 1024),
        Some(b'g' | b'G') => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1u64),
    };

    let num: u64 = num_str
        .parse()
        .map_err(|_| anyhow!("invalid memory value: '{s}'"))?;

    num.checked_mul(multiplier)
        .ok_or_else(|| anyhow!("memory value overflow: '{s}'"))
}

/// Parse an image reference into (name, tag).
///
/// If no tag is specified, defaults to `"latest"`.
///
/// # Examples
///
/// ```
/// # use corten::config::parse_image_ref;
/// assert_eq!(parse_image_ref("alpine"), ("alpine", "latest"));
/// assert_eq!(parse_image_ref("ubuntu:22.04"), ("ubuntu", "22.04"));
/// assert_eq!(parse_image_ref("debian:bookworm"), ("debian", "bookworm"));
/// ```
pub fn parse_image_ref(image: &str) -> (&str, &str) {
    match image.split_once(':') {
        Some((name, tag)) => (name, tag),
        None => (image, "latest"),
    }
}

/// Parse a volume mount specification.
///
/// Format: `/host/path:/container/path[:ro]`
///
/// Both paths must be absolute. The optional `:ro` suffix makes the mount read-only.
///
/// # Examples
///
/// ```
/// # use corten::config::parse_volume;
/// let vol = parse_volume("/src:/app").unwrap();
/// assert_eq!(vol.host_path.to_str().unwrap(), "/src");
/// assert_eq!(vol.container_path.to_str().unwrap(), "/app");
/// assert!(!vol.read_only);
///
/// let vol = parse_volume("/data:/mnt/data:ro").unwrap();
/// assert!(vol.read_only);
/// ```
pub fn parse_volume(s: &str) -> Result<VolumeMount> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();

    let (host_path, container_path, read_only) = match parts.len() {
        2 => (parts[0], parts[1], false),
        3 => {
            if parts[2] != "ro" && parts[2] != "rw" {
                return Err(anyhow!(
                    "invalid volume option '{}' (expected 'ro' or 'rw')",
                    parts[2]
                ));
            }
            (parts[0], parts[1], parts[2] == "ro")
        }
        _ => return Err(anyhow!(
            "invalid volume format '{}'. Expected: /host/path:/container/path[:ro]",
            s
        )),
    };

    let host = Path::new(host_path);
    let container = Path::new(container_path);

    if !host.is_absolute() {
        return Err(anyhow!("host path must be absolute: '{host_path}'"));
    }
    if !container.is_absolute() {
        return Err(anyhow!("container path must be absolute: '{container_path}'"));
    }

    Ok(VolumeMount {
        host_path: host.to_path_buf(),
        container_path: container.to_path_buf(),
        read_only,
    })
}

/// Parse a port mapping specification.
///
/// Formats:
/// - `host_port:container_port` — bind on all interfaces
/// - `ip:host_port:container_port` — bind on specific IP
///
/// # Examples
///
/// ```
/// # use corten::config::parse_port;
/// let port = parse_port("8080:80").unwrap();
/// assert_eq!(port.host_port, 8080);
/// assert_eq!(port.container_port, 80);
/// assert_eq!(port.host_ip, "0.0.0.0");
///
/// let port = parse_port("127.0.0.1:3000:3000").unwrap();
/// assert_eq!(port.host_ip, "127.0.0.1");
/// ```
pub fn parse_port(s: &str) -> Result<PortMapping> {
    let parts: Vec<&str> = s.split(':').collect();

    let (host_ip, host_port_str, container_port_str) = match parts.len() {
        2 => ("0.0.0.0", parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2]),
        _ => return Err(anyhow!(
            "invalid port format '{}'. Expected: host_port:container_port or ip:host_port:container_port",
            s
        )),
    };

    let host_port: u16 = host_port_str
        .parse()
        .map_err(|_| anyhow!("invalid host port: '{host_port_str}'"))?;
    let container_port: u16 = container_port_str
        .parse()
        .map_err(|_| anyhow!("invalid container port: '{container_port_str}'"))?;

    if host_port == 0 {
        return Err(anyhow!("host port cannot be 0"));
    }
    if container_port == 0 {
        return Err(anyhow!("container port cannot be 0"));
    }

    Ok(PortMapping {
        host_ip: host_ip.to_string(),
        host_port,
        container_port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_megabytes() {
        assert_eq!(parse_memory("256m").unwrap(), 256 * 1024 * 1024);
        assert_eq!(parse_memory("256M").unwrap(), 256 * 1024 * 1024);
    }

    #[test]
    fn test_parse_memory_gigabytes() {
        assert_eq!(parse_memory("1g").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_memory("2G").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_memory_kilobytes() {
        assert_eq!(parse_memory("512k").unwrap(), 512 * 1024);
    }

    #[test]
    fn test_parse_memory_raw_bytes() {
        assert_eq!(parse_memory("1048576").unwrap(), 1_048_576);
    }

    #[test]
    fn test_parse_memory_invalid() {
        assert!(parse_memory("").is_err());
        assert!(parse_memory("abc").is_err());
        assert!(parse_memory("m").is_err());
    }

    #[test]
    fn test_parse_image_ref_with_tag() {
        assert_eq!(parse_image_ref("ubuntu:22.04"), ("ubuntu", "22.04"));
    }

    #[test]
    fn test_parse_image_ref_without_tag() {
        assert_eq!(parse_image_ref("alpine"), ("alpine", "latest"));
    }

    #[test]
    fn test_parse_volume_basic() {
        let vol = parse_volume("/src:/app").unwrap();
        assert_eq!(vol.host_path, PathBuf::from("/src"));
        assert_eq!(vol.container_path, PathBuf::from("/app"));
        assert!(!vol.read_only);
    }

    #[test]
    fn test_parse_volume_read_only() {
        let vol = parse_volume("/data:/mnt:ro").unwrap();
        assert!(vol.read_only);
    }

    #[test]
    fn test_parse_volume_read_write_explicit() {
        let vol = parse_volume("/data:/mnt:rw").unwrap();
        assert!(!vol.read_only);
    }

    #[test]
    fn test_parse_volume_relative_host_fails() {
        assert!(parse_volume("relative:/app").is_err());
    }

    #[test]
    fn test_parse_volume_relative_container_fails() {
        assert!(parse_volume("/host:relative").is_err());
    }

    #[test]
    fn test_parse_volume_invalid_option_fails() {
        assert!(parse_volume("/src:/app:invalid").is_err());
    }

    #[test]
    fn test_parse_volume_missing_container_path_fails() {
        assert!(parse_volume("/src").is_err());
    }

    #[test]
    fn test_parse_port_basic() {
        let port = parse_port("8080:80").unwrap();
        assert_eq!(port.host_port, 8080);
        assert_eq!(port.container_port, 80);
        assert_eq!(port.host_ip, "0.0.0.0");
    }

    #[test]
    fn test_parse_port_with_ip() {
        let port = parse_port("127.0.0.1:3000:3000").unwrap();
        assert_eq!(port.host_ip, "127.0.0.1");
        assert_eq!(port.host_port, 3000);
        assert_eq!(port.container_port, 3000);
    }

    #[test]
    fn test_parse_port_invalid_format_fails() {
        assert!(parse_port("8080").is_err());
        assert!(parse_port("").is_err());
        assert!(parse_port("abc:80").is_err());
        assert!(parse_port("8080:abc").is_err());
    }

    #[test]
    fn test_parse_port_zero_fails() {
        assert!(parse_port("0:80").is_err());
        assert!(parse_port("8080:0").is_err());
    }
}
