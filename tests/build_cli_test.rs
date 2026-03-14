//! CLI tests for the build subcommand.

use clap::Parser;
use corten::cli::Cli;

#[test]
fn cli_build_default_path() {
    let cli = Cli::parse_from(["corten", "build"]);
    match cli.command {
        corten::cli::Commands::Build(args) => {
            assert_eq!(args.path, ".");
        }
        _ => panic!("expected Build command"),
    }
}

#[test]
fn cli_build_custom_path() {
    let cli = Cli::parse_from(["corten", "build", "examples/nginx-php.toml"]);
    match cli.command {
        corten::cli::Commands::Build(args) => {
            assert_eq!(args.path, "examples/nginx-php.toml");
        }
        _ => panic!("expected Build command"),
    }
}

#[test]
fn cli_build_help() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_corten"))
        .args(["build", "--help"])
        .output()
        .expect("failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Corten.toml"));
}
