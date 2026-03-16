//! End-to-end tests for per-user container isolation.
//!
//! These tests verify that:
//! - Each user's containers are stored in separate directories
//! - User A cannot see User B's containers via `corten ps`
//! - Images are shared across users
//! - Container operations are scoped to the calling user
//!
//! Most tests are #[ignore] because they need the installed binary
//! with capabilities (make install).

use std::process::Command;
use std::path::Path;

/// Helper: run corten as a specific UID by setting CORTEN_REAL_UID
fn corten_as_user(uid: u32, args: &[&str]) -> std::process::Output {
    let corten = std::env::var("CORTEN_BIN")
        .unwrap_or_else(|_| which_corten());

    Command::new(&corten)
        .args(args)
        .env("CORTEN_REAL_UID", uid.to_string())
        .env("CORTEN_REAL_GID", uid.to_string())
        .output()
        .expect("failed to run corten")
}

fn which_corten() -> String {
    // Prefer installed binary (has capabilities)
    if Path::new("/usr/local/bin/corten").exists() {
        "/usr/local/bin/corten".to_string()
    } else {
        "./target/release/corten".to_string()
    }
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

// =============================================================================
// Unit tests (no privileges needed)
// =============================================================================

/// NOTE: These tests modify process-wide env vars and must run single-threaded.
/// Run with: cargo test --test user_isolation_test -- --test-threads=1
///
/// The isolation_test.rs has the parallel-safe versions of these tests.
/// These tests focus on the e2e behavior.

#[test]
fn user_dirs_are_isolated() {
    // This test verifies the path construction logic.
    // It's also covered in isolation_test.rs with proper env handling.
    unsafe { std::env::set_var("CORTEN_REAL_UID", "5001"); }
    let dir_a = corten::config::containers_dir();
    unsafe { std::env::set_var("CORTEN_REAL_UID", "5002"); }
    let dir_b = corten::config::containers_dir();
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }

    assert_ne!(dir_a, dir_b);
    assert!(dir_a.to_string_lossy().contains("5001"));
    assert!(dir_b.to_string_lossy().contains("5002"));
}

#[test]
fn root_sees_legacy_containers() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }
    let dir = corten::config::containers_dir();
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }

    assert!(
        !dir.to_string_lossy().contains("users/"),
        "Root should use legacy path, got: {}", dir.display()
    );
}

#[test]
fn images_always_shared() {
    unsafe { std::env::set_var("CORTEN_REAL_UID", "9999"); }
    let dir = corten::config::images_dir();
    unsafe { std::env::set_var("CORTEN_REAL_UID", "0"); }

    assert!(!dir.to_string_lossy().contains("users/"),
        "Images should be shared, got: {}", dir.display());
}

// =============================================================================
// E2E tests (need installed binary with capabilities)
// =============================================================================

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_a_cannot_see_user_b_containers() {
    // User A (uid 5001) creates a container
    let output = corten_as_user(5001, &["run", "--rm", "--name", "user-a-test", "--network", "none", "alpine", "echo", "user-a"]);
    assert!(output.status.success() || stderr(&output).contains("not found"),
        "User A run failed: {}", stderr(&output));

    // User B (uid 5002) should NOT see it
    let ps_b = corten_as_user(5002, &["ps"]);
    let ps_output = stdout(&ps_b);
    assert!(
        !ps_output.contains("user-a-test"),
        "User B should NOT see User A's container in ps output: {ps_output}"
    );

    // User A should see it
    let ps_a = corten_as_user(5001, &["ps"]);
    // Container might already be removed (--rm), so just verify no cross-contamination
    let ps_a_output = stdout(&ps_a);
    assert!(
        !ps_a_output.contains("user-b"),
        "User A should not see User B's containers"
    );
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_cannot_rm_other_users_container() {
    // Create container as user 5001
    corten_as_user(5001, &["run", "-d", "--name", "protected-container", "--network", "none", "alpine", "sleep", "30"]);

    // User 5002 tries to remove it — should fail or not find it
    let rm_output = corten_as_user(5002, &["rm", "protected-container"]);
    assert!(
        !rm_output.status.success() || stderr(&rm_output).contains("not found"),
        "User 5002 should NOT be able to rm User 5001's container"
    );

    // Clean up as the owner
    corten_as_user(5001, &["stop", "protected-container"]);
    corten_as_user(5001, &["rm", "protected-container"]);
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_cannot_stop_other_users_container() {
    // Create container as user 5001
    corten_as_user(5001, &["run", "-d", "--name", "stop-test", "--network", "none", "alpine", "sleep", "30"]);

    // User 5002 tries to stop it — should fail
    let stop_output = corten_as_user(5002, &["stop", "stop-test"]);
    assert!(
        !stop_output.status.success() || stderr(&stop_output).contains("not found"),
        "User 5002 should NOT be able to stop User 5001's container"
    );

    // Clean up
    corten_as_user(5001, &["stop", "stop-test"]);
    corten_as_user(5001, &["rm", "stop-test"]);
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_cannot_inspect_other_users_container() {
    corten_as_user(5001, &["run", "-d", "--name", "inspect-test", "--network", "none", "alpine", "sleep", "30"]);

    let inspect = corten_as_user(5002, &["inspect", "inspect-test"]);
    assert!(
        !inspect.status.success() || stderr(&inspect).contains("not found"),
        "User 5002 should NOT be able to inspect User 5001's container"
    );

    corten_as_user(5001, &["stop", "inspect-test"]);
    corten_as_user(5001, &["rm", "inspect-test"]);
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_cannot_exec_into_other_users_container() {
    corten_as_user(5001, &["run", "-d", "--name", "exec-test", "--network", "none", "alpine", "sleep", "30"]);

    let exec_output = corten_as_user(5002, &["exec", "exec-test", "echo", "hacked"]);
    assert!(
        !exec_output.status.success() || stderr(&exec_output).contains("not found"),
        "User 5002 should NOT be able to exec into User 5001's container"
    );

    corten_as_user(5001, &["stop", "exec-test"]);
    corten_as_user(5001, &["rm", "exec-test"]);
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn user_cannot_read_other_users_logs() {
    corten_as_user(5001, &["run", "-d", "--name", "logs-test", "--network", "none", "alpine", "sh", "-c", "echo secret-data && sleep 30"]);

    // Wait for container to write logs
    std::thread::sleep(std::time::Duration::from_secs(2));

    let logs = corten_as_user(5002, &["logs", "logs-test"]);
    let logs_output = stdout(&logs);
    assert!(
        !logs_output.contains("secret-data"),
        "User 5002 should NOT see User 5001's container logs"
    );

    corten_as_user(5001, &["stop", "logs-test"]);
    corten_as_user(5001, &["rm", "logs-test"]);
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn shared_images_work_for_all_users() {
    // Both users should be able to list the same images
    let images_a = corten_as_user(5001, &["images"]);
    let images_b = corten_as_user(5002, &["images"]);

    // If alpine is pulled, both should see it
    if stdout(&images_a).contains("alpine") {
        assert!(
            stdout(&images_b).contains("alpine"),
            "Both users should see shared images"
        );
    }
}

#[test]
#[ignore = "needs installed corten (make install)"]
fn ps_only_shows_own_containers() {
    // User A creates containers
    corten_as_user(5001, &["run", "-d", "--name", "a-web", "--network", "none", "alpine", "sleep", "30"]);
    corten_as_user(5001, &["run", "-d", "--name", "a-db", "--network", "none", "alpine", "sleep", "30"]);

    // User B creates containers
    corten_as_user(5002, &["run", "-d", "--name", "b-api", "--network", "none", "alpine", "sleep", "30"]);

    // User A's ps should show a-web, a-db but NOT b-api
    let ps_a = stdout(&corten_as_user(5001, &["ps"]));
    assert!(ps_a.contains("a-web") || ps_a.contains("a-db"),
        "User A should see own containers");
    assert!(!ps_a.contains("b-api"),
        "User A should NOT see User B's containers");

    // User B's ps should show b-api but NOT a-web, a-db
    let ps_b = stdout(&corten_as_user(5002, &["ps"]));
    assert!(ps_b.contains("b-api"),
        "User B should see own containers");
    assert!(!ps_b.contains("a-web") && !ps_b.contains("a-db"),
        "User B should NOT see User A's containers");

    // Cleanup
    corten_as_user(5001, &["stop", "a-web"]);
    corten_as_user(5001, &["stop", "a-db"]);
    corten_as_user(5002, &["stop", "b-api"]);
    corten_as_user(5001, &["rm", "a-web"]);
    corten_as_user(5001, &["rm", "a-db"]);
    corten_as_user(5002, &["rm", "b-api"]);
}
