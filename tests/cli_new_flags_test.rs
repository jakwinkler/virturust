//! Tests for newly added CLI flags (volume, network, port).
//!
//! These tests verify that the CLI argument definitions include the
//! expected flags by parsing known argument vectors via clap.

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_run_accepts_volume_flag() {
    let cli = Cli::parse_from(["corten", "run", "-v", "/host:/container", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.volumes, vec!["/host:/container"]);
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_long_volume_flag() {
    let cli = Cli::parse_from(["corten", "run", "--volume", "/src:/app:ro", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.volumes, vec!["/src:/app:ro"]);
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_network_flag() {
    let cli = Cli::parse_from(["corten", "run", "--network", "none", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.network, "none");
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_network_defaults_to_bridge() {
    let cli = Cli::parse_from(["corten", "run", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.network, "bridge");
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_publish_flag() {
    let cli = Cli::parse_from(["corten", "run", "-p", "8080:80", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.publish, vec!["8080:80"]);
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_long_publish_flag() {
    let cli = Cli::parse_from(["corten", "run", "--publish", "443:443", "alpine"]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.publish, vec!["443:443"]);
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_multiple_publish_flags() {
    let cli = Cli::parse_from([
        "corten", "run", "-p", "8080:80", "-p", "443:443", "alpine",
    ]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.publish, vec!["8080:80", "443:443"]);
        }
        _ => panic!("expected Run command"),
    }
}

#[test]
fn cli_run_accepts_multiple_volumes() {
    let cli = Cli::parse_from([
        "corten", "run", "-v", "/a:/b", "-v", "/c:/d:ro", "alpine",
    ]);
    match cli.command {
        corten::cli::Commands::Run(args) => {
            assert_eq!(args.volumes, vec!["/a:/b", "/c:/d:ro"]);
        }
        _ => panic!("expected Run command"),
    }
}
