//! Corten.toml build file parser and image builder.
//!
//! Builds container images from scratch using Corten.toml definitions.
//! Downloads base OS rootfs from official distro mirrors (no Docker Hub),
//! installs packages, copies files, and stores the result as a local image.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Run a command inside a chroot using syscalls (not the chroot binary).
///
/// The `chroot` binary doesn't inherit Linux capabilities from our process,
/// so we fork, call `nix::unistd::chroot()` (which uses our CAP_SYS_CHROOT),
/// then exec the command.
fn run_in_chroot(rootfs: &Path, cmd: &str, args: &[&str]) -> Result<std::process::ExitStatus> {
    run_in_chroot_with_env(rootfs, cmd, args, &[])
}

fn run_in_chroot_with_env(
    rootfs: &Path,
    cmd: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<std::process::ExitStatus> {
    use std::ffi::CString;

    // Build argv for execvp
    let shell_cmd = if args.is_empty() {
        cmd.to_string()
    } else {
        let escaped_args: Vec<String> = args.iter().map(|a| {
            if a.contains(' ') || a.contains('\'') || a.contains('"') || a.contains('$') || a.contains('\\') || a.contains('*') {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.to_string()
            }
        }).collect();
        format!("{cmd} {}", escaped_args.join(" "))
    };

    // Build envp
    let mut env_vec: Vec<String> = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        "HOME=/root".to_string(),
    ];
    for (k, v) in env {
        env_vec.push(format!("{k}={v}"));
    }

    let c_sh = CString::new("/bin/sh").unwrap();
    let c_flag = CString::new("-c").unwrap();
    let c_cmd = CString::new(shell_cmd.as_str()).unwrap();
    let c_argv = [c_sh.as_ptr(), c_flag.as_ptr(), c_cmd.as_ptr(), std::ptr::null()];
    let c_env: Vec<CString> = env_vec.iter().map(|e| CString::new(e.as_str()).unwrap()).collect();
    let c_envp: Vec<*const libc::c_char> = c_env.iter().map(|e| e.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();

    let rootfs_c = CString::new(rootfs.to_string_lossy().as_bytes())
        .context("invalid rootfs path")?;
    let root_c = CString::new("/").unwrap();

    // Fork, chroot (with our capabilities), exec
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(anyhow!("fork failed: {}", std::io::Error::last_os_error()));
    }
    if pid == 0 {
        // Child — become root (we have CAP_SETUID/CAP_SETGID), chroot, exec
        unsafe {
            // Become root so the chroot'd process can write to root-owned files
            libc::setgid(0);
            libc::setuid(0);

            if libc::chroot(rootfs_c.as_ptr()) != 0 {
                libc::_exit(127);
            }
            if libc::chdir(root_c.as_ptr()) != 0 {
                libc::_exit(127);
            }
            libc::execve(c_sh.as_ptr(), c_argv.as_ptr(), c_envp.as_ptr());
            libc::_exit(127); // exec failed
        }
    }

    // Parent — wait for child
    let mut status: libc::c_int = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };

    if libc::WIFEXITED(status) {
        let code = libc::WEXITSTATUS(status);
        // ExitStatus can't be constructed directly; use from_raw on the wait status
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(status))
    } else {
        Err(anyhow!("chroot command terminated abnormally"))
    }
}

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
    /// Log sources for `corten mlogs`
    pub logs: Option<LogsSection>,
}

/// Log sources definition for multi-log tailing.
#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct LogsSection {
    /// Specific log files to watch
    #[serde(default)]
    pub files: Vec<String>,
    /// Directories to watch (all files inside)
    #[serde(default)]
    pub dirs: Vec<String>,
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
/// Parse a build config from a file.
///
/// Supports three formats (auto-detected by extension):
/// - `.toml` — TOML (default, recommended)
/// - `.json` — JSON
/// - `.jsonc` — JSON with Comments (VS Code style)
pub fn parse_build_config(path: &Path) -> Result<BuildConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");

    let config: BuildConfig = match ext {
        "json" => serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON: {}", path.display()))?,
        "jsonc" => {
            let stripped = crate::strip_jsonc_comments(&content);
            serde_json::from_str(&stripped)
                .with_context(|| format!("failed to parse JSONC: {}", path.display()))?
        }
        _ => toml::from_str(&content)
            .with_context(|| format!("failed to parse TOML: {}", path.display()))?,
    };
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
}

// =============================================================================
// Image building pipeline
// =============================================================================

/// Build an image from a Corten.toml configuration.
///
/// The pipeline:
/// 1. Download or bootstrap the base OS rootfs
/// 2. Install packages via chroot
/// 3. Copy files
/// 4. Run setup commands
/// 5. Write image config
/// 6. Store in /var/lib/corten/images/<name>/<tag>/
pub async fn build_image(config: &BuildConfig, toml_dir: &Path) -> Result<std::path::PathBuf> {
    let image_name = config
        .image
        .as_ref()
        .map(|i| i.name.as_str())
        .unwrap_or(&config.base.system);
    let image_tag = config
        .image
        .as_ref()
        .map(|i| i.tag.as_str())
        .unwrap_or(&config.base.version);

    println!("Building {image_name}:{image_tag}...");

    // Determine where to store the image
    let image_dir = crate::config::images_dir().join(image_name).join(image_tag);
    let rootfs = image_dir.join("rootfs");

    if rootfs.exists() {
        std::fs::remove_dir_all(&rootfs).context("failed to clean existing rootfs")?;
    }
    std::fs::create_dir_all(&rootfs).context("failed to create rootfs directory")?;

    // Step 1: Bootstrap base OS
    println!("  [1/6] Bootstrapping {} {}...", config.base.system, config.base.version);
    bootstrap_rootfs(&config.base.system, &config.base.version, &rootfs).await?;

    // Step 2: Install packages
    if let Some(packages) = &config.packages {
        if !packages.install.is_empty() {
            println!(
                "  [2/6] Installing {} packages...",
                packages.install.len()
            );
            install_packages(&config.base.system, &rootfs, &packages.install)?;
        }
    } else {
        println!("  [2/6] No packages to install.");
    }

    // Step 3: Copy files
    if let Some(files) = &config.files {
        println!("  [3/6] Copying {} files...", files.copy.len());
        copy_files(toml_dir, &rootfs, &files.copy)?;
    } else {
        println!("  [3/6] No files to copy.");
    }

    // Step 4: Run setup commands
    if let Some(setup) = &config.setup {
        println!("  [4/6] Running {} setup commands...", setup.run.len());
        run_setup_commands(&rootfs, &setup.run)?;
    } else {
        println!("  [4/6] No setup commands.");
    }

    // Step 5: Clean package cache
    println!("  [5/6] Cleaning package cache...");
    clean_package_cache(&config.base.system, &rootfs);

    // Step 6: Write image config
    println!("  [6/6] Writing image config...");
    let img_config = crate::image::ImageConfig {
        env: config
            .env
            .as_ref()
            .map(|e| e.iter().map(|(k, v)| format!("{k}={v}")).collect())
            .unwrap_or_default(),
        cmd: config
            .container
            .as_ref()
            .and_then(|c| c.command.clone())
            .unwrap_or_default(),
        entrypoint: Vec::new(),
        working_dir: config
            .container
            .as_ref()
            .and_then(|c| c.workdir.clone())
            .unwrap_or_default(),
        user: config
            .container
            .as_ref()
            .and_then(|c| c.user.clone())
            .unwrap_or_default(),
        log_files: config.logs.as_ref().map(|l| l.files.clone()).unwrap_or_default(),
        log_dirs: config.logs.as_ref().map(|l| l.dirs.clone()).unwrap_or_default(),
    };
    let config_json = serde_json::to_string_pretty(&img_config)?;
    std::fs::write(image_dir.join("config.json"), config_json)
        .context("failed to write image config")?;

    println!();
    println!("Successfully built {image_name}:{image_tag}");
    println!("  Rootfs: {}", rootfs.display());

    Ok(rootfs)
}

/// Bootstrap a base OS rootfs.
async fn bootstrap_rootfs(system: &str, version: &str, rootfs: &Path) -> Result<()> {
    match system.to_lowercase().as_str() {
        "alpine" => bootstrap_alpine(version, rootfs).await,
        "ubuntu" | "debian" => bootstrap_debootstrap(system, version, rootfs),
        "fedora" | "rhel" | "centos" => bootstrap_dnf(version, rootfs),
        _ => Err(anyhow!("unsupported base system: '{system}'")),
    }
}

/// Bootstrap Alpine by downloading the minirootfs tarball.
async fn bootstrap_alpine(version: &str, rootfs: &Path) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => "x86_64",
    };

    // Try multiple URL patterns (Alpine versioning varies)
    let urls = vec![
        format!("https://dl-cdn.alpinelinux.org/alpine/v{version}/releases/{arch}/alpine-minirootfs-{version}.0-{arch}.tar.gz"),
        format!("https://dl-cdn.alpinelinux.org/alpine/v{version}/releases/{arch}/alpine-minirootfs-{version}.1-{arch}.tar.gz"),
        format!("https://dl-cdn.alpinelinux.org/alpine/v{version}/releases/{arch}/alpine-minirootfs-{version}.2-{arch}.tar.gz"),
        format!("https://dl-cdn.alpinelinux.org/alpine/v{version}/releases/{arch}/alpine-minirootfs-{version}.3-{arch}.tar.gz"),
    ];

    let client = reqwest::Client::new();
    let mut last_err = anyhow!("no URLs to try");

    for url in &urls {
        println!("    Trying {url}");
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await.context("failed to download rootfs")?;
                println!("    Downloaded {:.1} MB", bytes.len() as f64 / 1_048_576.0);

                // Extract tar.gz
                let decoder = flate2::read::GzDecoder::new(&bytes[..]);
                let mut archive = tar::Archive::new(decoder);
                archive.set_preserve_permissions(true);
                archive.set_preserve_ownerships(true);
                archive.unpack(rootfs).context("failed to extract rootfs")?;

                // Set up resolv.conf for package installation
                let etc = rootfs.join("etc");
                std::fs::create_dir_all(&etc).ok();
                if Path::new("/etc/resolv.conf").exists() {
                    std::fs::copy("/etc/resolv.conf", etc.join("resolv.conf")).ok();
                }

                return Ok(());
            }
            Ok(resp) => {
                last_err = anyhow!("HTTP {}: {url}", resp.status());
            }
            Err(e) => {
                last_err = anyhow!("download failed: {e}");
            }
        }
    }

    Err(last_err.context(format!(
        "failed to download Alpine {version} minirootfs. Check version number."
    )))
}

/// Bootstrap Ubuntu/Debian using debootstrap.
fn bootstrap_debootstrap(system: &str, version: &str, rootfs: &Path) -> Result<()> {
    // Map version to codename for debootstrap
    let suite = match (system, version) {
        ("ubuntu", "24.04") => "noble",
        ("ubuntu", "22.04") => "jammy",
        ("ubuntu", "20.04") => "focal",
        ("debian", "12") | ("debian", "bookworm") => "bookworm",
        ("debian", "11") | ("debian", "bullseye") => "bullseye",
        _ => version, // try using version as suite name directly
    };

    let mirror = match system {
        "ubuntu" => "http://archive.ubuntu.com/ubuntu",
        _ => "http://deb.debian.org/debian",
    };

    println!("    Running debootstrap --variant=minbase {suite} ...");

    let status = std::process::Command::new("debootstrap")
        .args([
            "--variant=minbase",
            suite,
            &rootfs.to_string_lossy(),
            mirror,
        ])
        .status()
        .context("failed to run debootstrap. Install with: sudo apt install debootstrap")?;

    if !status.success() {
        return Err(anyhow!("debootstrap failed with exit code {}", status));
    }

    Ok(())
}

/// Bootstrap Fedora/RHEL using dnf.
fn bootstrap_dnf(version: &str, rootfs: &Path) -> Result<()> {
    let status = std::process::Command::new("dnf")
        .args([
            "--installroot",
            &rootfs.to_string_lossy(),
            "--releasever",
            version,
            "install",
            "-y",
            "bash",
            "coreutils",
        ])
        .status()
        .context("failed to run dnf. Install with: sudo dnf install dnf")?;

    if !status.success() {
        return Err(anyhow!("dnf bootstrap failed"));
    }

    Ok(())
}

/// Install packages into the rootfs using the appropriate package manager.
fn install_packages(system: &str, rootfs: &Path, packages: &[String]) -> Result<()> {
    let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();

    match detect_package_manager(system) {
        "apk" => {
            let mut args = vec!["add", "--no-cache"];
            args.extend(&pkg_refs);
            let status = run_in_chroot(rootfs, "apk", &args)?;
            if !status.success() {
                return Err(anyhow!("apk add failed"));
            }
        }
        "apt" => {
            let status = run_in_chroot_with_env(
                rootfs, "apt-get", &["update", "-qq"],
                &[("DEBIAN_FRONTEND", "noninteractive")],
            )?;
            if !status.success() {
                log::warn!("apt-get update failed (may still work)");
            }

            let mut args = vec!["install", "-y", "-qq"];
            args.extend(&pkg_refs);
            let status = run_in_chroot_with_env(
                rootfs, "apt-get", &args,
                &[("DEBIAN_FRONTEND", "noninteractive")],
            )?;
            if !status.success() {
                return Err(anyhow!("apt-get install failed"));
            }
        }
        "dnf" => {
            let mut args = vec!["install", "-y"];
            args.extend(&pkg_refs);
            let status = run_in_chroot(rootfs, "dnf", &args)?;
            if !status.success() {
                return Err(anyhow!("dnf install failed"));
            }
        }
        other => {
            return Err(anyhow!(
                "package installation not supported for package manager: {other}"
            ));
        }
    }

    Ok(())
}

/// Copy files from the build context into the rootfs.
fn copy_files(toml_dir: &Path, rootfs: &Path, files: &[FileCopy]) -> Result<()> {
    for f in files {
        let src = toml_dir.join(&f.src);
        let dest = rootfs.join(f.dest.trim_start_matches('/'));

        if !src.exists() {
            log::warn!("source file not found: {} (skipping)", src.display());
            continue;
        }

        // Create parent directory
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if src.is_dir() {
            // Recursive copy
            copy_dir_all(&src, &dest)?;
        } else {
            std::fs::copy(&src, &dest)
                .with_context(|| format!("failed to copy {} -> {}", src.display(), dest.display()))?;
        }

        // Set owner if specified (best-effort)
        if let Some(owner) = &f.owner {
            run_in_chroot(rootfs, "chown", &["-R", owner, &f.dest]).ok();
        }

        println!("    {} -> {}", f.src, f.dest);
    }

    Ok(())
}

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

/// Run setup commands in chroot.
fn run_setup_commands(rootfs: &Path, commands: &[String]) -> Result<()> {
    for cmd_str in commands {
        println!("    $ {cmd_str}");
        let status = run_in_chroot(rootfs, "sh", &["-c", cmd_str])?;

        if !status.success() {
            return Err(anyhow!("setup command failed: {cmd_str}"));
        }
    }

    Ok(())
}

/// Clean package manager caches to reduce image size.
fn clean_package_cache(system: &str, rootfs: &Path) {
    match detect_package_manager(system) {
        "apk" => {
            run_in_chroot(rootfs, "rm", &["-rf", "/var/cache/apk/*"]).ok();
        }
        "apt" => {
            run_in_chroot(rootfs, "apt-get", &["clean"]).ok();
            // Remove apt lists
            let lists = rootfs.join("var/lib/apt/lists");
            if lists.exists() {
                std::fs::remove_dir_all(&lists).ok();
                std::fs::create_dir_all(&lists).ok();
            }
        }
        "dnf" => {
            run_in_chroot(rootfs, "dnf", &["clean", "all"]).ok();
        }
        _ => {}
    }
}
