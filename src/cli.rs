//! Command-line interface definitions for Corten.
//!
//! Uses `clap` with derive macros for a clean, self-documenting CLI.
//! All subcommands and their arguments are defined here.

use clap::{Parser, Subcommand};

/// Corten — A lightweight container runtime written in Rust.
///
/// Run Linux containers with minimal overhead using kernel namespaces,
/// cgroups v2, and pivot_root for filesystem isolation.
#[derive(Parser, Debug)]
#[command(name = "corten", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose/debug logging
    #[arg(long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a container from an image
    Run(RunArgs),

    /// Pull an image from Docker Hub
    Pull(PullArgs),

    /// List locally available images
    Images,

    /// List containers (running and stopped)
    Ps,

    /// Stop a running container
    Stop(StopArgs),

    /// Show detailed information about a container
    Inspect(InspectArgs),

    /// Remove a stopped container
    Rm(RmArgs),

    /// Manage networks
    Network(NetworkArgs),
}

/// Arguments for the `run` subcommand.
#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Image to run (e.g., "alpine", "ubuntu:22.04", "debian:bookworm")
    pub image: String,

    /// Command and arguments to execute inside the container.
    /// Defaults to /bin/sh if not specified.
    #[arg(trailing_var_arg = true)]
    pub command: Vec<String>,

    /// Memory limit (e.g., "128m", "1g", "512k").
    /// Enforced as a hard limit via cgroups v2 — the OOM killer will
    /// terminate processes if the container exceeds this.
    #[arg(short, long)]
    pub memory: Option<String>,

    /// CPU limit as fractional CPUs (e.g., 0.5 = half a core, 2.0 = two cores).
    /// Enforced via cgroups v2 cpu.max bandwidth control.
    #[arg(short, long)]
    pub cpus: Option<f64>,

    /// Maximum number of processes allowed inside the container.
    /// Prevents fork bombs and runaway process creation.
    #[arg(long)]
    pub pids_limit: Option<u64>,

    /// Container hostname (defaults to a short container ID)
    #[arg(long)]
    pub hostname: Option<String>,

    /// Human-readable container name (defaults to a short container ID)
    #[arg(long)]
    pub name: Option<String>,

    /// Bind mount a host directory into the container.
    /// Format: /host/path:/container/path[:ro]
    #[arg(short = 'v', long = "volume")]
    pub volumes: Vec<String>,

    /// Network mode: bridge (default), none, or host
    #[arg(long, default_value = "bridge")]
    pub network: String,

    /// Publish a container port to the host.
    /// Format: host_port:container_port or ip:host_port:container_port
    #[arg(short = 'p', long = "publish")]
    pub publish: Vec<String>,
}

/// Arguments for the `pull` subcommand.
#[derive(clap::Args, Debug)]
pub struct PullArgs {
    /// Image to pull (e.g., "alpine", "ubuntu:22.04", "debian:bookworm")
    pub image: String,
}

/// Arguments for the `stop` subcommand.
#[derive(clap::Args, Debug)]
pub struct StopArgs {
    /// Container name or ID to stop
    pub name: String,

    /// Seconds to wait before sending SIGKILL (default: 10)
    #[arg(short, long, default_value = "10")]
    pub time: u64,
}

/// Arguments for the `inspect` subcommand.
#[derive(clap::Args, Debug)]
pub struct InspectArgs {
    /// Container name or ID to inspect
    pub name: String,
}

/// Arguments for the `rm` subcommand.
#[derive(clap::Args, Debug)]
pub struct RmArgs {
    /// Container name or ID to remove
    pub name: String,
}

/// Arguments for the `network` subcommand.
#[derive(clap::Args, Debug)]
pub struct NetworkArgs {
    #[command(subcommand)]
    pub command: NetworkCommands,
}

#[derive(Subcommand, Debug)]
pub enum NetworkCommands {
    /// Create a named network
    Create(NetworkCreateArgs),
    /// List networks
    Ls,
    /// Remove a network
    Rm(NetworkRmArgs),
}

/// Arguments for `network create`.
#[derive(clap::Args, Debug)]
pub struct NetworkCreateArgs {
    /// Name for the network
    pub name: String,
}

/// Arguments for `network rm`.
#[derive(clap::Args, Debug)]
pub struct NetworkRmArgs {
    /// Name of the network to remove
    pub name: String,
}
