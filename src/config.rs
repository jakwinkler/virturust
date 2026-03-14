//! Configuration types and parsing utilities.
//!
//! Defines the core data structures that describe a container's
//! configuration and resource limits, along with helper functions
//! for parsing human-readable values from CLI arguments.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

/// Base directory for all VirtuRust data.
pub fn data_dir() -> PathBuf {
    PathBuf::from("/var/lib/virturust")
}

/// Directory where pulled images are stored.
/// Layout: `<images_dir>/<name>/<tag>/rootfs/`
pub fn images_dir() -> PathBuf {
    data_dir().join("images")
}

/// Directory where container state is stored.
/// Layout: `<containers_dir>/<container-id>/config.json`
pub fn containers_dir() -> PathBuf {
    data_dir().join("containers")
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
/// # use virturust::config::parse_memory;
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
/// # use virturust::config::parse_image_ref;
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
}
