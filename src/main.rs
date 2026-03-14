//! Corten CLI entry point.
//!
//! This binary provides the `corten` command-line tool for managing
//! containers. See [`corten`] (the library crate) for architecture details.

use anyhow::{anyhow, Result};
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
    }

    Ok(())
}

/// Verify we have the privileges needed for container operations.
///
/// Accepts either root (sudo) or Linux capabilities set via `setcap`.
/// After `make install`, capabilities are set on the binary so sudo
/// is not required.
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
    require_privileges()?;

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

    // Resolve command: CLI args > image ENTRYPOINT + CMD
    let command = if !args.command.is_empty() {
        args.command
    } else if !img_config.entrypoint.is_empty() {
        // ENTRYPOINT + CMD (Docker semantics)
        let mut cmd = img_config.entrypoint.clone();
        cmd.extend(img_config.cmd.clone());
        cmd
    } else if !img_config.cmd.is_empty() {
        img_config.cmd.clone()
    } else {
        vec!["/bin/sh".to_string()]
    };

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
        env: img_config.env,
        working_dir: img_config.working_dir,
        user: img_config.user,
        network_mode: args.network,
        ports,
    };

    let exit_code = container::run(&config)?;
    std::process::exit(exit_code);
}

/// Execute the `pull` subcommand — download an image from Docker Hub.
async fn cmd_pull(args: corten::cli::PullArgs) -> Result<()> {
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
