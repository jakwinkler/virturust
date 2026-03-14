//! Tests for restart policy parsing and CLI integration.

use clap::Parser;
use corten::cli::Cli;
use corten::config::ContainerConfig;

// =============================================================================
// ContainerConfig restart_policy deserialization tests
// =============================================================================

#[test]
fn config_default_restart_policy_is_no() {
    let json = r#"{
        "id": "test", "name": "test", "image": "alpine",
        "command": ["/bin/sh"], "hostname": "test",
        "resources": {}, "rootfs": "/tmp/rootfs"
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.restart_policy, "no");
}

#[test]
fn config_preserves_restart_always() {
    let json = r#"{
        "id": "test", "name": "test", "image": "alpine",
        "command": ["/bin/sh"], "hostname": "test",
        "resources": {}, "rootfs": "/tmp/rootfs",
        "restart_policy": "always"
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.restart_policy, "always");
}

#[test]
fn config_preserves_on_failure_with_max() {
    let json = r#"{
        "id": "test", "name": "test", "image": "alpine",
        "command": ["/bin/sh"], "hostname": "test",
        "resources": {}, "rootfs": "/tmp/rootfs",
        "restart_policy": "on-failure:5"
    }"#;
    let config: ContainerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.restart_policy, "on-failure:5");
}

// =============================================================================
// CLI restart flag parsing tests
// =============================================================================

#[test]
fn cli_run_restart_defaults_to_no() {
    let cli = Cli::parse_from(["corten", "run", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => assert_eq!(args.restart, "no"),
        _ => panic!("expected Run"),
    }
}

#[test]
fn cli_run_restart_always() {
    let cli = Cli::parse_from(["corten", "run", "--restart", "always", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => assert_eq!(args.restart, "always"),
        _ => panic!("expected Run"),
    }
}

#[test]
fn cli_run_restart_on_failure() {
    let cli = Cli::parse_from(["corten", "run", "--restart", "on-failure:3", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => assert_eq!(args.restart, "on-failure:3"),
        _ => panic!("expected Run"),
    }
}

// =============================================================================
// CLI image and system subcommand parsing tests
// =============================================================================

#[test]
fn cli_image_prune() {
    let cli = Cli::parse_from(["corten", "image", "prune"]);
    match cli.command {
        corten::cli::Commands::Image(_) => {}
        _ => panic!("expected Image"),
    }
}

#[test]
fn cli_system_prune() {
    let cli = Cli::parse_from(["corten", "system", "prune"]);
    match cli.command {
        corten::cli::Commands::System(_) => {}
        _ => panic!("expected System"),
    }
}
