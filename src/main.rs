//! Corten CLI entry point.
//!
//! This binary provides the `corten` command-line tool for managing
//! containers. See [`corten`] (the library crate) for architecture details.

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use corten::cli::{Cli, Commands};
use corten::config::{
    self, has_cap_sys_admin, parse_image_ref, parse_memory, parse_port, parse_volume,
    ContainerConfig, ContainerStatus, ResourceLimits,
};
use corten::container;
use corten::image;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Save the real user who invoked corten (before privilege elevation).
    // Used for per-user container isolation.
    // If CORTEN_REAL_UID is already set (e.g., by tests or wrapper scripts),
    // use that instead of the actual UID — allows testing user isolation.
    let real_uid: u32 = std::env::var("CORTEN_REAL_UID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });
    let real_gid: u32 = std::env::var("CORTEN_REAL_GID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| unsafe { libc::getgid() });

    // Check if user is in the 'corten' group (or is root).
    // If the 'corten' group exists, only members can use corten.
    // If it doesn't exist, anyone can use it (backwards compatible).
    if real_uid != 0 {
        if let Some(required_gid) = corten_group_gid() {
            if !user_in_group(real_uid, required_gid) {
                eprintln!("Permission denied: user is not in the 'corten' group.");
                eprintln!("");
                eprintln!("Add yourself:  sudo usermod -aG corten $(whoami)");
                eprintln!("Then log out and back in.");
                std::process::exit(1);
            }
        }
    }

    // Elevate to root if we have CAP_SETUID (from make install).
    // This allows all child processes (ip, iptables, nsenter, chroot)
    // and netlink operations to work without sudo.
    unsafe {
        let r1 = libc::setresgid(0, 0, 0);
        let r2 = libc::setresuid(0, 0, 0);
        if r1 != 0 || r2 != 0 {
            log::debug!("setresuid(0) failed — running without root elevation");
        }
    }

    // Store real UID for per-user container isolation
    unsafe {
        std::env::set_var("CORTEN_REAL_UID", real_uid.to_string());
        std::env::set_var("CORTEN_REAL_GID", real_gid.to_string());
    }

    // Configure logging — default to "info", use "debug" with --verbose
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp(None)
        .init();

    match cli.command {
        Commands::Run(args) => cmd_run(args).await?,
        Commands::Pull(args) => cmd_pull(args).await?,
        Commands::Images => cmd_images()?,
        Commands::Ps => cmd_ps()?,
        Commands::Stop(args) => cmd_stop(args)?,
        Commands::Inspect(args) => cmd_inspect(args)?,
        Commands::Rm(args) => cmd_rm(args)?,
        Commands::Network(args) => cmd_network(args)?,
        Commands::Logs(args) => cmd_logs(args)?,
        Commands::Exec(args) => cmd_exec(args)?,
        Commands::Build(args) => cmd_build(args).await?,
        Commands::Image(args) => cmd_image(args)?,
        Commands::System(args) => cmd_system(args)?,
        Commands::Stats(args) => cmd_stats(args)?,
        Commands::Kill(args) => cmd_kill(args)?,
        Commands::Cp(args) => cmd_cp(args)?,
        Commands::Forge(args) => cmd_forge(args).await?,
    }

    Ok(())
}

/// Verify we have the privileges needed for container operations.
///
/// Accepts either root (sudo) or Linux capabilities set via `setcap`.
/// After `make install`, capabilities are set on the binary so sudo
/// is not required.
/// Get the GID of the 'corten' group, if it exists.
fn corten_group_gid() -> Option<u32> {
    use std::ffi::CStr;
    let name = std::ffi::CString::new("corten").ok()?;
    let grp = unsafe { libc::getgrnam(name.as_ptr()) };
    if grp.is_null() {
        None // Group doesn't exist — no restriction
    } else {
        Some(unsafe { (*grp).gr_gid })
    }
}

/// Check if a user is in a specific group.
fn user_in_group(uid: u32, target_gid: u32) -> bool {
    // Check primary group
    let pw = unsafe { libc::getpwuid(uid) };
    if !pw.is_null() && unsafe { (*pw).pw_gid } == target_gid {
        return true;
    }

    // Check supplementary groups
    let mut ngroups: libc::c_int = 64;
    let mut groups = vec![0u32; ngroups as usize];
    if !pw.is_null() {
        let username = unsafe { (*pw).pw_name };
        unsafe {
            libc::getgrouplist(
                username,
                (*pw).pw_gid as libc::gid_t,
                groups.as_mut_ptr() as *mut libc::gid_t,
                &mut ngroups,
            );
        }
        groups.truncate(ngroups as usize);
        return groups.contains(&target_gid);
    }
    false
}

fn require_privileges() -> Result<()> {
    if has_cap_sys_admin() {
        return Ok(());
    }

    Err(anyhow!(
        "insufficient privileges for container operations.\n\n\
         Option 1 (recommended): Install with capabilities (one-time sudo):\n\
         \x20 make install\n\n\
         Option 2: Run with sudo:\n\
         \x20 sudo corten run ..."
    ))
}

/// Execute the `run` subcommand — pull image if needed and start a container.
async fn cmd_run(args: corten::cli::RunArgs) -> Result<()> {
    if !args.rootless {
        require_privileges()?;
    }

    let detach = args.detach;
    let (name, tag) = parse_image_ref(&args.image);

    // Auto-pull if the image isn't available locally
    if !image::image_exists(name, tag) {
        println!("Image '{name}:{tag}' not found locally, pulling...");
        image::pull_image(name, tag).await?;
    }

    let rootfs = image::image_rootfs(name, tag);

    // Load OCI image config (ENV, CMD, ENTRYPOINT, WORKDIR, USER)
    let img_config = image::load_image_config(name, tag);

    // Parse resource limits from CLI arguments
    let memory_bytes = args.memory.as_deref().map(parse_memory).transpose()?;

    let resources = ResourceLimits {
        memory_bytes,
        cpu_quota: args.cpus,
        pids_max: args.pids_limit,
    };

    // Generate unique container ID and default name/hostname
    let id = uuid::Uuid::new_v4().to_string();
    let container_name = args.name.unwrap_or_else(|| id[..12].to_string());
    let hostname = args.hostname.unwrap_or_else(|| id[..12].to_string());

    // Resolve command: CLI args > entrypoint override > image ENTRYPOINT + CMD
    let entrypoint_override = args.entrypoint.clone();
    let command = if let Some(ref ep) = entrypoint_override {
        let mut cmd = vec![ep.clone()];
        cmd.extend(args.command);
        cmd
    } else if !args.command.is_empty() {
        args.command
    } else if !img_config.entrypoint.is_empty() {
        let mut cmd = img_config.entrypoint.clone();
        cmd.extend(img_config.cmd.clone());
        cmd
    } else if !img_config.cmd.is_empty() {
        img_config.cmd.clone()
    } else {
        vec!["/bin/sh".to_string()]
    };

    // Merge environment: image ENV + CLI -e flags + --env-file
    let mut env = img_config.env;
    if let Some(env_file) = &args.env_file {
        let content = std::fs::read_to_string(env_file)
            .with_context(|| format!("failed to read env file: {env_file}"))?;
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                env.push(line.to_string());
            }
        }
    }
    env.extend(args.env);

    // Parse volume mounts
    let volumes = args
        .volumes
        .iter()
        .map(|v| parse_volume(v))
        .collect::<Result<Vec<_>>>()?;

    // Parse port mappings
    let ports = args
        .publish
        .iter()
        .map(|p| parse_port(p))
        .collect::<Result<Vec<_>>>()?;

    let config = ContainerConfig {
        id,
        name: container_name,
        image: args.image,
        command,
        hostname,
        resources,
        rootfs,
        volumes,
        env,
        working_dir: img_config.working_dir,
        user: img_config.user,
        network_mode: args.network,
        ports,
        restart_policy: args.restart,
        rootless: args.rootless,
        privileged: args.privileged,
        read_only: args.read_only,
        auto_remove: args.rm,
    };

    let exit_code = container::run(&config, detach)?;
    if !detach {
        std::process::exit(exit_code);
    }
    // In detach mode, run() returns 0 immediately
    Ok(())
}

/// Execute the `pull` subcommand — download an image from official distro mirrors.
async fn cmd_pull(args: corten::cli::PullArgs) -> Result<()> {
    require_privileges()?;
    let (name, tag) = parse_image_ref(&args.image);
    image::pull_image(name, tag).await?;
    Ok(())
}

/// Execute the `images` subcommand — list locally available images.
fn cmd_images() -> Result<()> {
    let images = image::list_images()?;

    if images.is_empty() {
        println!("No images found. Pull one with: corten pull <image>");
        return Ok(());
    }

    println!("{:<30} {:<15}", "IMAGE", "TAG");
    println!("{}", "-".repeat(45));
    for (name, tag) in &images {
        println!("{:<30} {:<15}", name, tag);
    }

    Ok(())
}

/// Execute the `ps` subcommand — list all containers with live status.
fn cmd_ps() -> Result<()> {
    let dir = config::containers_dir();

    if !dir.exists() {
        println!("No containers found.");
        return Ok(());
    }

    let mut found = false;
    println!(
        "{:<14} {:<15} {:<20} {:<10} {:<10}",
        "CONTAINER ID", "NAME", "IMAGE", "STATUS", "EXIT CODE"
    );
    println!("{}", "-".repeat(70));

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        let Ok(cfg) = container::load_config(&path) else {
            continue;
        };
        let Ok(mut state) = container::load_state(&path) else {
            continue;
        };

        // Check if a "running" container is actually still alive
        if state.status == ContainerStatus::Running {
            if let Some(pid) = state.pid {
                if !container::is_process_alive(pid) {
                    state.status = ContainerStatus::Stopped;
                }
            }
        }

        found = true;
        let exit_str = state
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<14} {:<15} {:<20} {:<10} {:<10}",
            &cfg.id[..12],
            cfg.name,
            cfg.image,
            state.status,
            exit_str
        );
    }

    if !found {
        println!("No containers found.");
    }

    Ok(())
}

/// Execute the `stop` subcommand — stop a running container.
fn cmd_stop(args: corten::cli::StopArgs) -> Result<()> {
    require_privileges()?;

    let container_dir = container::find_container(&args.name)?;
    container::stop(&container_dir, args.time)
}

/// Execute the `inspect` subcommand — show detailed container info.
fn cmd_inspect(args: corten::cli::InspectArgs) -> Result<()> {
    let container_dir = container::find_container(&args.name)?;
    let cfg = container::load_config(&container_dir)?;
    let mut state = container::load_state(&container_dir)?;

    // Live status check
    if state.status == ContainerStatus::Running {
        if let Some(pid) = state.pid {
            if !container::is_process_alive(pid) {
                state.status = ContainerStatus::Stopped;
            }
        }
    }

    println!("Container:    {}", cfg.id);
    println!("Name:         {}", cfg.name);
    println!("Image:        {}", cfg.image);
    println!("Command:      {:?}", cfg.command);
    println!("Hostname:     {}", cfg.hostname);
    println!("Status:       {}", state.status);
    println!("PID:          {}", state.pid.map(|p| p.to_string()).unwrap_or("-".into()));
    println!("Created:      {}", state.created_at);
    println!("Started:      {}", state.started_at.map(|t| t.to_string()).unwrap_or("-".into()));
    println!("Finished:     {}", state.finished_at.map(|t| t.to_string()).unwrap_or("-".into()));
    println!("Exit code:    {}", state.exit_code.map(|c| c.to_string()).unwrap_or("-".into()));
    println!();
    println!("Resource limits:");
    println!("  Memory:     {}", cfg.resources.memory_bytes
        .map(|b| format_bytes(b))
        .unwrap_or("unlimited".into()));
    println!("  CPUs:       {}", cfg.resources.cpu_quota
        .map(|c| format!("{c}"))
        .unwrap_or("unlimited".into()));
    println!("  PIDs max:   {}", cfg.resources.pids_max
        .map(|p| p.to_string())
        .unwrap_or("unlimited".into()));
    if !cfg.env.is_empty() {
        println!();
        println!("Environment:");
        for var in &cfg.env {
            println!("  {var}");
        }
    }
    if !cfg.working_dir.is_empty() {
        println!("WorkingDir:   {}", cfg.working_dir);
    }
    if !cfg.user.is_empty() {
        println!("User:         {}", cfg.user);
    }
    println!("Network:      {}", cfg.network_mode);
    println!("Restart:      {}", cfg.restart_policy);
    if cfg.rootless {
        println!("Rootless:     yes");
    }
    if !cfg.ports.is_empty() {
        println!();
        println!("Ports:");
        for port in &cfg.ports {
            println!(
                "  {}:{} -> {}",
                port.host_ip, port.host_port, port.container_port
            );
        }
    }
    if !cfg.volumes.is_empty() {
        println!();
        println!("Volumes:");
        for vol in &cfg.volumes {
            let mode = if vol.read_only { "ro" } else { "rw" };
            println!(
                "  {} -> {} ({})",
                vol.host_path.display(),
                vol.container_path.display(),
                mode
            );
        }
    }
    println!();
    println!("Rootfs:       {}", cfg.rootfs.display());

    Ok(())
}

/// Format bytes into a human-readable string.
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Execute the `rm` subcommand — remove a stopped container.
fn cmd_rm(args: corten::cli::RmArgs) -> Result<()> {
    let container_dir = container::find_container(&args.name)?;
    let cfg = container::load_config(&container_dir)?;

    // Don't remove running containers
    if let Ok(state) = container::load_state(&container_dir) {
        if state.status == ContainerStatus::Running {
            if let Some(pid) = state.pid {
                if container::is_process_alive(pid) {
                    return Err(anyhow!(
                        "cannot remove running container '{}'. Stop it first: corten stop {}",
                        cfg.name,
                        cfg.name
                    ));
                }
            }
        }
    }

    std::fs::remove_dir_all(&container_dir)?;
    println!("Removed container: {} ({})", cfg.name, &cfg.id[..12]);
    Ok(())
}

/// Execute the `logs` subcommand — view container logs.
fn cmd_logs(args: corten::cli::LogsArgs) -> Result<()> {
    let container_dir = container::find_container(&args.name)?;
    let stdout_log = container_dir.join("stdout.log");

    if !stdout_log.exists() {
        println!("No logs available for this container.");
        return Ok(());
    }

    if args.follow {
        // Stream the file (like tail -f)
        use std::io::{BufRead, BufReader, Seek, SeekFrom};
        let file = std::fs::File::open(&stdout_log)?;
        let mut reader = BufReader::new(file);
        // Seek to end and then poll for new content
        reader.seek(SeekFrom::End(0))?;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Ok(_) => print!("{line}"),
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        // Read last N lines
        let content = std::fs::read_to_string(&stdout_log)?;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(args.tail);
        for line in &lines[start..] {
            println!("{line}");
        }
    }

    Ok(())
}

/// Execute the `exec` subcommand — run a command in a running container.
fn cmd_exec(args: corten::cli::ExecArgs) -> Result<()> {
    require_privileges()?;

    let container_dir = container::find_container(&args.name)?;
    let state = container::load_state(&container_dir)?;

    if state.status != ContainerStatus::Running {
        return Err(anyhow!("container '{}' is not running", args.name));
    }

    let pid = state
        .pid
        .ok_or_else(|| anyhow!("container has no PID"))?;

    if !container::is_process_alive(pid) {
        return Err(anyhow!(
            "container process (PID {pid}) is no longer running"
        ));
    }

    // Use nsenter to enter the container's namespaces and run the command
    let mut cmd = std::process::Command::new("nsenter");
    cmd.args([
        "--target",
        &pid.to_string(),
        "--mount",
        "--uts",
        "--ipc",
        "--net",
        "--pid",
        "--",
    ]);
    cmd.args(&args.command);

    let status = cmd
        .status()
        .context("failed to execute nsenter")?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Execute the `build` subcommand — build an image from Corten.toml.
async fn cmd_build(args: corten::cli::BuildArgs) -> Result<()> {
    use corten::build;

    let path = std::path::Path::new(&args.path);
    let toml_path = if path.is_dir() {
        // Look for Corten.toml, Corten.json, or Corten.jsonc
        if path.join("Corten.toml").exists() {
            path.join("Corten.toml")
        } else if path.join("Corten.jsonc").exists() {
            path.join("Corten.jsonc")
        } else if path.join("Corten.json").exists() {
            path.join("Corten.json")
        } else {
            path.join("Corten.toml") // default, will error below
        }
    } else {
        path.to_path_buf()
    };

    if !toml_path.exists() {
        return Err(anyhow!("Build config not found at {} (supports .toml, .json, .jsonc)", toml_path.display()));
    }

    let config = build::parse_build_config(&toml_path)?;
    build::validate_build_config(&config)?;

    if args.dry_run {
        build::print_build_plan(&config);
        return Ok(());
    }

    require_privileges()?;

    let toml_dir = toml_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    build::build_image(&config, toml_dir).await?;

    Ok(())
}

/// Execute the `network` subcommand — manage named networks.
fn cmd_network(args: corten::cli::NetworkArgs) -> Result<()> {
    use corten::cli::NetworkCommands;
    use corten::network;

    match args.command {
        NetworkCommands::Create(create_args) => {
            require_privileges()?;
            let info = network::create_network(&create_args.name)?;
            println!(
                "Created network '{}' (bridge={}, subnet={})",
                info.name, info.bridge, info.subnet
            );
        }
        NetworkCommands::Ls => {
            let networks = network::list_networks()?;
            if networks.is_empty() {
                println!("No networks found. Create one with: corten network create <name>");
            } else {
                println!("{:<20} {:<15} {:<20} {}", "NAME", "BRIDGE", "SUBNET", "CONTAINERS");
                println!("{}", "-".repeat(65));
                for net in &networks {
                    println!(
                        "{:<20} {:<15} {:<20} {}",
                        net.name, net.bridge, net.subnet, net.containers.len()
                    );
                }
            }
        }
        NetworkCommands::Rm(rm_args) => {
            require_privileges()?;
            network::remove_network(&rm_args.name)?;
            println!("Removed network '{}'", rm_args.name);
        }
    }
    Ok(())
}

/// Execute the `image` subcommand — manage images.
fn cmd_image(args: corten::cli::ImageSubArgs) -> Result<()> {
    match args.command {
        corten::cli::ImageCommands::Prune => cmd_image_prune()?,
    }
    Ok(())
}

/// Remove all locally stored images.
fn cmd_image_prune() -> Result<()> {
    let images = image::list_images()?;
    if images.is_empty() {
        println!("No images to prune.");
        return Ok(());
    }

    let mut removed = 0;
    for (name, tag) in &images {
        let rootfs = image::image_rootfs(name, tag);
        // The tag directory contains rootfs/ and config.json — remove the whole tag dir
        if let Some(parent) = rootfs.parent() {
            std::fs::remove_dir_all(parent).ok();
            removed += 1;
            println!("Removed: {name}:{tag}");
        }
    }
    println!("Removed {removed} image(s).");
    Ok(())
}

/// Execute the `system` subcommand — system maintenance.
fn cmd_system(args: corten::cli::SystemSubArgs) -> Result<()> {
    match args.command {
        corten::cli::SystemCommands::Prune => cmd_system_prune()?,
    }
    Ok(())
}

/// Execute the `stats` subcommand — show live resource usage.
fn cmd_stats(args: corten::cli::StatsArgs) -> Result<()> {
    loop {
        // Clear screen for streaming mode
        if !args.no_stream {
            print!("\x1b[2J\x1b[H"); // ANSI clear + home
        }

        println!("{:<14} {:<10} {:<20} {:<15} {:<10}",
            "CONTAINER", "CPU %", "MEMORY", "MEM %", "PIDS");
        println!("{}", "-".repeat(70));

        let containers_dir = config::containers_dir();
        if containers_dir.exists() {
            for entry in std::fs::read_dir(&containers_dir)? {
                let entry = entry?;
                let path = entry.path();
                let Ok(cfg) = container::load_config(&path) else { continue };
                let Ok(state) = container::load_state(&path) else { continue };

                if state.status != ContainerStatus::Running { continue }
                if let Some(ref name) = args.name {
                    if cfg.name != *name && !cfg.id.starts_with(name) { continue }
                }

                let pid = state.pid.unwrap_or(0);
                if pid == 0 || !container::is_process_alive(pid) { continue }

                // Read cgroup stats
                let cgroup_path = format!("/sys/fs/cgroup/corten/{}", cfg.id);

                let mem_current = std::fs::read_to_string(format!("{cgroup_path}/memory.current"))
                    .unwrap_or_default().trim().parse::<u64>().unwrap_or(0);
                let mem_max = std::fs::read_to_string(format!("{cgroup_path}/memory.max"))
                    .unwrap_or_default().trim().parse::<u64>().unwrap_or(0);

                let pids_current = std::fs::read_to_string(format!("{cgroup_path}/pids.current"))
                    .unwrap_or_default().trim().parse::<u64>().unwrap_or(0);

                // Read CPU usage from cpu.stat
                let cpu_stat = std::fs::read_to_string(format!("{cgroup_path}/cpu.stat"))
                    .unwrap_or_default();
                let cpu_usage = cpu_stat.lines()
                    .find(|l| l.starts_with("usage_usec"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);

                let mem_str = format_bytes_short(mem_current);
                let mem_limit_str = if mem_max > 0 && mem_max < u64::MAX / 2 {
                    format!("{} / {}", mem_str, format_bytes_short(mem_max))
                } else {
                    format!("{} / --", mem_str)
                };
                let mem_pct = if mem_max > 0 && mem_max < u64::MAX / 2 {
                    format!("{:.1}%", mem_current as f64 / mem_max as f64 * 100.0)
                } else {
                    "--".to_string()
                };

                let cpu_str = format!("{:.2}s", cpu_usage as f64 / 1_000_000.0);

                println!("{:<14} {:<10} {:<20} {:<15} {:<10}",
                    &cfg.name[..cfg.name.len().min(13)],
                    cpu_str,
                    mem_limit_str,
                    mem_pct,
                    pids_current);
            }
        }

        if args.no_stream || args.name.is_none() && !containers_dir_has_running(&config::containers_dir()) {
            break;
        }

        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Ok(())
}

fn containers_dir_has_running(dir: &std::path::Path) -> bool {
    if !dir.exists() { return false; }
    std::fs::read_dir(dir).ok()
        .map(|entries| entries.filter_map(|e| e.ok())
            .any(|e| container::load_state(&e.path())
                .map(|s| s.status == ContainerStatus::Running).unwrap_or(false)))
        .unwrap_or(false)
}

fn format_bytes_short(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

/// Execute the `kill` subcommand — send a signal to a running container.
fn cmd_kill(args: corten::cli::KillArgs) -> Result<()> {
    require_privileges()?;
    let container_dir = container::find_container(&args.name)?;
    let state = container::load_state(&container_dir)?;
    let pid = state.pid.ok_or_else(|| anyhow!("container has no PID"))?;

    if !container::is_process_alive(pid) {
        return Err(anyhow!("container is not running"));
    }

    let signal = match args.signal.to_uppercase().as_str() {
        "KILL" | "SIGKILL" | "9" => libc::SIGKILL,
        "TERM" | "SIGTERM" | "15" => libc::SIGTERM,
        "HUP" | "SIGHUP" | "1" => libc::SIGHUP,
        "INT" | "SIGINT" | "2" => libc::SIGINT,
        "QUIT" | "SIGQUIT" | "3" => libc::SIGQUIT,
        "USR1" | "SIGUSR1" | "10" => libc::SIGUSR1,
        "USR2" | "SIGUSR2" | "12" => libc::SIGUSR2,
        "STOP" | "SIGSTOP" | "19" => libc::SIGSTOP,
        "CONT" | "SIGCONT" | "18" => libc::SIGCONT,
        other => return Err(anyhow!("unknown signal: {other}")),
    };

    unsafe { libc::kill(pid, signal) };
    let cfg = container::load_config(&container_dir)?;
    println!("Sent {} to container '{}'", args.signal.to_uppercase(), cfg.name);
    Ok(())
}

/// Execute the `cp` subcommand — copy files between container and host.
fn cmd_cp(args: corten::cli::CpArgs) -> Result<()> {
    // Parse src and dst — one must be container:path, other is host path
    let (container_name, container_path, host_path, to_container) =
        if let Some((name, path)) = args.src.split_once(':') {
            (name, path, &args.dst, false)
        } else if let Some((name, path)) = args.dst.split_once(':') {
            (name, path, &args.src, true)
        } else {
            return Err(anyhow!(
                "one of src/dst must be container:path format\n\
                 Usage: corten cp <container>:/path /host/path\n\
                        corten cp /host/path <container>:/path"
            ));
        };

    let container_dir = container::find_container(container_name)?;
    let cfg = container::load_config(&container_dir)?;

    // Find the container's rootfs (overlay merged or copy)
    let overlay_merged = container_dir.join("overlay").join("merged");
    let rootfs_copy = container_dir.join("rootfs");
    let rootfs = if overlay_merged.exists() {
        overlay_merged
    } else if rootfs_copy.exists() {
        rootfs_copy
    } else {
        cfg.rootfs.clone()
    };

    let full_container_path = rootfs.join(container_path.trim_start_matches('/'));

    if to_container {
        // Host → Container
        let src = std::path::Path::new(host_path);
        if src.is_dir() {
            copy_dir_for_cp(src, &full_container_path)?;
        } else {
            if let Some(parent) = full_container_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src, &full_container_path)
                .with_context(|| format!("failed to copy to container"))?;
        }
        println!("Copied {} -> {}:{}", host_path, container_name, container_path);
    } else {
        // Container → Host
        let dst = std::path::Path::new(host_path);
        if full_container_path.is_dir() {
            copy_dir_for_cp(&full_container_path, dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&full_container_path, dst)
                .with_context(|| format!("failed to copy from container"))?;
        }
        println!("Copied {}:{} -> {}", container_name, container_path, host_path);
    }

    Ok(())
}

fn copy_dir_for_cp(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_for_cp(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

/// Remove stopped containers and unused images.
fn cmd_system_prune() -> Result<()> {
    let containers_dir = config::containers_dir();
    let mut removed_containers = 0;
    if containers_dir.exists() {
        for entry in std::fs::read_dir(&containers_dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Ok(state) = container::load_state(&path) {
                if state.status == ContainerStatus::Stopped {
                    std::fs::remove_dir_all(&path).ok();
                    removed_containers += 1;
                }
            }
        }
    }
    println!("Removed {removed_containers} stopped container(s).");

    // Clean up stale network resources
    corten::network::flush_port_forwarding();
    corten::network::cleanup_stale_veths();

    // Then prune images
    cmd_image_prune()?;
    Ok(())
}

async fn cmd_forge(args: corten::cli::ComposeArgs) -> Result<()> {
    use corten::compose;
    use corten::cli::ComposeCommands;

    let compose_path = std::path::Path::new(&args.file);

    match args.command {
        ComposeCommands::Up(_up_args) => {
            require_privileges()?;

            if !compose_path.exists() {
                return Err(anyhow!("compose file not found: {}", compose_path.display()));
            }

            let comp = compose::parse_forge_file(compose_path)?;
            let order = compose::resolve_order(&comp)?;

            println!("Starting {} services...", comp.services.len());
            compose::print_forge_summary(&comp);
            println!();

            // Create project network
            let project_name = compose_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("corten");
            let net_name = format!("{project_name}_default");
            corten::network::create_network(&net_name).ok(); // ignore if exists

            // Start services in dependency order
            for svc_name in &order {
                let svc = &comp.services[svc_name];
                let image = svc.image.as_deref().unwrap_or("alpine");
                let (img_name, img_tag) = corten::config::parse_image_ref(image);

                // Auto-pull if needed
                if !corten::image::image_exists(img_name, img_tag) {
                    println!("Pulling {image}...");
                    corten::image::pull_image(img_name, img_tag).await?;
                }

                let container_name = svc.container_name.clone()
                    .unwrap_or_else(|| format!("{project_name}-{svc_name}"));

                let rootfs = corten::image::image_rootfs(img_name, img_tag);
                let img_config = corten::image::load_image_config(img_name, img_tag);

                let command = svc.command.clone()
                    .or_else(|| if !img_config.cmd.is_empty() { Some(img_config.cmd.clone()) } else { None })
                    .unwrap_or_else(|| vec!["/bin/sh".to_string()]);

                let memory_bytes = svc.memory.as_deref()
                    .map(corten::config::parse_memory)
                    .transpose()?;

                let cpu_quota = svc.cpus.as_deref()
                    .and_then(|c| c.parse::<f64>().ok());

                let volumes = svc.volumes.iter()
                    .map(|v| corten::config::parse_volume(v))
                    .collect::<Result<Vec<_>>>()?;

                let ports = svc.ports.iter()
                    .map(|p| corten::config::parse_port(p))
                    .collect::<Result<Vec<_>>>()?;

                let id = uuid::Uuid::new_v4().to_string();

                let cfg = config::ContainerConfig {
                    id,
                    name: container_name.clone(),
                    image: image.to_string(),
                    command,
                    hostname: svc.hostname.clone().unwrap_or_else(|| svc_name.clone()),
                    resources: config::ResourceLimits {
                        memory_bytes,
                        cpu_quota,
                        pids_max: None,
                    },
                    rootfs,
                    volumes,
                    env: {
                        let mut env = img_config.env;
                        for (k, v) in &svc.env {
                            env.push(format!("{k}={v}"));
                        }
                        env
                    },
                    working_dir: svc.working_dir.clone().unwrap_or(img_config.working_dir),
                    user: svc.user.clone().unwrap_or(img_config.user),
                    network_mode: svc.network.clone().unwrap_or_else(|| net_name.clone()),
                    ports,
                    restart_policy: svc.restart.clone().unwrap_or_else(|| "no".to_string()),
                    rootless: false,
                    privileged: svc.privileged,
                    read_only: svc.read_only,
                    auto_remove: false,
                };

                println!("  Starting {svc_name} ({image})...");
                container::run(&cfg, true)?;
                println!("  Started {container_name}");
            }

            println!("\nAll services started.");
        }

        ComposeCommands::Down => {
            require_privileges()?;

            if !compose_path.exists() {
                return Err(anyhow!("compose file not found: {}", compose_path.display()));
            }

            let comp = compose::parse_forge_file(compose_path)?;
            let order = compose::resolve_order(&comp)?;

            let project_name = compose_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("corten");

            // Stop in reverse dependency order
            for svc_name in order.iter().rev() {
                let container_name = comp.services[svc_name].container_name.clone()
                    .unwrap_or_else(|| format!("{project_name}-{svc_name}"));

                if let Ok(dir) = container::find_container(&container_name) {
                    println!("  Stopping {container_name}...");
                    container::stop(&dir, 10).ok();
                    std::fs::remove_dir_all(&dir).ok();
                    println!("  Removed {container_name}");
                }
            }

            // Remove project network
            let net_name = format!("{project_name}_default");
            corten::network::remove_network(&net_name).ok();

            println!("All services stopped and removed.");
        }

        ComposeCommands::Ps => {
            if !compose_path.exists() {
                return Err(anyhow!("compose file not found: {}", compose_path.display()));
            }

            let comp = compose::parse_forge_file(compose_path)?;
            let project_name = compose_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("corten");

            println!("{:<20} {:<15} {:<10}", "SERVICE", "CONTAINER", "STATUS");
            println!("{}", "-".repeat(50));

            for svc_name in comp.services.keys() {
                let container_name = comp.services[svc_name].container_name.clone()
                    .unwrap_or_else(|| format!("{project_name}-{svc_name}"));

                let status = if let Ok(dir) = container::find_container(&container_name) {
                    if let Ok(state) = container::load_state(&dir) {
                        state.status.to_string()
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "not created".to_string()
                };

                println!("{:<20} {:<15} {:<10}", svc_name, container_name, status);
            }
        }

        ComposeCommands::Logs(log_args) => {
            if !compose_path.exists() {
                return Err(anyhow!("compose file not found: {}", compose_path.display()));
            }

            let comp = compose::parse_forge_file(compose_path)?;
            let project_name = compose_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("corten");

            for svc_name in comp.services.keys() {
                if let Some(ref target) = log_args.service {
                    if svc_name != target { continue; }
                }

                let container_name = comp.services[svc_name].container_name.clone()
                    .unwrap_or_else(|| format!("{project_name}-{svc_name}"));

                if let Ok(dir) = container::find_container(&container_name) {
                    let log_file = dir.join("stdout.log");
                    if log_file.exists() {
                        let content = std::fs::read_to_string(&log_file)?;
                        for line in content.lines() {
                            println!("{svc_name} | {line}");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
