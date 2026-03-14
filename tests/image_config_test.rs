//! Unit tests for OCI image configuration handling.

use corten::image::{ImageConfig, load_image_config};

#[test]
fn image_config_default_has_empty_fields() {
    let config = ImageConfig::default();
    assert!(config.env.is_empty());
    assert!(config.cmd.is_empty());
    assert!(config.entrypoint.is_empty());
    assert!(config.working_dir.is_empty());
    assert!(config.user.is_empty());
}

#[test]
fn image_config_serialization_roundtrip() {
    let config = ImageConfig {
        env: vec!["PATH=/usr/bin".to_string(), "HOME=/root".to_string()],
        cmd: vec!["/bin/sh".to_string()],
        entrypoint: vec!["/docker-entrypoint.sh".to_string()],
        working_dir: "/app".to_string(),
        user: "nobody".to_string(),
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: ImageConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.env, config.env);
    assert_eq!(deserialized.cmd, config.cmd);
    assert_eq!(deserialized.entrypoint, config.entrypoint);
    assert_eq!(deserialized.working_dir, config.working_dir);
    assert_eq!(deserialized.user, config.user);
}

#[test]
fn image_config_deserialization_with_missing_fields() {
    // When fields are missing from JSON, they should default to empty
    let json = r#"{"env": ["FOO=bar"]}"#;
    let config: ImageConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.env, vec!["FOO=bar"]);
    assert!(config.cmd.is_empty());
    assert!(config.entrypoint.is_empty());
    assert!(config.working_dir.is_empty());
    assert!(config.user.is_empty());
}

#[test]
fn load_image_config_returns_default_for_missing_image() {
    let config = load_image_config("nonexistent_image_xyz", "latest");
    assert!(config.env.is_empty());
    assert!(config.cmd.is_empty());
}
