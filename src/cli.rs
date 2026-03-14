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

    /// View container logs
    Logs(LogsArgs),

    /// Execute a command in a running container
    Exec(ExecArgs),

    /// Build an image from a Corten.toml file
    Build(BuildArgs),

    /// Manage images
    Image(ImageSubArgs),

    /// System maintenance
    System(SystemSubArgs),

    /// Show live resource usage statistics
    Stats(StatsArgs),

    /// Send a signal to a running container
    Kill(KillArgs),

    /// Copy files between container and host
    Cp(CpArgs),
}

#[derive(clap::Args, Debug)]
pub struct StatsArgs {
    /// Container name or ID (shows all if omitted)
    pub name: Option<String>,

    /// Show a single snapshot instead of streaming
    #[arg(long)]
    pub no_stream: bool,
}

#[derive(clap::Args, Debug)]
pub struct KillArgs {
    /// Container name or ID
    pub name: String,

    /// Signal to send (default: SIGKILL)
    #[arg(short, long, default_value = "KILL")]
    pub signal: String,
}

#[derive(clap::Args, Debug)]
pub struct ImageSubArgs {
    #[command(subcommand)]
    pub command: ImageCommands,
}

#[derive(Subcommand, Debug)]
pub enum ImageCommands {
    /// Remove unused images
    Prune,
}

#[derive(clap::Args, Debug)]
pub struct SystemSubArgs {
    #[command(subcommand)]
    pub command: SystemCommands,
}

#[derive(Subcommand, Debug)]
pub enum SystemCommands {
    /// Remove stopped containers and unused images
    Prune,
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

    /// Run container in the background (detached mode)
    #[arg(short = 'd', long)]
    pub detach: bool,

    /// Restart policy: no (default), always, on-failure[:max-retries]
    #[arg(long, default_value = "no")]
    pub restart: String,

    /// Run in rootless mode (user namespace, no root required)
    #[arg(long)]
    pub rootless: bool,

    /// Automatically remove the container when it exits
    #[arg(long)]
    pub rm: bool,

    /// Set environment variables (KEY=VALUE)
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,

    /// Read environment variables from a file
    #[arg(long)]
    pub env_file: Option<String>,

    /// Give extended privileges to this container
    #[arg(long)]
    pub privileged: bool,

    /// Mount the container's root filesystem as read-only
    #[arg(long)]
    pub read_only: bool,

    /// Override the image entrypoint
    #[arg(long)]
    pub entrypoint: Option<String>,
}

/// Arguments for the `cp` subcommand.
#[derive(clap::Args, Debug)]
pub struct CpArgs {
    /// Source (container:path or host path)
    pub src: String,
    /// Destination (container:path or host path)
    pub dst: String,
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

/// Arguments for the `logs` subcommand.
#[derive(clap::Args, Debug)]
pub struct LogsArgs {
    /// Container name or ID
    pub name: String,

    /// Follow log output (stream new lines)
    #[arg(short, long)]
    pub follow: bool,

    /// Number of lines to show from the end
    #[arg(short = 'n', long, default_value = "100")]
    pub tail: usize,
}

/// Arguments for the `exec` subcommand.
#[derive(clap::Args, Debug)]
pub struct ExecArgs {
    /// Container name or ID
    pub name: String,

    /// Command and arguments to execute
    #[arg(trailing_var_arg = true, required = true)]
    pub command: Vec<String>,
}

/// Arguments for the `build` subcommand.
#[derive(clap::Args, Debug)]
pub struct BuildArgs {
    /// Path to directory containing Corten.toml, or path to the .toml file
    #[arg(default_value = ".")]
    pub path: String,

    /// Show the build plan without actually building
    #[arg(long)]
    pub dry_run: bool,
}
