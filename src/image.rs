//! OCI image management — pulling, storing, and listing container images.
//!
//! Corten pulls images from Docker Hub (`registry-1.docker.io`) using the
//! [OCI Distribution Specification](https://github.com/opencontainers/distribution-spec).
//!
//! ## Pull flow
//!
//! 1. **Authenticate** — obtain a bearer token from `auth.docker.io`
//! 2. **Fetch manifest** — get the image manifest (or manifest list for multi-arch)
//! 3. **Download layers** — each layer is a gzipped tar archive
//! 4. **Extract layers** — unpack in order to build the root filesystem
//!
//! ## Local storage layout
//!
//! ```text
//! /var/lib/corten/images/
//!   alpine/
//!     latest/
//!       rootfs/          # extracted filesystem ready for pivot_root
//!   ubuntu/
//!     22.04/
//!       rootfs/
//! ```

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::images_dir;

/// Docker Hub registry base URL.
const REGISTRY: &str = "https://registry-1.docker.io";

/// Docker Hub authentication service.
const AUTH_SERVICE: &str = "https://auth.docker.io/token";

// --- Registry API response types ---

#[derive(Deserialize)]
struct AuthToken {
    token: String,
}

/// OCI/Docker image manifest.
#[derive(Deserialize)]
struct Manifest {
    #[allow(dead_code)]
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[allow(dead_code)]
    config: Descriptor,
    layers: Vec<Descriptor>,
}

/// A content-addressable blob descriptor.
#[derive(Deserialize)]
struct Descriptor {
    #[allow(dead_code)]
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
}

/// Manifest list (fat manifest) for multi-architecture images.
#[derive(Deserialize)]
struct ManifestList {
    #[allow(dead_code)]
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    manifests: Vec<PlatformManifest>,
}

/// A single platform entry in a manifest list.
#[derive(Deserialize)]
struct PlatformManifest {
    #[allow(dead_code)]
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    #[allow(dead_code)]
    size: u64,
    platform: Platform,
}

/// Platform specification (OS + architecture).
#[derive(Deserialize)]
struct Platform {
    architecture: String,
    os: String,
}

// --- OCI image configuration types ---

/// OCI image configuration blob (the JSON config referenced by the manifest).
#[derive(Deserialize)]
struct OciImageConfig {
    config: Option<OciContainerConfig>,
}

/// Container-specific configuration within the OCI image config.
#[derive(Deserialize)]
struct OciContainerConfig {
    #[serde(rename = "Env")]
    env: Option<Vec<String>>,
    #[serde(rename = "Cmd")]
    cmd: Option<Vec<String>>,
    #[serde(rename = "Entrypoint")]
    entrypoint: Option<Vec<String>>,
    #[serde(rename = "WorkingDir")]
    working_dir: Option<String>,
    #[serde(rename = "User")]
    user: Option<String>,
}

/// Resolved image configuration stored alongside the rootfs.
/// This is Corten's simplified representation of the OCI config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageConfig {
    /// Environment variables (KEY=VALUE format)
    #[serde(default)]
    pub env: Vec<String>,
    /// Default command (CMD in Dockerfile)
    #[serde(default)]
    pub cmd: Vec<String>,
    /// Entrypoint (ENTRYPOINT in Dockerfile)
    #[serde(default)]
    pub entrypoint: Vec<String>,
    /// Working directory
    #[serde(default)]
    pub working_dir: String,
    /// User (user or user:group)
    #[serde(default)]
    pub user: String,
}

/// Map Rust's `std::env::consts::ARCH` to Docker's architecture names.
fn docker_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "riscv64" => "riscv64",
        _ => "amd64",
    }
}

/// Pull an image from Docker Hub and extract it locally.
///
/// Downloads all layers and extracts them in order to build a complete
/// root filesystem at `/var/lib/corten/images/<name>/<tag>/rootfs/`.
///
/// Supports multi-architecture images by automatically selecting the
/// platform matching the host architecture.
///
/// # Arguments
///
/// * `name` — Image name (e.g., `"alpine"`, `"ubuntu"`)
/// * `tag` — Image tag (e.g., `"latest"`, `"22.04"`, `"bookworm"`)
///
/// # Returns
///
/// Path to the extracted root filesystem.
pub async fn pull_image(name: &str, tag: &str) -> Result<PathBuf> {
    // Docker Hub requires the "library/" prefix for official images
    let repo = if name.contains('/') {
        name.to_string()
    } else {
        format!("library/{name}")
    };

    println!("Pulling {name}:{tag}...");

    let client = reqwest::Client::new();

    // Step 1: Authenticate with Docker Hub
    let token = get_auth_token(&client, &repo)
        .await
        .context("authentication failed")?;

    // Step 2: Fetch the image manifest
    let manifest = get_manifest(&client, &repo, tag, &token)
        .await
        .context("failed to fetch manifest")?;

    // Step 3: Prepare local storage
    let image_dir = images_dir().join(name).join(tag);
    let rootfs_dir = image_dir.join("rootfs");

    if rootfs_dir.exists() {
        fs::remove_dir_all(&rootfs_dir).context("failed to clean existing rootfs")?;
    }
    fs::create_dir_all(&rootfs_dir).context("failed to create rootfs directory")?;

    // Step 4: Download and extract each layer in order
    let total = manifest.layers.len();
    for (i, layer) in manifest.layers.iter().enumerate() {
        println!(
            "  Layer {}/{} ({:.1} MB) {}",
            i + 1,
            total,
            layer.size as f64 / 1_048_576.0,
            &layer.digest[..19], // Show short digest
        );
        download_and_extract_layer(&client, &repo, &layer.digest, &token, &rootfs_dir)
            .await
            .with_context(|| format!("failed to extract layer {}", layer.digest))?;
    }

    // Step 5: Download and save the image config
    let image_config = download_image_config(&client, &repo, &manifest.config.digest, &token)
        .await
        .context("failed to download image config")?;
    let config_path = image_dir.join("config.json");
    fs::write(&config_path, serde_json::to_string_pretty(&image_config)?)
        .context("failed to save image config")?;
    log::info!("saved image config to {}", config_path.display());

    println!("Successfully pulled {name}:{tag}");
    Ok(rootfs_dir)
}

/// Obtain a bearer token for pulling from the given repository.
async fn get_auth_token(client: &reqwest::Client, repo: &str) -> Result<String> {
    let url = format!("{AUTH_SERVICE}?service=registry.docker.io&scope=repository:{repo}:pull");

    let resp: AuthToken = client
        .get(&url)
        .send()
        .await
        .context("auth request failed")?
        .json()
        .await
        .context("failed to parse auth response")?;

    Ok(resp.token)
}

/// Fetch the image manifest, handling both single-arch and multi-arch images.
///
/// When a manifest list (fat manifest) is returned, automatically selects
/// the manifest matching the host architecture.
async fn get_manifest(
    client: &reqwest::Client,
    repo: &str,
    tag: &str,
    token: &str,
) -> Result<Manifest> {
    let url = format!("{REGISTRY}/v2/{repo}/manifests/{tag}");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header(
            "Accept",
            [
                "application/vnd.oci.image.manifest.v1+json",
                "application/vnd.docker.distribution.manifest.v2+json",
                "application/vnd.docker.distribution.manifest.list.v2+json",
                "application/vnd.oci.image.index.v1+json",
            ]
            .join(", "),
        )
        .send()
        .await
        .context("manifest request failed")?;

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp.text().await.context("failed to read manifest body")?;

    // Multi-arch images return a manifest list — we need to pick our platform
    if content_type.contains("manifest.list") || content_type.contains("image.index") {
        let list: ManifestList =
            serde_json::from_str(&body).context("failed to parse manifest list")?;

        let arch = docker_arch();
        let platform_manifest = list
            .manifests
            .iter()
            .find(|m| m.platform.architecture == arch && m.platform.os == "linux")
            .ok_or_else(|| anyhow!("no {arch}/linux manifest found for this image"))?;

        log::info!(
            "selected {arch}/linux platform (digest: {})",
            &platform_manifest.digest[..19]
        );

        // Fetch the actual manifest by digest
        let url = format!("{REGISTRY}/v2/{repo}/manifests/{}", platform_manifest.digest);
        let manifest: Manifest = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header(
                "Accept",
                [
                    "application/vnd.oci.image.manifest.v1+json",
                    "application/vnd.docker.distribution.manifest.v2+json",
                ]
                .join(", "),
            )
            .send()
            .await
            .context("failed to fetch platform manifest")?
            .json()
            .await
            .context("failed to parse platform manifest")?;

        Ok(manifest)
    } else {
        // Single-arch image — parse directly
        let manifest: Manifest =
            serde_json::from_str(&body).context("failed to parse manifest")?;
        Ok(manifest)
    }
}

/// Download a single layer blob and extract it into the rootfs.
///
/// Layers are gzipped tar archives. They are extracted in order,
/// with later layers overwriting files from earlier layers (union
/// filesystem semantics).
async fn download_and_extract_layer(
    client: &reqwest::Client,
    repo: &str,
    digest: &str,
    token: &str,
    rootfs: &Path,
) -> Result<()> {
    let url = format!("{REGISTRY}/v2/{repo}/blobs/{digest}");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .context("layer download failed")?;

    let bytes = resp.bytes().await.context("failed to read layer data")?;

    // Decompress (gzip) and unpack (tar) with OCI whiteout handling
    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(true);
    archive.set_overwrite(true);

    for entry in archive.entries().context("failed to read tar entries")? {
        let mut entry = entry.context("failed to read tar entry")?;
        let path = entry.path().context("failed to read entry path")?.into_owned();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name == ".wh..wh..opq" {
            // Opaque whiteout: clear the parent directory contents
            let parent = rootfs.join(
                path.parent().unwrap_or_else(|| std::path::Path::new("")),
            );
            if parent.exists() {
                for child in fs::read_dir(&parent)? {
                    let child = child?;
                    let child_path = child.path();
                    if child_path.is_dir() {
                        fs::remove_dir_all(&child_path).ok();
                    } else {
                        fs::remove_file(&child_path).ok();
                    }
                }
            }
            continue;
        }

        if let Some(target_name) = file_name.strip_prefix(".wh.") {
            // File whiteout: delete the target file/dir
            let target = rootfs.join(
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .join(target_name),
            );
            if target.is_dir() {
                fs::remove_dir_all(&target).ok();
            } else {
                fs::remove_file(&target).ok();
            }
            continue;
        }

        // Normal entry: extract to rootfs
        entry.unpack_in(rootfs).context("failed to unpack entry")?;
    }

    Ok(())
}

/// Download and parse the OCI image config blob.
async fn download_image_config(
    client: &reqwest::Client,
    repo: &str,
    digest: &str,
    token: &str,
) -> Result<ImageConfig> {
    let url = format!("{REGISTRY}/v2/{repo}/blobs/{digest}");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .context("config blob download failed")?;

    let body = resp.text().await.context("failed to read config blob")?;
    let oci_config: OciImageConfig =
        serde_json::from_str(&body).context("failed to parse OCI image config")?;

    let container_config = oci_config.config.unwrap_or(OciContainerConfig {
        env: None,
        cmd: None,
        entrypoint: None,
        working_dir: None,
        user: None,
    });

    Ok(ImageConfig {
        env: container_config.env.unwrap_or_default(),
        cmd: container_config.cmd.unwrap_or_default(),
        entrypoint: container_config.entrypoint.unwrap_or_default(),
        working_dir: container_config.working_dir.unwrap_or_default(),
        user: container_config.user.unwrap_or_default(),
    })
}

/// Load the saved image config for a locally stored image.
///
/// Returns `ImageConfig::default()` if no config file exists
/// (e.g., for images pulled before config support was added).
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
///
/// Scans the images directory and returns (name, tag) pairs for
/// every image that has an extracted rootfs.
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
