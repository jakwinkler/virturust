//! Tests for CLI argument parsing.
//!
//! These tests verify that the clap-derived CLI correctly parses
//! all supported argument combinations.

// Test the CLI via the compiled binary to verify end-to-end argument parsing.

/// Helper to run the corten binary with args and capture output.
fn run_corten(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_corten"))
        .args(args)
        .output()
        .expect("failed to execute corten binary")
}

#[test]
fn cli_help_exits_successfully() {
    let output = run_corten(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("corten"));
    assert!(stdout.contains("container runtime"));
}

#[test]
fn cli_version_exits_successfully() {
    let output = run_corten(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("corten"));
    assert!(stdout.contains("0.1.0"));
}

#[test]
fn cli_run_help_shows_options() {
    let output = run_corten(&["run", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--memory"));
    assert!(stdout.contains("--cpus"));
    assert!(stdout.contains("--pids-limit"));
    assert!(stdout.contains("--hostname"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--volume"));
}

#[test]
fn cli_pull_help_shows_usage() {
    let output = run_corten(&["pull", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Pull an image"));
}

#[test]
fn cli_no_subcommand_shows_help() {
    let output = run_corten(&[]);
    // Should fail with usage info
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage") || stderr.contains("usage"));
}

#[test]
fn cli_unknown_subcommand_fails() {
    let output = run_corten(&["nonexistent"]);
    assert!(!output.status.success());
}

#[test]
fn cli_images_help() {
    let output = run_corten(&["images", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("List locally available images"));
}

#[test]
fn cli_ps_help() {
    let output = run_corten(&["ps", "--help"]);
    assert!(output.status.success());
}

#[test]
fn cli_rm_help() {
    let output = run_corten(&["rm", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remove"));
}

#[test]
fn cli_rm_requires_name() {
    let output = run_corten(&["rm"]);
    assert!(!output.status.success());
}

#[test]
fn cli_pull_requires_image() {
    let output = run_corten(&["pull"]);
    assert!(!output.status.success());
}

#[test]
fn cli_run_requires_image() {
    let output = run_corten(&["run"]);
    assert!(!output.status.success());
}

#[test]
fn cli_verbose_flag_accepted() {
    let output = run_corten(&["--verbose", "--help"]);
    assert!(output.status.success());
}

#[test]
fn cli_volume_short_flag_accepted() {
    // -v is now the volume flag, not verbose
    let output = run_corten(&["run", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("-v"));
}

#[test]
fn cli_stop_help() {
    let output = run_corten(&["stop", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Stop"));
    assert!(stdout.contains("--time"));
}

#[test]
fn cli_stop_requires_name() {
    let output = run_corten(&["stop"]);
    assert!(!output.status.success());
}

#[test]
fn cli_inspect_help() {
    let output = run_corten(&["inspect", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("detailed"));
}

#[test]
fn cli_inspect_requires_name() {
    let output = run_corten(&["inspect"]);
    assert!(!output.status.success());
}

#[test]
fn cli_stop_default_timeout() {
    let output = run_corten(&["stop", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("10"));
}
