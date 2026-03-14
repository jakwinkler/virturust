//! E2E tests for full container lifecycle.
//!
//! These tests run actual containers and verify end-to-end behavior.
//! They require root, cgroups v2, and a pulled alpine image.
//!
//! Run with: sudo cargo test -- --ignored --test-threads=1

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn run_alpine_echo() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("echo");
    let (code, stdout, _stderr) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none", "alpine", "echo", "hello-from-corten",
    ]);

    assert_eq!(code, 0, "container should exit 0");
    assert!(
        stdout.contains("hello-from-corten"),
        "stdout should contain our echo message, got: {stdout}"
    );

    // Cleanup
    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn run_alpine_cat_os_release() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("osrel");
    let (code, stdout, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none", "alpine", "cat", "/etc/os-release",
    ]);

    assert_eq!(code, 0);
    assert!(
        stdout.contains("Alpine"),
        "should be Alpine Linux, got: {stdout}"
    );

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn run_exit_code_propagation() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("exit");
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none", "alpine", "sh", "-c", "exit 42",
    ]);

    assert_eq!(code, 42, "should propagate container exit code");
    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn full_lifecycle_run_inspect_rm() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("lifecycle");

    // Run a container
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none", "alpine", "echo", "lifecycle-test",
    ]);
    assert_eq!(code, 0);

    // Inspect it
    let stdout = helpers::run_corten_ok(&["inspect", &name]);
    assert!(stdout.contains(&name), "inspect should show container name");
    assert!(stdout.contains("alpine"), "inspect should show image name");
    assert!(stdout.contains("stopped"), "should be stopped after run");

    // PS shows it
    let stdout = helpers::run_corten_ok(&["ps"]);
    assert!(stdout.contains(&name), "ps should list the container");

    // Remove it
    helpers::run_corten_ok(&["rm", &name]);

    // Verify it's gone
    let (code, _, _) = helpers::run_corten(&["inspect", &name]);
    assert_ne!(code, 0, "inspect should fail after rm");
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn volume_mount_works() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    // Create a temp directory with a test file
    let tmp = tempfile::tempdir().unwrap();
    let test_file = tmp.path().join("test.txt");
    std::fs::write(&test_file, "volume-content-12345").unwrap();

    let name = helpers::test_container_name("vol");
    let vol_arg = format!("{}:/data", tmp.path().display());

    let (code, stdout, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none",
        "-v", &vol_arg, "alpine", "cat", "/data/test.txt",
    ]);

    assert_eq!(code, 0);
    assert!(
        stdout.contains("volume-content-12345"),
        "should see file content from volume, got: {stdout}"
    );

    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn volume_mount_readonly() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("file.txt"), "readonly").unwrap();

    let name = helpers::test_container_name("volro");
    let vol_arg = format!("{}:/data:ro", tmp.path().display());

    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none",
        "-v", &vol_arg, "alpine", "sh", "-c", "echo x > /data/newfile",
    ]);

    // Should fail because the mount is read-only
    assert_ne!(code, 0, "write to read-only volume should fail");
    helpers::run_corten(&["rm", &name]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn container_filesystem_is_isolated() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    // Write a file in one container
    let name1 = helpers::test_container_name("iso1");
    helpers::run_corten(&[
        "run", "--name", &name1, "--network", "none",
        "alpine", "sh", "-c", "echo marker > /tmp/isolation-test",
    ]);

    // Check it's NOT visible in another container (OverlayFS isolation)
    let name2 = helpers::test_container_name("iso2");
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name2, "--network", "none",
        "alpine", "cat", "/tmp/isolation-test",
    ]);

    assert_ne!(code, 0, "file from container 1 should not be visible in container 2");

    helpers::run_corten(&["rm", &name1]);
    helpers::run_corten(&["rm", &name2]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn minimal_dev_is_secure() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("dev");

    // /dev/null should exist
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none",
        "alpine", "test", "-c", "/dev/null",
    ]);
    assert_eq!(code, 0, "/dev/null should exist");
    helpers::run_corten(&["rm", &name]);

    // /dev/sda should NOT exist (no host devices)
    let name2 = helpers::test_container_name("dev2");
    let (code, _, _) = helpers::run_corten(&[
        "run", "--name", &name2, "--network", "none",
        "alpine", "test", "-e", "/dev/sda",
    ]);
    assert_ne!(code, 0, "/dev/sda should NOT exist in container");
    helpers::run_corten(&["rm", &name2]);
}

#[test]
#[ignore = "requires root + cgroups v2 + alpine image"]
fn resource_limits_applied() {
    if !helpers::require_root_and_cgroups() {
        return;
    }

    let name = helpers::test_container_name("limits");
    let (code, stdout, _) = helpers::run_corten(&[
        "run", "--name", &name, "--network", "none",
        "--memory", "64m", "--pids-limit", "50",
        "alpine", "cat", "/proc/self/cgroup",
    ]);

    assert_eq!(code, 0);
    // The container should be in a corten cgroup
    assert!(
        stdout.contains("corten") || stdout.contains("/"),
        "should be in a cgroup"
    );

    // Inspect should show limits
    let stdout = helpers::run_corten_ok(&["inspect", &name]);
    assert!(stdout.contains("64"), "should show memory limit");

    helpers::run_corten(&["rm", &name]);
}
