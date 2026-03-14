//! Tests for rootless container mode.
//!
//! Verifies CLI flag parsing, config deserialization defaults,
//! and that the rootless flag is correctly wired through.

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_run_rootless_flag() {
    let cli = Cli::parse_from(["corten", "run", "--rootless", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert!(args.rootless);
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn cli_run_rootless_defaults_false() {
    let cli = Cli::parse_from(["corten", "run", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert!(!args.rootless);
        }
        _ => panic!("expected Run"),
    }
}

#[test]
fn config_rootless_defaults_false() {
    let json = r#"{
        "id": "test", "name": "test", "image": "alpine",
        "command": ["/bin/sh"], "hostname": "test",
        "resources": {}, "rootfs": "/tmp/rootfs"
    }"#;
    let config: corten::config::ContainerConfig = serde_json::from_str(json).unwrap();
    assert!(!config.rootless);
}

#[test]
fn config_rootless_set_true() {
    let json = r#"{
        "id": "test", "name": "test", "image": "alpine",
        "command": ["/bin/sh"], "hostname": "test",
        "resources": {}, "rootfs": "/tmp/rootfs",
        "rootless": true
    }"#;
    let config: corten::config::ContainerConfig = serde_json::from_str(json).unwrap();
    assert!(config.rootless);
}

#[test]
fn cli_run_help_shows_rootless() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_corten"))
        .args(["run", "--help"])
        .output()
        .expect("failed to execute corten binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--rootless"), "run --help should mention --rootless flag");
}
