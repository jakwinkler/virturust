//! Unit tests for volume mount parsing.

use corten::config::parse_volume;
use std::path::PathBuf;

#[test]
fn parse_volume_basic_rw() {
    let vol = parse_volume("/host:/container").unwrap();
    assert_eq!(vol.host_path, PathBuf::from("/host"));
    assert_eq!(vol.container_path, PathBuf::from("/container"));
    assert!(!vol.read_only);
}

#[test]
fn parse_volume_read_only() {
    let vol = parse_volume("/data:/mnt/data:ro").unwrap();
    assert_eq!(vol.host_path, PathBuf::from("/data"));
    assert_eq!(vol.container_path, PathBuf::from("/mnt/data"));
    assert!(vol.read_only);
}

#[test]
fn parse_volume_explicit_rw() {
    let vol = parse_volume("/src:/app:rw").unwrap();
    assert!(!vol.read_only);
}

#[test]
fn parse_volume_deep_paths() {
    let vol = parse_volume("/home/user/projects/myapp:/var/www/html:ro").unwrap();
    assert_eq!(vol.host_path, PathBuf::from("/home/user/projects/myapp"));
    assert_eq!(vol.container_path, PathBuf::from("/var/www/html"));
    assert!(vol.read_only);
}

#[test]
fn parse_volume_root_paths() {
    let vol = parse_volume("/:/mnt").unwrap();
    assert_eq!(vol.host_path, PathBuf::from("/"));
    assert_eq!(vol.container_path, PathBuf::from("/mnt"));
}

#[test]
fn parse_volume_named_volume_resolves() {
    // "myvolume:/container" is now a named volume, not a relative path error
    let vol = parse_volume("testvolume123:/container").unwrap();
    assert!(vol.host_path.to_string_lossy().contains("volumes/testvolume123"));
    std::fs::remove_dir_all(vol.host_path).ok();
}

#[test]
fn parse_volume_relative_container_path_fails() {
    assert!(parse_volume("/host:relative").is_err());
}

#[test]
fn parse_volume_invalid_option_fails() {
    assert!(parse_volume("/host:/container:invalid").is_err());
}

#[test]
fn parse_volume_missing_container_path_fails() {
    assert!(parse_volume("/host").is_err());
}

#[test]
fn parse_volume_empty_string_fails() {
    assert!(parse_volume("").is_err());
}

#[test]
fn parse_volume_just_colon_fails() {
    assert!(parse_volume(":").is_err());
}
