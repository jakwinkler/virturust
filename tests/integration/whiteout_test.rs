//! Tests for OCI whiteout handling during layer extraction.
//!
//! These tests create synthetic tar layers with whiteout markers
//! and verify that the extraction logic handles them correctly.

use std::fs;

/// Create a tar archive in memory with the given entries.
/// Each entry is (path, content). Directories end with '/'.
fn create_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let buf = Vec::new();
    let encoder = flate2::write::GzEncoder::new(buf, flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);

    for (path, content) in entries {
        if path.ends_with('/') {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, &[][..]).unwrap();
        } else {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, *content).unwrap();
        }
    }

    let encoder = tar.into_inner().unwrap();
    encoder.finish().unwrap()
}

/// Simulate layer extraction with whiteout handling (same logic as image.rs).
fn extract_layer(tar_gz: &[u8], rootfs: &std::path::Path) {
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);

    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().into_owned();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name == ".wh..wh..opq" {
            let parent = rootfs.join(
                path.parent().unwrap_or_else(|| std::path::Path::new("")),
            );
            if parent.exists() {
                for child in fs::read_dir(&parent).unwrap() {
                    let child = child.unwrap();
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

        entry.unpack_in(rootfs).unwrap();
    }
}

#[test]
fn whiteout_file_deletes_target() {
    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path();

    // Layer 1: create a file
    let layer1 = create_tar_gz(&[("hello.txt", b"hello world")]);
    extract_layer(&layer1, rootfs);
    assert!(rootfs.join("hello.txt").exists());

    // Layer 2: whiteout deletes it
    let layer2 = create_tar_gz(&[(".wh.hello.txt", b"")]);
    extract_layer(&layer2, rootfs);
    assert!(!rootfs.join("hello.txt").exists(), "file should be deleted by whiteout");
}

#[test]
fn whiteout_file_deletes_directory() {
    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path();

    // Layer 1: create a directory with files
    let layer1 = create_tar_gz(&[
        ("mydir/", &[]),
        ("mydir/file1.txt", b"content1"),
        ("mydir/file2.txt", b"content2"),
    ]);
    extract_layer(&layer1, rootfs);
    assert!(rootfs.join("mydir").exists());
    assert!(rootfs.join("mydir/file1.txt").exists());

    // Layer 2: whiteout the entire directory
    let layer2 = create_tar_gz(&[(".wh.mydir", b"")]);
    extract_layer(&layer2, rootfs);
    assert!(!rootfs.join("mydir").exists(), "directory should be deleted by whiteout");
}

#[test]
fn opaque_whiteout_clears_directory() {
    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path();

    // Layer 1: create a directory with files
    let layer1 = create_tar_gz(&[
        ("config/", &[]),
        ("config/old.conf", b"old config"),
        ("config/legacy.conf", b"legacy config"),
    ]);
    extract_layer(&layer1, rootfs);
    assert!(rootfs.join("config/old.conf").exists());
    assert!(rootfs.join("config/legacy.conf").exists());

    // Layer 2: opaque whiteout clears the dir, then add new content
    let layer2 = create_tar_gz(&[
        ("config/.wh..wh..opq", b""),
        ("config/new.conf", b"new config"),
    ]);
    extract_layer(&layer2, rootfs);

    // Old files should be gone
    assert!(!rootfs.join("config/old.conf").exists(), "old file should be cleared");
    assert!(!rootfs.join("config/legacy.conf").exists(), "legacy file should be cleared");
    // New file should exist
    assert!(rootfs.join("config/new.conf").exists(), "new file should be added");
}

#[test]
fn whiteout_for_nonexistent_file_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path();

    // Whiteout for a file that doesn't exist — should not error
    let layer = create_tar_gz(&[(".wh.nonexistent", b"")]);
    extract_layer(&layer, rootfs);
    // If we got here without panicking, the test passes
}

#[test]
fn normal_files_extract_normally() {
    let dir = tempfile::tempdir().unwrap();
    let rootfs = dir.path();

    let layer = create_tar_gz(&[
        ("bin/", &[]),
        ("bin/sh", b"#!/bin/sh"),
        ("etc/", &[]),
        ("etc/hostname", b"container"),
    ]);
    extract_layer(&layer, rootfs);

    assert!(rootfs.join("bin/sh").exists());
    assert_eq!(fs::read_to_string(rootfs.join("etc/hostname")).unwrap(), "container");
}
