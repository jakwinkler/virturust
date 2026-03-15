//! Container lifecycle management.
//!
//! Orchestrates the full lifecycle of a container by combining
//! all isolation primitives:
//!
//! 1. **networking** — create bridge, NAT, and DNS before the container starts
//! 2. **cgroups** — create resource limits before the container starts
//! 3. **clone()** — create the container process in new namespaces
//! 4. **network setup** — create veth pair and configure container networking
//! 5. **sync** — signal the child after cgroup + network setup is complete
//! 6. **wait** — block until the container process exits
//! 7. **cleanup** — remove cgroup, network, update container state
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

/// Parsed restart policy.
enum RestartPolicy {
    /// Never restart (default).
    No,
    /// Always restart regardless of exit code.
    Always,
    /// Restart only on non-zero exit. 0 means unlimited retries.
    OnFailure(u32),
}

/// Parse a restart policy string into a `RestartPolicy`.
fn parse_restart_policy(s: &str) -> RestartPolicy {
    if s == "always" {
        RestartPolicy::Always
    } else if s == "no" || s.is_empty() {
        RestartPolicy::No
    } else if let Some(rest) = s.strip_prefix("on-failure") {
        let max = rest
            .strip_prefix(':')
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        RestartPolicy::OnFailure(max)
    } else {
        RestartPolicy::No
    }
}

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
pub fn run(config: &ContainerConfig, detach: bool) -> Result<i32> {
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

    // In rootless mode, some features are unavailable
    if config.rootless {
        log::info!("rootless mode: skipping cgroups, using copy rootfs, network=none");
    }

    // Set up per-container writable rootfs using OverlayFS.
    // lower = image rootfs (read-only, shared), upper = per-container writable layer.
    // Falls back to a full copy if OverlayFS is not available.
    // In rootless mode, OverlayFS requires root, so skip it and use copy fallback.
    let overlay_dir = container_dir.join("overlay");
    let container_rootfs = if !config.rootless && crate::filesystem::setup_overlay(
        &config.rootfs,
        &overlay_dir,
    )
    .is_ok()
    {
        log::info!("using OverlayFS for container rootfs");
        overlay_dir.join("merged")
    } else {
        log::warn!("OverlayFS not available, falling back to rootfs copy");
        let rootfs_copy = container_dir.join("rootfs");
        copy_dir_recursive(&config.rootfs, &rootfs_copy)
            .context("failed to copy image rootfs to container directory")?;
        rootfs_copy
    };

    // Set up container DNS (copy host resolv.conf into rootfs)
    // Skip in rootless mode (no network access)
    if config.network_mode != "none" && !config.rootless {
        crate::network::setup_container_dns(&container_rootfs).ok();
    }

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

    // Set up bridge and NAT before creating the container process
    // Skip in rootless mode (requires CAP_NET_ADMIN)
    if config.network_mode == "bridge" && !config.rootless {
        crate::network::ensure_bridge()
            .context("failed to set up bridge network")?;
        crate::network::setup_nat()
            .context("failed to set up NAT")?;
    }

    // Create cgroup and apply resource limits
    // Skip in rootless mode (requires root for cgroup creation)
    let cgroup = if !config.rootless {
        let cg = Cgroup::create(&config.id)?;

        if let Some(mem) = config.resources.memory_bytes {
            cg.set_memory_limit(mem)?;
        }
        if let Some(cpu) = config.resources.cpu_quota {
            cg.set_cpu_limit(cpu)?;
        }
        if let Some(pids) = config.resources.pids_max {
            cg.set_pids_limit(pids)?;
        }

        Some(cg)
    } else {
        None
    };

    let restart_policy = parse_restart_policy(&config.restart_policy);
    let is_named_network = !matches!(config.network_mode.as_str(), "bridge" | "none" | "host");
    let mut restart_count = 0u32;
    let final_exit_code;

    // In detach mode, fork BEFORE clone so the monitor is the parent of the container.
    // waitpid only works on direct children.
    if detach {
        let monitor_pid = unsafe { libc::fork() };
        if monitor_pid < 0 {
            return Err(anyhow!("fork failed: {}", std::io::Error::last_os_error()));
        }
        if monitor_pid > 0 {
            // Original process — print container ID and return immediately
            println!("{}", config.id);
            return Ok(0);
        }
        // Monitor process — detach from terminal
        unsafe {
            libc::setsid();
            libc::close(0);
            libc::close(1);
            libc::close(2);
        }
    }

    loop {
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
            rootfs: container_rootfs.to_string_lossy().to_string(),
            hostname: config.hostname.clone(),
            command: config.command.clone(),
            sync_pipe_rd: pipe_rd,
            volumes: config.volumes.clone(),
            env: config.env.clone(),
            working_dir: config.working_dir.clone(),
            user: config.user.clone(),
            network_mode: config.network_mode.clone(),
            stdout_log: if detach {
                container_dir.join("stdout.log").to_string_lossy().to_string()
            } else {
                String::new()
            },
            stderr_log: if detach {
                container_dir.join("stderr.log").to_string_lossy().to_string()
            } else {
                String::new()
            },
            rootless: config.rootless,
            privileged: config.privileged,
            read_only: config.read_only,
        };

        let child_pid = create_namespaced_process(child_args)?;

        // Close read end in parent
        unsafe { libc::close(pipe_rd) };

        // Add child to cgroup before it starts doing work
        if let Some(ref cg) = cgroup {
            cg.add_process(child_pid)?;
        }

        // Set up UID/GID mappings for rootless mode.
        // Must happen BEFORE signaling the child so it can operate as root
        // inside the user namespace.
        if config.rootless {
            setup_uid_gid_mappings(child_pid)?;
        }

        // Set up container networking (veth pair, IP allocation, routing)
        // Skip in rootless mode (requires CAP_NET_ADMIN)
        let mut container_ip = String::new();

        if config.network_mode == "bridge" && !config.rootless {
            let container_net = crate::network::setup_container_network(&config.id, child_pid)
                .context("failed to set up container network")?;
            container_ip = container_net.ip.clone();
            log::info!(
                "container network: IP={}, bridge={}, veth={}",
                container_net.ip,
                container_net.bridge,
                container_net.veth_host
            );

            // Set up port forwarding
            if !config.ports.is_empty() {
                crate::network::setup_port_forwarding(&container_net.ip, &config.ports)
                    .context("failed to set up port forwarding")?;
            }
        } else if is_named_network && !config.rootless {
            let container_net = crate::network::setup_container_named_network(
                &config.network_mode,
                &config.id,
                child_pid,
            )
            .with_context(|| format!("failed to set up named network '{}'", config.network_mode))?;
            container_ip = container_net.ip.clone();

            // Register container for DNS resolution
            crate::network::register_container_in_network(
                &config.network_mode,
                &config.name,
                &container_net.ip,
            )
            .ok();

            // Write /etc/hosts for DNS name resolution
            crate::network::setup_named_network_dns(
                &container_rootfs,
                &config.network_mode,
                &config.name,
                &container_net.ip,
            )
            .ok();

            log::info!(
                "container network: IP={}, network={}, bridge={}",
                container_net.ip,
                config.network_mode,
                container_net.bridge
            );

            if !config.ports.is_empty() {
                crate::network::setup_port_forwarding(&container_net.ip, &config.ports)
                    .context("failed to set up port forwarding")?;
            }
        }

        // Signal child that cgroup and network are ready
        unsafe {
            libc::write(pipe_wr, [0u8].as_ptr() as *const libc::c_void, 1);
            libc::close(pipe_wr);
        }

        // Update state: running
        state.status = ContainerStatus::Running;
        state.pid = Some(child_pid);
        state.started_at = Some(unix_timestamp());
        save_state(&container_dir, &state)?;

        if restart_count == 0 {
            println!("Container '{}' started (PID {})", config.name, child_pid);
        }

        // Wait for the container process to exit (foreground or monitor)
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

        // Cleanup networking for this iteration (skip in rootless mode)
        if !config.rootless {
            if !config.ports.is_empty() && !container_ip.is_empty() {
                crate::network::cleanup_port_forwarding(&container_ip, &config.ports).ok();
            }
            if config.network_mode == "bridge" || is_named_network {
                crate::network::cleanup_container_network(&config.id).ok();
            }
        }

        // Check restart policy
        let should_restart = match &restart_policy {
            RestartPolicy::No => false,
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure(max) => exit_code != 0 && (*max == 0 || restart_count < *max),
        };

        if !should_restart {
            final_exit_code = exit_code;
            break;
        }

        restart_count += 1;
        println!("Container '{}' restarting (attempt {restart_count})...", config.name);
        std::thread::sleep(std::time::Duration::from_secs(1)); // Brief pause before restart
    }

    // Update state: stopped
    state.status = ContainerStatus::Stopped;
    state.finished_at = Some(unix_timestamp());
    state.exit_code = Some(final_exit_code);
    save_state(&container_dir, &state)?;

    // Cleanup named network registration (skip in rootless mode)
    if is_named_network && !config.rootless {
        crate::network::unregister_container_from_network(
            &config.network_mode,
            &config.name,
        )
        .ok();
    }

    // Cleanup OverlayFS mount
    crate::filesystem::cleanup_overlay(&overlay_dir).ok();

    // Cleanup cgroup (container state is preserved for inspection)
    if let Some(cg) = cgroup {
        cg.destroy().ok();
    }

    // Auto-remove if --rm was specified
    if config.auto_remove {
        fs::remove_dir_all(&container_dir).ok();
        log::info!("auto-removed container '{}'", config.name);
    }

    log::info!(
        "container '{}' exited with code {final_exit_code}",
        config.name
    );
    Ok(final_exit_code)
}

/// Set up UID/GID mappings for rootless containers.
///
/// Maps the current user's UID/GID to root (0) inside the container.
/// This allows the container process to appear as root inside its
/// user namespace while actually running as an unprivileged user.
fn setup_uid_gid_mappings(child_pid: i32) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    // Must deny setgroups before writing gid_map (kernel requirement)
    fs::write(
        format!("/proc/{child_pid}/setgroups"),
        "deny",
    )
    .context("failed to write setgroups deny")?;

    // Map current UID to root (0) inside the container
    // Format: <inside_id> <outside_id> <count>
    fs::write(
        format!("/proc/{child_pid}/uid_map"),
        format!("0 {uid} 1"),
    )
    .context("failed to write uid_map")?;

    // Map current GID to root (0) inside the container
    fs::write(
        format!("/proc/{child_pid}/gid_map"),
        format!("0 {gid} 1"),
    )
    .context("failed to write gid_map")?;

    log::info!("set up rootless UID/GID mappings: uid {uid} -> 0, gid {gid} -> 0");
    Ok(())
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

    // Clean up container network
    if config.network_mode == "bridge" {
        crate::network::cleanup_container_network(&config.id).ok();
    }

    // Clean up cgroup
    Cgroup::create(&config.id)
        .and_then(|c| c.destroy())
        .ok();

    println!("Container '{}' stopped.", config.name);
    Ok(())
}

/// Recursively copy a directory tree, preserving permissions.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in fs::read_dir(src)
        .with_context(|| format!("failed to read directory {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&src_path)?;
            std::os::unix::fs::symlink(&link_target, &dst_path).ok();
        } else {
            fs::copy(&src_path, &dst_path)
                .with_context(|| format!(
                    "failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                ))?;
        }
    }

    // Preserve directory permissions
    let metadata = fs::metadata(src)?;
    fs::set_permissions(dst, metadata.permissions())?;

    Ok(())
}
