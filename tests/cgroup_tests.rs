//! Tests for cgroup v2 management.
//!
//! Most cgroup tests require root privileges and a cgroups v2 hierarchy.
//! Unit tests that don't require root test the logic and error handling.
//! Integration tests (marked #[ignore]) test actual cgroup operations.

use std::fs;
use std::path::Path;

/// Check if cgroups v2 is available on this system.
fn cgroups_v2_available() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}

/// Check if we're running as root.
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[test]
#[ignore = "requires root and cgroups v2"]
fn cgroup_create_and_destroy() {
    if !is_root() || !cgroups_v2_available() {
        return;
    }

    let cgroup = virturust::cgroup::Cgroup::create("test-container-001").unwrap();

    // Verify the directory was created
    assert!(Path::new("/sys/fs/cgroup/virturust/test-container-001").exists());

    // Set limits
    cgroup.set_memory_limit(256 * 1024 * 1024).unwrap(); // 256 MiB
    cgroup.set_cpu_limit(0.5).unwrap(); // half a CPU
    cgroup.set_pids_limit(100).unwrap();

    // Verify limits were written
    let mem_max =
        fs::read_to_string("/sys/fs/cgroup/virturust/test-container-001/memory.max").unwrap();
    assert_eq!(mem_max.trim(), "268435456");

    let cpu_max =
        fs::read_to_string("/sys/fs/cgroup/virturust/test-container-001/cpu.max").unwrap();
    assert_eq!(cpu_max.trim(), "50000 100000");

    let pids_max =
        fs::read_to_string("/sys/fs/cgroup/virturust/test-container-001/pids.max").unwrap();
    assert_eq!(pids_max.trim(), "100");

    // Cleanup
    cgroup.destroy().unwrap();
    assert!(!Path::new("/sys/fs/cgroup/virturust/test-container-001").exists());
}

#[test]
#[ignore = "requires root and cgroups v2"]
fn cgroup_memory_limit_various_values() {
    if !is_root() || !cgroups_v2_available() {
        return;
    }

    let test_values = [
        (64 * 1024 * 1024u64, "67108864"),       // 64 MiB
        (128 * 1024 * 1024, "134217728"),         // 128 MiB
        (1024 * 1024 * 1024, "1073741824"),       // 1 GiB
        (2 * 1024 * 1024 * 1024, "2147483648"),   // 2 GiB
    ];

    for (i, (bytes, expected)) in test_values.iter().enumerate() {
        let id = format!("test-mem-{i}");
        let cgroup = virturust::cgroup::Cgroup::create(&id).unwrap();
        cgroup.set_memory_limit(*bytes).unwrap();

        let actual = fs::read_to_string(format!("/sys/fs/cgroup/virturust/{id}/memory.max"))
            .unwrap();
        assert_eq!(actual.trim(), *expected, "memory limit mismatch for {bytes} bytes");

        cgroup.destroy().unwrap();
    }
}

#[test]
#[ignore = "requires root and cgroups v2"]
fn cgroup_cpu_limit_various_values() {
    if !is_root() || !cgroups_v2_available() {
        return;
    }

    let test_values = [
        (0.25f64, "25000 100000"),   // quarter CPU
        (0.5, "50000 100000"),       // half CPU
        (1.0, "100000 100000"),      // one CPU
        (2.0, "200000 100000"),      // two CPUs
        (4.0, "400000 100000"),      // four CPUs
    ];

    for (i, (cpus, expected)) in test_values.iter().enumerate() {
        let id = format!("test-cpu-{i}");
        let cgroup = virturust::cgroup::Cgroup::create(&id).unwrap();
        cgroup.set_cpu_limit(*cpus).unwrap();

        let actual = fs::read_to_string(format!("/sys/fs/cgroup/virturust/{id}/cpu.max"))
            .unwrap();
        assert_eq!(actual.trim(), *expected, "CPU limit mismatch for {cpus} CPUs");

        cgroup.destroy().unwrap();
    }
}

#[test]
#[ignore = "requires root and cgroups v2"]
fn cgroup_pids_limit_values() {
    if !is_root() || !cgroups_v2_available() {
        return;
    }

    for max in [1u64, 10, 50, 100, 1000, 32768] {
        let id = format!("test-pids-{max}");
        let cgroup = virturust::cgroup::Cgroup::create(&id).unwrap();
        cgroup.set_pids_limit(max).unwrap();

        let actual = fs::read_to_string(format!("/sys/fs/cgroup/virturust/{id}/pids.max"))
            .unwrap();
        assert_eq!(actual.trim(), max.to_string());

        cgroup.destroy().unwrap();
    }
}
