//! Integration tests for filesystem isolation.

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2"]
fn minimal_dev_has_required_devices() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: verify /dev/null, /dev/zero, /dev/random, /dev/urandom exist
    // and that /dev/sda does NOT exist
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn container_rootfs_is_isolated() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: write a file in one container, verify it's absent in the next
}
