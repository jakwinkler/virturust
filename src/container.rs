//! Container lifecycle management.
//!
//! This module orchestrates the full lifecycle of a container by combining
//! all isolation primitives:
//!
//! 1. **cgroups** — create resource limits before the container starts
//! 2. **clone()** — create the container process in new namespaces
//! 3. **sync** — signal the child after cgroup setup is complete
//! 4. **wait** — block until the container process exits
//! 5. **cleanup** — remove cgroup and container state

use anyhow::{Context, Result};
use std::fs;

use crate::cgroup::Cgroup;
use crate::config::{containers_dir, ContainerConfig};
use crate::namespace::{create_namespaced_process, ChildArgs};

/// Run a container with the given configuration.
///
/// This is the main entry point for container execution. It manages the
/// full lifecycle from creation to cleanup.
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

    // Create a pipe for parent-child synchronization.
    // The child blocks on the read end until the parent writes to the
    // write end, signaling that cgroup setup is complete.
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(anyhow::anyhow!(
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

    // Close the read end in the parent (only the child needs it)
    unsafe { libc::close(pipe_rd) };

    // Add the child to the cgroup (must happen before signaling the child)
    cgroup.add_process(child_pid)?;

    // Signal the child that cgroup setup is complete
    unsafe {
        libc::write(pipe_wr, [0u8].as_ptr() as *const libc::c_void, 1);
        libc::close(pipe_wr);
    }

    println!(
        "Container '{}' started (PID {})",
        config.name, child_pid
    );

    // Wait for the container process to exit
    let mut status: libc::c_int = 0;
    let ret = unsafe { libc::waitpid(child_pid, &mut status, 0) };
    if ret == -1 {
        log::error!(
            "waitpid failed: {}",
            std::io::Error::last_os_error()
        );
    }

    let exit_code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    };

    // Cleanup: remove cgroup and container state
    cgroup.destroy().ok();
    fs::remove_dir_all(&container_dir).ok();

    log::info!(
        "container '{}' exited with code {exit_code}",
        config.name
    );
    Ok(exit_code)
}
