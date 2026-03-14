//! VirtuRust CLI entry point.
//!
//! This binary provides the `virturust` command-line tool for managing
//! containers. See [`virturust`] (the library crate) for architecture details.

use anyhow::{anyhow, Result};
use clap::Parser;

use virturust::cli::{Cli, Commands};
use virturust::config::{self, parse_image_ref, parse_memory, ContainerConfig, ResourceLimits};
use virturust::image;

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
        Commands::Rm(args) => cmd_rm(args)?,
    }

    Ok(())
}

/// Execute the `run` subcommand — pull image if needed and start a container.
async fn cmd_run(args: virturust::cli::RunArgs) -> Result<()> {
    // Namespace creation requires root privileges
    if !nix::unistd::geteuid().is_root() {
        return Err(anyhow!(
            "virturust must be run as root (required for namespace creation).\n\
             Try: sudo virturust run ..."
        ));
    }

    let (name, tag) = parse_image_ref(&args.image);

    // Auto-pull if the image isn't available locally
    if !image::image_exists(name, tag) {
        println!("Image '{name}:{tag}' not found locally, pulling...");
        image::pull_image(name, tag).await?;
    }

    let rootfs = image::image_rootfs(name, tag);

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
    let command = if args.command.is_empty() {
        vec!["/bin/sh".to_string()]
    } else {
        args.command
    };

    let config = ContainerConfig {
        id,
        name: container_name,
        image: args.image,
        command,
        hostname,
        resources,
        rootfs,
    };

    let exit_code = virturust::container::run(&config)?;
    std::process::exit(exit_code);
}

/// Execute the `pull` subcommand — download an image from Docker Hub.
async fn cmd_pull(args: virturust::cli::PullArgs) -> Result<()> {
    let (name, tag) = parse_image_ref(&args.image);
    image::pull_image(name, tag).await?;
    Ok(())
}

/// Execute the `images` subcommand — list locally available images.
fn cmd_images() -> Result<()> {
    let images = image::list_images()?;

    if images.is_empty() {
        println!("No images found. Pull one with: virturust pull <image>");
        return Ok(());
    }

    println!("{:<30} {:<15}", "IMAGE", "TAG");
    println!("{}", "-".repeat(45));
    for (name, tag) in &images {
        println!("{:<30} {:<15}", name, tag);
    }

    Ok(())
}

/// Execute the `ps` subcommand — list containers.
fn cmd_ps() -> Result<()> {
    let dir = config::containers_dir();

    if !dir.exists() {
        println!("No containers found.");
        return Ok(());
    }

    let mut found = false;
    println!(
        "{:<14} {:<15} {:<20} {:<10}",
        "CONTAINER ID", "NAME", "IMAGE", "STATUS"
    );
    println!("{}", "-".repeat(60));

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let config_path = entry.path().join("config.json");
        if config_path.exists() {
            let data = std::fs::read_to_string(&config_path)?;
            if let Ok(cfg) = serde_json::from_str::<ContainerConfig>(&data) {
                found = true;
                println!(
                    "{:<14} {:<15} {:<20} {:<10}",
                    &cfg.id[..12],
                    cfg.name,
                    cfg.image,
                    "running"
                );
            }
        }
    }

    if !found {
        println!("No containers found.");
    }

    Ok(())
}

/// Execute the `rm` subcommand — remove a stopped container.
fn cmd_rm(args: virturust::cli::RmArgs) -> Result<()> {
    let dir = config::containers_dir().join(&args.name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
        println!("Removed container: {}", args.name);
    } else {
        println!("Container not found: {}", args.name);
    }
    Ok(())
}
