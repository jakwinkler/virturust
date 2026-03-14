//! Tests for image management functionality.
//!
//! Note: Tests that actually pull images from Docker Hub are marked
//! with #[ignore] to avoid network dependencies in CI. Run them with:
//! `cargo test -- --ignored`

use virturust::image;

#[test]
fn image_exists_returns_false_for_missing() {
    assert!(!image::image_exists("nonexistent_image_xyz", "latest"));
}

#[test]
fn list_images_returns_empty_when_no_images() {
    // When no images directory exists, should return empty vec
    let result = image::list_images();
    assert!(result.is_ok());
    // May or may not be empty depending on system state, but shouldn't error
}

#[test]
fn image_rootfs_returns_correct_path() {
    let path = image::image_rootfs("alpine", "latest");
    assert!(path.ends_with("alpine/latest/rootfs"));
}

#[test]
fn image_rootfs_with_versioned_tag() {
    let path = image::image_rootfs("ubuntu", "22.04");
    assert!(path.ends_with("ubuntu/22.04/rootfs"));
}

#[test]
fn image_rootfs_with_named_tag() {
    let path = image::image_rootfs("debian", "bookworm");
    assert!(path.ends_with("debian/bookworm/rootfs"));
}

// =============================================================================
// Integration tests (require network, run with --ignored)
// =============================================================================

#[tokio::test]
#[ignore = "requires network access and root for storage directory"]
async fn pull_alpine_image() {
    let result = image::pull_image("alpine", "latest").await;
    assert!(result.is_ok(), "Failed to pull alpine: {:?}", result.err());
    assert!(image::image_exists("alpine", "latest"));
}

#[tokio::test]
#[ignore = "requires network access and root for storage directory"]
async fn pull_ubuntu_image() {
    let result = image::pull_image("ubuntu", "22.04").await;
    assert!(
        result.is_ok(),
        "Failed to pull ubuntu:22.04: {:?}",
        result.err()
    );
}

#[tokio::test]
#[ignore = "requires network access and root for storage directory"]
async fn pull_debian_image() {
    let result = image::pull_image("debian", "bookworm").await;
    assert!(
        result.is_ok(),
        "Failed to pull debian:bookworm: {:?}",
        result.err()
    );
}
