//! Image management — pulling, storing, and listing container images.
//!
//! Corten fetches base OS rootfs tarballs directly from official distro
//! mirrors. No Docker Hub, no OCI layers, no daemon.
//!
//! ## Supported distros
//!
//! | Distro  | Source                              | Architectures       |
//! |---------|-------------------------------------|----------------------|
//! | Alpine  | dl-cdn.alpinelinux.org              | x86_64, aarch64      |
//! | Ubuntu  | cloud-images.ubuntu.com             | amd64, arm64         |
//! | Debian  | cdimage.debian.org (debootstrap)     | amd64, arm64         |
//! | Fedora  | kojipkgs.fedoraproject.org           | x86_64, aarch64      |
//! | Arch    | geo.mirror.pkgbuild.com             | x86_64               |
//! | Void    | repo-default.voidlinux.org           | x86_64, aarch64      |
//!
//! ## Local storage layout
//!
//! ```text
//! /var/lib/corten/images/
//!   alpine/
//!     3.20/
//!       rootfs/          # extracted filesystem
//!       config.json      # image config (ENV, CMD, USER, etc.)
//! ```

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::images_dir;

/// Resolved image configuration stored alongside the rootfs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageConfig {
    /// Environment variables (KEY=VALUE format)
    #[serde(default)]
    pub env: Vec<String>,
    /// Default command
    #[serde(default)]
    pub cmd: Vec<String>,
    /// Entrypoint
    #[serde(default)]
    pub entrypoint: Vec<String>,
    /// Working directory
    #[serde(default)]
    pub working_dir: String,
    /// User (user or user:group)
    #[serde(default)]
    pub user: String,
    /// Log sources for corten mlogs
    #[serde(default)]
    pub log_files: Vec<String>,
    /// Log directories for corten mlogs
    #[serde(default)]
    pub log_dirs: Vec<String>,
}

/// Host architecture in distro-native naming.
fn native_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => "x86_64",
    }
}

/// Host architecture in Debian/Ubuntu naming.
fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

/// Pull an image by downloading the base OS rootfs from official mirrors.
///
/// Supported image references:
/// - `alpine`, `alpine:3.20` — Alpine Linux minirootfs
/// - `ubuntu:noble`, `ubuntu:22.04` — Ubuntu cloud rootfs
/// - `debian:bookworm` — Debian via debootstrap
/// - `fedora:40` — Fedora container base
/// - `archlinux` — Arch Linux bootstrap
/// - `void` — Void Linux rootfs
pub async fn pull_image(name: &str, tag: &str) -> Result<PathBuf> {
    let image_dir = images_dir().join(name).join(tag);
    let rootfs = image_dir.join("rootfs");

    if rootfs.exists() {
        fs::remove_dir_all(&rootfs).context("failed to clean existing rootfs")?;
    }
    fs::create_dir_all(&rootfs).context("failed to create rootfs directory")?;

    println!("Pulling {name}:{tag} from official mirrors...");

    match name.to_lowercase().as_str() {
        "alpine" => pull_alpine(tag, &rootfs).await?,
        "ubuntu" => pull_ubuntu(tag, &rootfs).await?,
        "debian" => pull_debian(tag, &rootfs)?,
        "fedora" => pull_fedora(tag, &rootfs).await?,
        "archlinux" | "arch" => pull_arch(&rootfs).await?,
        "void" | "voidlinux" => pull_void(&rootfs).await?,
        _ => {
            return Err(anyhow!(
                "unsupported image '{name}'. Supported: alpine, ubuntu, debian, fedora, archlinux, void\n\
                 Or build your own with: corten build <path-to-Corten.toml>"
            ));
        }
    }

    // Copy host DNS for package operations
    let etc = rootfs.join("etc");
    fs::create_dir_all(&etc).ok();
    if Path::new("/etc/resolv.conf").exists() {
        fs::copy("/etc/resolv.conf", etc.join("resolv.conf")).ok();
    }

    // Write default image config
    let config = default_image_config(name);
    fs::write(
        image_dir.join("config.json"),
        serde_json::to_string_pretty(&config)?,
    )?;

    println!("Successfully pulled {name}:{tag}");
    Ok(rootfs)
}

/// Default image config for known distros.
fn default_image_config(_name: &str) -> ImageConfig {
    ImageConfig {
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        ],
        cmd: vec!["/bin/sh".to_string()],
        entrypoint: Vec::new(),
        working_dir: String::new(),
        user: String::new(),
        log_files: Vec::new(),
        log_dirs: Vec::new(),
    }
}

/// Pull Alpine Linux minirootfs.
async fn pull_alpine(tag: &str, rootfs: &Path) -> Result<()> {
    let version = if tag == "latest" { "3.20" } else { tag };
    let arch = native_arch();

    // Try multiple patch versions
    let urls: Vec<String> = (0..=5)
        .map(|patch| {
            format!(
                "https://dl-cdn.alpinelinux.org/alpine/v{version}/releases/{arch}/alpine-minirootfs-{version}.{patch}-{arch}.tar.gz"
            )
        })
        .collect();

    download_and_extract_tarball(&urls, rootfs, "Alpine").await
}

/// Pull Ubuntu cloud rootfs.
async fn pull_ubuntu(tag: &str, rootfs: &Path) -> Result<()> {
    let (version, codename) = match tag {
        "latest" | "24.04" | "noble" => ("24.04", "noble"),
        "22.04" | "jammy" => ("22.04", "jammy"),
        "20.04" | "focal" => ("20.04", "focal"),
        other => return Err(anyhow!("unsupported Ubuntu version: {other}. Use: 24.04, 22.04, 20.04")),
    };

    let arch = deb_arch();
    let urls = vec![
        format!("https://cloud-images.ubuntu.com/{codename}/current/{codename}-server-cloudimg-{arch}-root.tar.xz"),
        format!("https://cloud-images.ubuntu.com/minimal/releases/{codename}/release/ubuntu-{version}-minimal-cloudimg-{arch}-root.tar.xz"),
    ];

    download_and_extract_tarball(&urls, rootfs, "Ubuntu").await
}

/// Pull Debian via debootstrap (local tool required).
fn pull_debian(tag: &str, rootfs: &Path) -> Result<()> {
    let suite = match tag {
        "latest" | "12" | "bookworm" => "bookworm",
        "11" | "bullseye" => "bullseye",
        other => other,
    };

    println!("  Using debootstrap for Debian {suite}...");

    let status = std::process::Command::new("debootstrap")
        .args(["--variant=minbase", suite, &rootfs.to_string_lossy()])
        .status()
        .context(
            "debootstrap not found. Install with: sudo apt install debootstrap\n\
             Or use Alpine instead: corten pull alpine",
        )?;

    if !status.success() {
        return Err(anyhow!("debootstrap failed"));
    }

    Ok(())
}

/// Pull Fedora container base image.
async fn pull_fedora(tag: &str, rootfs: &Path) -> Result<()> {
    let version = if tag == "latest" { "41" } else { tag };
    let arch = native_arch();

    let urls = vec![
        format!("https://kojipkgs.fedoraproject.org/packages/Fedora-Container-Base/{version}/20240912.n.0/images/Fedora-Container-Base-{version}-20240912.n.0.{arch}.tar.xz"),
    ];

    // Fedora container base is a tarball with a nested rootfs
    // For simplicity, try direct extraction
    download_and_extract_tarball(&urls, rootfs, "Fedora").await
}

/// Pull Arch Linux bootstrap.
async fn pull_arch(rootfs: &Path) -> Result<()> {
    let urls = vec![
        "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.zst".to_string(),
        "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.gz".to_string(),
    ];

    download_and_extract_tarball(&urls, rootfs, "Arch Linux").await
}

/// Pull Void Linux rootfs.
async fn pull_void(rootfs: &Path) -> Result<()> {
    let arch = native_arch();
    let musl_suffix = if arch == "x86_64" { "x86_64-musl" } else { "aarch64-musl" };

    let urls = vec![
        format!("https://repo-default.voidlinux.org/live/current/void-{musl_suffix}-ROOTFS-20240314.tar.xz"),
        format!("https://repo-default.voidlinux.org/live/current/void-{musl_suffix}-ROOTFS-20230628.tar.xz"),
    ];

    download_and_extract_tarball(&urls, rootfs, "Void Linux").await
}

/// Download a tarball from the first working URL and extract it.
async fn download_and_extract_tarball(urls: &[String], rootfs: &Path, distro: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let mut last_err = anyhow!("no URLs to try");

    for url in urls {
        println!("  Trying {url}");
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await.context("failed to download")?;
                let size_mb = bytes.len() as f64 / 1_048_576.0;
                println!("  Downloaded {size_mb:.1} MB");

                // Detect format and extract
                if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
                    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
                    let mut archive = tar::Archive::new(decoder);
                    archive.set_preserve_permissions(true);
                    archive.set_preserve_ownerships(true);
                    archive.unpack(rootfs).context("failed to extract tar.gz")?;
                } else if url.ends_with(".tar.xz") {
                    extract_tar_xz(&bytes, rootfs)?;
                } else if url.ends_with(".tar.zst") {
                    extract_tar_zst(&bytes, rootfs)?;
                } else {
                    // Try as tar.gz by default
                    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
                    let mut archive = tar::Archive::new(decoder);
                    archive.set_preserve_permissions(true);
                    archive.unpack(rootfs).context("failed to extract tarball")?;
                }

                return Ok(());
            }
            Ok(resp) => {
                last_err = anyhow!("HTTP {}", resp.status());
            }
            Err(e) => {
                last_err = anyhow!("request failed: {e}");
            }
        }
    }

    Err(last_err.context(format!("failed to download {distro} rootfs")))
}

/// Extract a .tar.xz archive using the system `tar` command.
fn extract_tar_xz(data: &[u8], rootfs: &Path) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("tar")
        .args(["xJf", "-", "-C", &rootfs.to_string_lossy()])
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to run tar (is xz-utils installed?)")?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(data)
        .context("failed to pipe data to tar")?;

    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow!("tar xJ failed"));
    }
    Ok(())
}

/// Extract a .tar.zst archive using the system `tar` command.
fn extract_tar_zst(data: &[u8], rootfs: &Path) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("tar")
        .args(["--zstd", "-xf", "-", "-C", &rootfs.to_string_lossy()])
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to run tar with zstd (is zstd installed?)")?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(data)
        .context("failed to pipe data to tar")?;

    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow!("tar --zstd failed"));
    }
    Ok(())
}

/// Load the saved image config for a locally stored image.
pub fn load_image_config(name: &str, tag: &str) -> ImageConfig {
    let config_path = images_dir().join(name).join(tag).join("config.json");
    match fs::read_to_string(&config_path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => ImageConfig::default(),
    }
}

/// Check if an image exists in local storage.
pub fn image_exists(name: &str, tag: &str) -> bool {
    images_dir().join(name).join(tag).join("rootfs").exists()
}

/// Get the rootfs path for a locally stored image.
pub fn image_rootfs(name: &str, tag: &str) -> PathBuf {
    images_dir().join(name).join(tag).join("rootfs")
}

/// List all locally available images.
pub fn list_images() -> Result<Vec<(String, String)>> {
    let dir = images_dir();
    let mut images = Vec::new();

    if !dir.exists() {
        return Ok(images);
    }

    for name_entry in fs::read_dir(&dir).context("failed to read images directory")? {
        let name_entry = name_entry?;
        if !name_entry.file_type()?.is_dir() {
            continue;
        }
        let name = name_entry.file_name().to_string_lossy().to_string();

        for tag_entry in fs::read_dir(name_entry.path())? {
            let tag_entry = tag_entry?;
            if !tag_entry.file_type()?.is_dir() {
                continue;
            }
            let tag = tag_entry.file_name().to_string_lossy().to_string();

            if tag_entry.path().join("rootfs").exists() {
                images.push((name.clone(), tag));
            }
        }
    }

    Ok(images)
}
