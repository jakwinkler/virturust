//! Container lifecycle management.
//!
//! Orchestrates the full lifecycle of a container by combining
//! all isolation primitives:
//!
//! 1. **cgroups** — create resource limits before the container starts
//! 2. **clone()** — create the container process in new namespaces
//! 3. **sync** — signal the child after cgroup setup is complete
//! 4. **wait** — block until the container process exits
//! 5. **cleanup** — remove cgroup, update container state
//!
//! Container state is persisted to disk throughout the lifecycle,
//! allowing `ps`, `inspect`, and `stop` to query/control containers.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;

use crate::cgroup::Cgroup;
use crate::config::{
    containers_dir, unix_timestamp, ContainerConfig, ContainerState, ContainerStatus,
};
use crate::namespace::{create_namespaced_process, ChildArgs};

/// Save container state to disk.
fn save_state(container_dir: &Path, state: &ContainerState) -> Result<()> {
    let json = serde_json::to_string_pretty(state)?;
    fs::write(container_dir.join("state.json"), json).context("failed to save container state")
}

/// Load container state from disk.
pub fn load_state(container_dir: &Path) -> Result<ContainerState> {
    let data = fs::read_to_string(container_dir.join("state.json"))
        .context("failed to read container state")?;
    serde_json::from_str(&data).context("failed to parse container state")
}

/// Load container config from disk.
pub fn load_config(container_dir: &Path) -> Result<ContainerConfig> {
    let data = fs::read_to_string(container_dir.join("config.json"))
        .context("failed to read container config")?;
    serde_json::from_str(&data).context("failed to parse container config")
}

/// Find a container directory by name or ID prefix.
///
/// Searches all containers and returns the path to the first match.
/// Matches against both the container name and ID prefix.
pub fn find_container(name_or_id: &str) -> Result<std::path::PathBuf> {
    let dir = containers_dir();
    if !dir.exists() {
        return Err(anyhow!("no containers found"));
    }

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        // Match by exact directory name (full ID)
        if entry.file_name().to_string_lossy() == name_or_id {
            return Ok(path);
        }

        // Match by ID prefix
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(name_or_id)
        {
            return Ok(path);
        }

        // Match by container name in config
        let config_path = path.join("config.json");
        if config_path.exists() {
            if let Ok(cfg) = load_config(&path) {
                if cfg.name == name_or_id {
                    return Ok(path);
                }
            }
        }
    }

    Err(anyhow!("container not found: {name_or_id}"))
}

/// Check if a process is still alive.
pub fn is_process_alive(pid: i32) -> bool {
    // kill(pid, 0) checks if a signal CAN be sent without actually sending one
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Run a container with the given configuration.
///
/// This is the main entry point for container execution. It manages the
/// full lifecycle from creation to cleanup, persisting state at each step.
///
/// # Returns
///
/// The container process's exit code (0 = success, non-zero = error,
/// 128+N = killed by signal N).
pub fn run(config: &ContainerConfig) -> Result<i32> {
    log::info!(
        "starting container '{}' (id={}, image={})",
        config.name,
        &config.id[..12],
        config.image
    );

    // Persist container metadata
    let container_dir = containers_dir().join(&config.id);
    fs::create_dir_all(&container_dir).context("failed to create container directory")?;
    fs::write(
        container_dir.join("config.json"),
        serde_json::to_string_pretty(config)?,
    )
    .context("failed to save container config")?;

    // Initial state: created
    let mut state = ContainerState {
        status: ContainerStatus::Created,
        pid: None,
        created_at: unix_timestamp(),
        started_at: None,
        finished_at: None,
        exit_code: None,
    };
    save_state(&container_dir, &state)?;

    // Create cgroup and apply resource limits
    let cgroup = Cgroup::create(&config.id)?;

    if let Some(mem) = config.resources.memory_bytes {
        cgroup.set_memory_limit(mem)?;
    }
    if let Some(cpu) = config.resources.cpu_quota {
        cgroup.set_cpu_limit(cpu)?;
    }
    if let Some(pids) = config.resources.pids_max {
        cgroup.set_pids_limit(pids)?;
    }

    // Create sync pipe for parent-child coordination
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(anyhow!(
            "pipe() failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let (pipe_rd, pipe_wr) = (fds[0], fds[1]);

    // Create the container process in new namespaces
    let child_args = ChildArgs {
        rootfs: config.rootfs.to_string_lossy().to_string(),
        hostname: config.hostname.clone(),
        command: config.command.clone(),
        sync_pipe_rd: pipe_rd,
    };

    let child_pid = create_namespaced_process(child_args)?;

    // Close read end in parent
    unsafe { libc::close(pipe_rd) };

    // Add child to cgroup before it starts doing work
    cgroup.add_process(child_pid)?;

    // Signal child that cgroup is ready
    unsafe {
        libc::write(pipe_wr, [0u8].as_ptr() as *const libc::c_void, 1);
        libc::close(pipe_wr);
    }

    // Update state: running
    state.status = ContainerStatus::Running;
    state.pid = Some(child_pid);
    state.started_at = Some(unix_timestamp());
    save_state(&container_dir, &state)?;

    println!("Container '{}' started (PID {})", config.name, child_pid);

    // Wait for the container process to exit
    let mut wait_status: libc::c_int = 0;
    let ret = unsafe { libc::waitpid(child_pid, &mut wait_status, 0) };
    if ret == -1 {
        log::error!("waitpid failed: {}", std::io::Error::last_os_error());
    }

    let exit_code = if libc::WIFEXITED(wait_status) {
        libc::WEXITSTATUS(wait_status)
    } else if libc::WIFSIGNALED(wait_status) {
        128 + libc::WTERMSIG(wait_status)
    } else {
        1
    };

    // Update state: stopped
    state.status = ContainerStatus::Stopped;
    state.finished_at = Some(unix_timestamp());
    state.exit_code = Some(exit_code);
    save_state(&container_dir, &state)?;

    // Cleanup cgroup (container state is preserved for inspection)
    cgroup.destroy().ok();

    log::info!(
        "container '{}' exited with code {exit_code}",
        config.name
    );
    Ok(exit_code)
}

/// Stop a running container by sending signals to its process.
///
/// First sends SIGTERM and waits up to `timeout_secs` for graceful
/// shutdown. If the process is still alive after the timeout, sends
/// SIGKILL to force termination.
pub fn stop(container_dir: &Path, timeout_secs: u64) -> Result<()> {
    let config = load_config(container_dir)?;
    let mut state = load_state(container_dir)?;

    let pid = state
        .pid
        .ok_or_else(|| anyhow!("container has no PID (never started?)"))?;

    if !is_process_alive(pid) {
        state.status = ContainerStatus::Stopped;
        state.finished_at = Some(unix_timestamp());
        save_state(container_dir, &state)?;
        println!("Container '{}' is already stopped.", config.name);
        return Ok(());
    }

    // Send SIGTERM for graceful shutdown
    println!(
        "Stopping container '{}' (PID {pid})... ({}s timeout)",
        config.name, timeout_secs
    );
    unsafe { libc::kill(pid, libc::SIGTERM) };

    // Wait for process to exit
    let start = std::time::Instant::now();
    loop {
        if !is_process_alive(pid) {
            break;
        }
        if start.elapsed().as_secs() >= timeout_secs {
            // Force kill
            println!("Timeout reached, sending SIGKILL...");
            unsafe { libc::kill(pid, libc::SIGKILL) };
            std::thread::sleep(std::time::Duration::from_millis(100));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // Update state
    state.status = ContainerStatus::Stopped;
    state.finished_at = Some(unix_timestamp());
    save_state(container_dir, &state)?;

    // Clean up cgroup
    Cgroup::create(&config.id)
        .and_then(|c| c.destroy())
        .ok();

    println!("Container '{}' stopped.", config.name);
    Ok(())
}
