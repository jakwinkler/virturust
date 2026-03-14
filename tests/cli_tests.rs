//! Tests for CLI argument parsing.
//!
//! These tests verify that the clap-derived CLI correctly parses
//! all supported argument combinations.

// Test the CLI via the compiled binary to verify end-to-end argument parsing.

/// Helper to run the virturust binary with args and capture output.
fn run_virturust(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_virturust"))
        .args(args)
        .output()
        .expect("failed to execute virturust binary")
}

#[test]
fn cli_help_exits_successfully() {
    let output = run_virturust(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("virturust"));
    assert!(stdout.contains("container runtime"));
}

#[test]
fn cli_version_exits_successfully() {
    let output = run_virturust(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("virturust"));
    assert!(stdout.contains("0.1.0"));
}

#[test]
fn cli_run_help_shows_options() {
    let output = run_virturust(&["run", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--memory"));
    assert!(stdout.contains("--cpus"));
    assert!(stdout.contains("--pids-limit"));
    assert!(stdout.contains("--hostname"));
    assert!(stdout.contains("--name"));
}

#[test]
fn cli_pull_help_shows_usage() {
    let output = run_virturust(&["pull", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Pull an image"));
}

#[test]
fn cli_no_subcommand_shows_help() {
    let output = run_virturust(&[]);
    // Should fail with usage info
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage") || stderr.contains("usage"));
}

#[test]
fn cli_unknown_subcommand_fails() {
    let output = run_virturust(&["nonexistent"]);
    assert!(!output.status.success());
}

#[test]
fn cli_images_help() {
    let output = run_virturust(&["images", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("List locally available images"));
}

#[test]
fn cli_ps_help() {
    let output = run_virturust(&["ps", "--help"]);
    assert!(output.status.success());
}

#[test]
fn cli_rm_help() {
    let output = run_virturust(&["rm", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remove"));
}

#[test]
fn cli_rm_requires_name() {
    let output = run_virturust(&["rm"]);
    assert!(!output.status.success());
}

#[test]
fn cli_pull_requires_image() {
    let output = run_virturust(&["pull"]);
    assert!(!output.status.success());
}

#[test]
fn cli_run_requires_image() {
    let output = run_virturust(&["run"]);
    assert!(!output.status.success());
}

#[test]
fn cli_verbose_flag_accepted() {
    let output = run_virturust(&["--verbose", "--help"]);
    assert!(output.status.success());
}

#[test]
fn cli_verbose_short_flag_accepted() {
    let output = run_virturust(&["-v", "--help"]);
    assert!(output.status.success());
}
