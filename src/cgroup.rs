//! cgroup v2 management for container resource limits.
//!
//! Corten uses the unified cgroup v2 hierarchy to enforce resource
//! constraints on containers. Each container gets its own cgroup under
//! `/sys/fs/cgroup/corten/<container-id>/`.
//!
//! ## Supported controllers
//!
//! | Controller | File         | Description                              |
//! |------------|--------------|------------------------------------------|
//! | memory     | `memory.max` | Hard memory limit (OOM kill on exceed)   |
//! | cpu        | `cpu.max`    | CFS bandwidth control (quota/period)     |
//! | pids       | `pids.max`   | Maximum number of processes              |

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Root of the cgroup v2 unified hierarchy.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Parent cgroup for all Corten containers.
const CORTEN_CGROUP: &str = "corten";

/// CFS period in microseconds (100ms). The quota is calculated relative
/// to this period: `quota = cpus * period`.
const CPU_PERIOD_US: u64 = 100_000;

/// Manages a cgroup v2 group for a single container.
///
/// Created via [`Cgroup::create`], which sets up the directory structure.
/// Resource limits are applied by writing to control files within the cgroup.
/// The cgroup is cleaned up by calling [`Cgroup::destroy`].
pub struct Cgroup {
    /// Absolute path to this cgroup's directory
    path: PathBuf,
}

impl Cgroup {
    /// Create a new cgroup for the given container ID.
    ///
    /// Creates the directory at `/sys/fs/cgroup/corten/<id>/`.
    /// The parent `corten` directory is also created if it doesn't exist.
    pub fn create(container_id: &str) -> Result<Self> {
        let path = PathBuf::from(CGROUP_ROOT)
            .join(CORTEN_CGROUP)
            .join(container_id);

        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create cgroup at {}", path.display()))?;

        log::info!("created cgroup at {}", path.display());
        Ok(Self { path })
    }

    /// Set the hard memory limit in bytes.
    ///
    /// Writes to `memory.max`. When a container's memory usage exceeds
    /// this limit, the kernel's OOM killer will terminate processes
    /// within the cgroup.
    ///
    /// # Example values
    /// - 256 MiB: `268_435_456`
    /// - 1 GiB: `1_073_741_824`
    pub fn set_memory_limit(&self, bytes: u64) -> Result<()> {
        fs::write(self.path.join("memory.max"), bytes.to_string())
            .with_context(|| format!("failed to set memory limit to {bytes} bytes"))?;
        log::info!("set memory.max = {bytes} bytes");
        Ok(())
    }

    /// Set the CPU bandwidth limit using CFS (Completely Fair Scheduler).
    ///
    /// Writes to `cpu.max` in the format `"<quota> <period>"` where both
    /// values are in microseconds. The `cpus` parameter is a fractional
    /// CPU count:
    ///
    /// - `0.5` = 50% of one CPU core → `"50000 100000"`
    /// - `1.0` = 100% of one CPU core → `"100000 100000"`
    /// - `2.0` = two full CPU cores → `"200000 100000"`
    pub fn set_cpu_limit(&self, cpus: f64) -> Result<()> {
        let quota = (cpus * CPU_PERIOD_US as f64) as u64;
        let value = format!("{quota} {CPU_PERIOD_US}");

        fs::write(self.path.join("cpu.max"), &value)
            .with_context(|| format!("failed to set CPU limit to {cpus} CPUs ({value})"))?;
        log::info!("set cpu.max = {value} ({cpus} CPUs)");
        Ok(())
    }

    /// Set the maximum number of processes (tasks) in this cgroup.
    ///
    /// Writes to `pids.max`. This prevents fork bombs and runaway
    /// process creation inside the container.
    pub fn set_pids_limit(&self, max: u64) -> Result<()> {
        fs::write(self.path.join("pids.max"), max.to_string())
            .with_context(|| format!("failed to set pids limit to {max}"))?;
        log::info!("set pids.max = {max}");
        Ok(())
    }

    /// Add a process to this cgroup.
    ///
    /// Writes the PID to `cgroup.procs`, which moves the process
    /// (and all its threads) into this cgroup. All resource limits
    /// immediately apply to the process.
    pub fn add_process(&self, pid: i32) -> Result<()> {
        fs::write(self.path.join("cgroup.procs"), pid.to_string())
            .with_context(|| format!("failed to add PID {pid} to cgroup"))?;
        log::debug!("added PID {pid} to cgroup");
        Ok(())
    }

    /// Remove this cgroup directory.
    ///
    /// The cgroup must have no running processes before removal.
    /// Called during container cleanup after the container process exits.
    pub fn destroy(self) -> Result<()> {
        if self.path.exists() {
            fs::remove_dir(&self.path)
                .with_context(|| format!("failed to remove cgroup at {}", self.path.display()))?;
            log::info!("removed cgroup at {}", self.path.display());
        }
        Ok(())
    }
}
