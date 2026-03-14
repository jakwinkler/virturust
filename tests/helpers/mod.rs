//! Shared test helpers for Corten integration and E2E tests.

#![allow(dead_code)]

use std::path::Path;

/// Check if we're running as root.
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Check if cgroups v2 is available on this system.
pub fn cgroups_v2_available() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}

/// Skip test if not running as root with cgroups v2.
/// Returns false if prerequisites are missing (caller should return early).
pub fn require_root_and_cgroups() -> bool {
    if !is_root() {
        eprintln!("skipping: requires root");
        return false;
    }
    if !cgroups_v2_available() {
        eprintln!("skipping: requires cgroups v2");
        return false;
    }
    true
}

/// Get a temporary data directory for test isolation.
/// Sets the CORTEN_DATA_DIR env var to isolate test data.
pub fn temp_data_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

/// Wait for a TCP port to become reachable, with timeout.
pub fn wait_for_port(port: u16, timeout: std::time::Duration) -> bool {
    use std::net::TcpStream;
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Run the corten binary with given args and return (exit_code, stdout, stderr).
pub fn run_corten(args: &[&str]) -> (i32, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_corten"))
        .args(args)
        .output()
        .expect("failed to execute corten binary");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// Run corten and assert it succeeds, returning stdout.
pub fn run_corten_ok(args: &[&str]) -> String {
    let (code, stdout, stderr) = run_corten(args);
    assert!(
        code == 0,
        "corten {:?} failed (exit {code}):\nstdout: {stdout}\nstderr: {stderr}",
        args
    );
    stdout
}

/// Generate a unique container name for tests.
pub fn test_container_name(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    format!("{prefix}-{ts}")
}
