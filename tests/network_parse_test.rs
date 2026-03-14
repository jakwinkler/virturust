//! Unit tests for network-related parsing and configuration.

use corten::config::ContainerConfig;

#[test]
fn container_config_default_network_mode_is_bridge() {
    // When deserializing a config without network_mode, it should default to "bridge"
    let json = r#"{
        "id": "test-id",
        "name": "test",
        "image": "alpine:latest",
        "command": ["/bin/sh"],
        "hostname": "test",
        "resources": {},
        "rootfs": "/tmp/rootfs",
        "volumes": [],
        "env": [],
        "working_dir": "",
        "user": ""
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.network_mode, "bridge");
}

#[test]
fn container_config_preserves_network_mode() {
    let json = r#"{
        "id": "test-id",
        "name": "test",
        "image": "alpine:latest",
        "command": ["/bin/sh"],
        "hostname": "test",
        "resources": {},
        "rootfs": "/tmp/rootfs",
        "volumes": [],
        "env": [],
        "working_dir": "",
        "user": "",
        "network_mode": "none"
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.network_mode, "none");
}

#[test]
fn container_config_host_network_mode() {
    let json = r#"{
        "id": "test-id",
        "name": "test",
        "image": "alpine:latest",
        "command": ["/bin/sh"],
        "hostname": "test",
        "resources": {},
        "rootfs": "/tmp/rootfs",
        "volumes": [],
        "env": [],
        "working_dir": "",
        "user": "",
        "network_mode": "host"
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.network_mode, "host");
}
