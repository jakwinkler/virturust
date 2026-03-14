//! Corten.toml build file parser.
//!
//! Defines the schema for declarative image building.
//! The actual build pipeline (bootstrapping, package installation,
//! SquashFS packing) will be implemented in a future phase.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Top-level Corten.toml structure.
#[derive(Debug, Deserialize)]
pub struct BuildConfig {
    /// Image metadata
    pub image: Option<ImageSection>,
    /// Base OS to build from
    pub base: BaseSection,
    /// Packages to install
    pub packages: Option<PackagesSection>,
    /// Files to copy into the image
    pub files: Option<FilesSection>,
    /// Environment variables
    pub env: Option<std::collections::HashMap<String, String>>,
    /// Setup commands (escape hatch for custom build steps)
    pub setup: Option<SetupSection>,
    /// Container runtime defaults
    pub container: Option<ContainerSection>,
}

/// Image metadata.
#[derive(Debug, Deserialize)]
pub struct ImageSection {
    /// Image name
    pub name: String,
    /// Image tag
    pub tag: String,
}

/// Base OS specification.
#[derive(Debug, Deserialize)]
pub struct BaseSection {
    /// OS name (e.g., "ubuntu", "alpine", "fedora")
    pub system: String,
    /// OS version (e.g., "22.04", "3.19")
    pub version: String,
}

/// Packages to install.
#[derive(Debug, Deserialize)]
pub struct PackagesSection {
    /// List of package names
    pub install: Vec<String>,
}

/// Files to copy into the image.
#[derive(Debug, Deserialize)]
pub struct FilesSection {
    /// List of file copy operations
    pub copy: Vec<FileCopy>,
}

/// A single file copy operation.
#[derive(Debug, Deserialize)]
pub struct FileCopy {
    /// Source path (relative to Corten.toml)
    pub src: String,
    /// Destination path inside the image
    pub dest: String,
    /// Owner (optional, e.g., "www-data")
    pub owner: Option<String>,
}

/// Setup commands to run during build.
#[derive(Debug, Deserialize)]
pub struct SetupSection {
    /// Commands to execute in order
    pub run: Vec<String>,
}

/// Container runtime defaults.
#[derive(Debug, Deserialize)]
pub struct ContainerSection {
    /// Default command
    pub command: Option<Vec<String>>,
    /// Default user
    pub user: Option<String>,
    /// Default working directory
    pub workdir: Option<String>,
    /// Ports to expose (informational)
    pub expose: Option<Vec<u16>>,
}

/// Detect the package manager for a given base OS.
pub fn detect_package_manager(system: &str) -> &'static str {
    match system.to_lowercase().as_str() {
        "ubuntu" | "debian" => "apt",
        "alpine" => "apk",
        "fedora" | "rhel" | "centos" | "rocky" | "alma" => "dnf",
        "arch" | "manjaro" => "pacman",
        "opensuse" | "suse" => "zypper",
        _ => "unknown",
    }
}

/// Parse a Corten.toml file.
pub fn parse_build_config(path: &Path) -> Result<BuildConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: BuildConfig = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}

/// Validate a parsed build config.
pub fn validate_build_config(config: &BuildConfig) -> Result<()> {
    // Base section is required and already enforced by serde
    let pkg_mgr = detect_package_manager(&config.base.system);
    if pkg_mgr == "unknown" {
        log::warn!(
            "unknown package manager for '{}' — setup commands may be needed",
            config.base.system
        );
    }

    // Validate file copy paths
    if let Some(files) = &config.files {
        for copy in &files.copy {
            if copy.dest.is_empty() {
                anyhow::bail!("file copy destination cannot be empty (src: {})", copy.src);
            }
        }
    }

    Ok(())
}

/// Print a summary of what a build would do (dry-run).
pub fn print_build_plan(config: &BuildConfig) {
    println!("Build plan:");
    println!();

    if let Some(image) = &config.image {
        println!("  Image:     {}:{}", image.name, image.tag);
    }

    println!("  Base:      {} {}", config.base.system, config.base.version);
    println!("  Pkg mgr:   {}", detect_package_manager(&config.base.system));

    if let Some(packages) = &config.packages {
        println!("  Packages:  {} to install", packages.install.len());
        for pkg in &packages.install {
            println!("    - {pkg}");
        }
    }

    if let Some(files) = &config.files {
        println!("  Files:     {} to copy", files.copy.len());
        for f in &files.copy {
            let owner = f.owner.as_deref().unwrap_or("-");
            println!("    {} -> {} (owner: {owner})", f.src, f.dest);
        }
    }

    if let Some(env) = &config.env {
        println!("  Env vars:  {}", env.len());
        for (k, v) in env {
            println!("    {k}={v}");
        }
    }

    if let Some(setup) = &config.setup {
        println!("  Setup:     {} commands", setup.run.len());
        for cmd in &setup.run {
            println!("    $ {cmd}");
        }
    }

    if let Some(container) = &config.container {
        if let Some(cmd) = &container.command {
            println!("  Command:   {}", cmd.join(" "));
        }
        if let Some(user) = &container.user {
            println!("  User:      {user}");
        }
        if let Some(workdir) = &container.workdir {
            println!("  Workdir:   {workdir}");
        }
    }

    println!();
    println!("  NOTE: Actual image building is not yet implemented.");
    println!("        This is a preview of what `corten build` will do.");
}
