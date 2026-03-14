//! OCI image management — pulling, storing, and listing container images.
//!
//! VirtuRust pulls images from Docker Hub (`registry-1.docker.io`) using the
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
//! /var/lib/virturust/images/
//!   alpine/
//!     latest/
//!       rootfs/          # extracted filesystem ready for pivot_root
//!   ubuntu/
//!     22.04/
//!       rootfs/
//! ```

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
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
/// root filesystem at `/var/lib/virturust/images/<name>/<tag>/rootfs/`.
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

    // Decompress (gzip) and unpack (tar)
    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(true);
    archive.set_overwrite(true);

    archive.unpack(rootfs).context("failed to unpack layer")?;

    Ok(())
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
