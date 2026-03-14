//! Integration tests for OverlayFS.

#[path = "../helpers/mod.rs"]
mod helpers;

#[test]
#[ignore = "requires root + cgroups v2"]
fn overlay_writable_layer_is_per_container() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: run two containers on same image, verify writes don't leak
}

#[test]
#[ignore = "requires root + cgroups v2"]
fn overlay_cleanup_on_container_exit() {
    if !helpers::require_root_and_cgroups() { return; }
    // TODO: verify overlay mounts are cleaned up after container exits
}
