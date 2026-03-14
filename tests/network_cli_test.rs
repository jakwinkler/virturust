//! Tests for network CLI subcommands and named network configuration.

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_network_create_parses() {
    let cli = Cli::parse_from(["corten", "network", "create", "backend"]);
    match cli.command {
        corten::cli::Commands::Network(args) => match args.command {
            corten::cli::NetworkCommands::Create(create) => {
                assert_eq!(create.name, "backend");
            }
            _ => panic!("expected Network Create"),
        },
        _ => panic!("expected Network command"),
    }
}

#[test]
fn cli_network_ls_parses() {
    let cli = Cli::parse_from(["corten", "network", "ls"]);
    match cli.command {
        corten::cli::Commands::Network(args) => match args.command {
            corten::cli::NetworkCommands::Ls => {}
            _ => panic!("expected Network Ls"),
        },
        _ => panic!("expected Network command"),
    }
}

#[test]
fn cli_network_rm_parses() {
    let cli = Cli::parse_from(["corten", "network", "rm", "backend"]);
    match cli.command {
        corten::cli::Commands::Network(args) => match args.command {
            corten::cli::NetworkCommands::Rm(rm) => {
                assert_eq!(rm.name, "backend");
            }
            _ => panic!("expected Network Rm"),
        },
        _ => panic!("expected Network command"),
    }
}

#[test]
fn cli_network_help_exits_successfully() {
    // Parse with --help should succeed (clap exits 0)
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_corten"))
        .args(["network", "--help"])
        .output()
        .expect("failed to execute");
    assert!(result.status.success());
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("create") || stdout.contains("Create"));
}

#[test]
fn cli_run_with_named_network() {
    let cli = Cli::parse_from(["corten", "run", "--network", "backend", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.network, "backend");
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn network_info_serialization_roundtrip() {
    use corten::network::NetworkInfo;
    use std::collections::HashMap;

    let mut containers = HashMap::new();
    containers.insert("api".to_string(), "10.0.43.2".to_string());
    containers.insert("db".to_string(), "10.0.43.3".to_string());

    let info = NetworkInfo {
        name: "backend".to_string(),
        bridge: "corten-backend".to_string(),
        subnet: "10.0.43.0/24".to_string(),
        gateway: "10.0.43.1".to_string(),
        containers,
    };

    let json = serde_json::to_string(&info).unwrap();
    let deserialized: NetworkInfo = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.name, "backend");
    assert_eq!(deserialized.bridge, "corten-backend");
    assert_eq!(deserialized.subnet, "10.0.43.0/24");
    assert_eq!(deserialized.gateway, "10.0.43.1");
    assert_eq!(deserialized.containers.len(), 2);
    assert_eq!(deserialized.containers["api"], "10.0.43.2");
}
