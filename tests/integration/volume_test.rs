//! Integration tests for volume mounts.

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2"]
fn volume_mount_makes_host_dir_visible() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: create temp dir with file, mount into container, verify visible
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn volume_mount_readonly_prevents_writes() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: mount with :ro, verify write fails
}
